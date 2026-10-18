//! 文件消息：分类（忽略 / 仅下载 / 下载后打开）、下载到桌面、按需打开。
//!
//! 分类只看后缀名，桶来自配置项 `files`（open = 下载后按需打开，ignore = 完全不管，
//! 其余兜底只下载）。下载走 OneBot 的 get_file——kovi 没封装这个动作，
//! 用 RuntimeBot::send_api_return 直接发（kovi-0.13.5/src/bot/runtimebot.rs:167）。
//!
//! 打开要过两道门：这位好友的 role 命中「当前或下一节课」的科目名，且当前没有全屏窗口。

use crate::config::{Config, FilePolicy, WindowState};
use kovi::RuntimeBot;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// 后缀名分类结果。
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileAction {
    /// 完全不管
    Ignore,
    /// 仅下载到桌面
    Download,
    /// 下载到桌面，按需自动打开
    Open,
}

/// 取后缀名（小写、不带点）。没有后缀返回空串。
pub fn ext_of(name: &str) -> String {
    match name.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() => ext.to_ascii_lowercase(),
        _ => String::new(),
    }
}

fn listed(list: &[String], ext: &str) -> bool {
    list.iter()
        .any(|item| item.trim().trim_start_matches('.').eq_ignore_ascii_case(ext))
}

/// 按配置把后缀分到三类。
pub fn plan(ext: &str, policy: &FilePolicy) -> FileAction {
    let ext = ext.trim().trim_start_matches('.').to_ascii_lowercase();
    if ext.is_empty() {
        // 没后缀的东西判不出类型，按兜底处理（只下载）。
        return FileAction::Download;
    }
    if listed(&policy.ignore, &ext) {
        return FileAction::Ignore;
    }
    if listed(&policy.open, &ext) {
        return FileAction::Open;
    }
    FileAction::Download
}

/// 一条文件段里我们关心的字段（OneBot v11 私聊文件段的 data）。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncomingFile {
    pub name: String,
    pub file_id: Option<String>,
}

/// 挑出消息里的文件段。
pub fn incoming(message: &kovi::message::Message) -> Vec<IncomingFile> {
    message
        .iter()
        .filter(|segment| segment.kind == "file")
        .filter_map(|segment| {
            let name = segment
                .data
                .get("file")
                .and_then(|value| value.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let file_id = segment
                .data
                .get("file_id")
                .and_then(|value| value.as_str())
                .map(|id| id.to_string());
            if name.is_empty() && file_id.is_none() {
                return None;
            }
            let name = if name.is_empty() { "文件".to_string() } else { name };
            Some(IncomingFile { name, file_id })
        })
        .collect()
}

/// 桌面目录。中文系统的 Desktop 一定是这个名字。
pub fn desktop_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let desktop = home.join("Desktop");
    if desktop.is_dir() {
        return Some(desktop);
    }
    let onedrive = home.join("OneDrive").join("Desktop");
    if onedrive.is_dir() {
        return Some(onedrive);
    }
    Some(desktop)
}

/// 文件名里不允许出现路径分隔符：附件名是对方给的，不能让它写到桌面以外的地方。
fn sanitize(name: &str) -> String {
    let base = name.rsplit(['\\', '/']).next().unwrap_or(name).trim();
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            ':' | '*' | '?' | '"' | '<' | '>' | '|' => '_',
            other => other,
        })
        .collect();
    if cleaned.is_empty() { "文件".to_string() } else { cleaned }
}

/// 复制到目标目录；重名时加 (2)、(3)……，不覆盖用户桌面上已有的文件。
pub fn deliver(source: &Path, name: &str, dir: &Path) -> std::io::Result<PathBuf> {
    let safe = sanitize(name);
    let mut target = dir.join(&safe);
    let mut counter = 2;
    while target.exists() && counter <= 500 {
        let stem = Path::new(&safe)
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("文件");
        let ext = Path::new(&safe)
            .extension()
            .and_then(|s| s.to_str())
            .unwrap_or("");
        let candidate = if ext.is_empty() {
            format!("{stem} ({counter})")
        } else {
            format!("{stem} ({counter}).{ext}")
        };
        target = dir.join(candidate);
        counter += 1;
    }
    std::fs::copy(source, &target)?;
    Ok(target)
}

/// 收到含文件段的消息时调用：该忽略的忽略，该下载的下载，该打开的打开。
pub async fn handle(
    bot: Arc<RuntimeBot>,
    user_id: i64,
    message: &kovi::message::Message,
    config: &Config,
) {
    for file in incoming(message) {
        let action = plan(&ext_of(&file.name), &config.files);
        if action == FileAction::Ignore {
            eprintln!("文件消息按策略忽略：{}", file.name);
            continue;
        }
        let Some(source) = local_source(&bot, &file).await else {
            eprintln!("取文件失败（get_file 没给本地路径）：{}", file.name);
            continue;
        };
        let Some(dir) = desktop_dir() else {
            eprintln!("找不到桌面目录，放弃下载：{}", file.name);
            continue;
        };
        match deliver(&source, &file.name, &dir) {
            Ok(path) => {
                eprintln!("文件已下载到桌面：{}", path.display());
                if action == FileAction::Open && should_open(user_id, config) {
                    open_path(&path);
                }
            }
            Err(e) => eprintln!("写文件到桌面失败：{e}"),
        }
    }
}

/// 当前或下一节课是不是这位好友对应的那门课，且现在没有全屏窗口。
fn should_open(user_id: i64, config: &Config) -> bool {
    if crate::win::current_window_state() == WindowState::Fullscreen {
        // 用户明确要求：全屏时只下载到桌面，不要抢焦点。
        eprintln!("当前有全屏窗口，只下载不打开");
        return false;
    }
    let Some(role) = config.roles.get(&user_id.to_string()) else {
        eprintln!("这位好友没有配身份（role），只下载不打开");
        return false;
    };
    let lessons = crate::schedule::current_and_next_subjects();
    let hit = lessons
        .iter()
        .any(|lesson| lesson.contains(role.as_str()) || role.contains(lesson.as_str()));
    if !hit {
        eprintln!("当前/下一节（{}）都不是「{}」的课，只下载不打开", lessons.join("、"), role);
    }
    hit
}

/// 问 OneBot 框架要文件的本地路径（get_file 返回 data.file）。
async fn local_source(bot: &RuntimeBot, file: &IncomingFile) -> Option<PathBuf> {
    let mut params = json!({ "file": file.name });
    if let Some(id) = &file.file_id {
        params["file_id"] = json!(id);
    }
    let ret = bot.send_api_return("get_file", params).await.ok()?;
    let raw = ret.data.get("file").and_then(|v| v.as_str())?.trim();
    if raw.is_empty() {
        return None;
    }
    let path = file_url_to_path(raw);
    if path.is_file() {
        Some(path)
    } else {
        eprintln!("get_file 给的路径不存在：{raw}");
        None
    }
}

/// OneBot 的 file 字段可能是本地路径，也可能是 file:///C:/... 形式。
fn file_url_to_path(raw: &str) -> PathBuf {
    match raw.strip_prefix("file://") {
        Some(rest) => {
            let rest = rest.trim_start_matches('/');
            PathBuf::from(rest.replace('/', "\\"))
        }
        None => PathBuf::from(raw),
    }
}

/// 用系统默认程序打开文件。走 cmd start，避免再引一个 shell 依赖。
#[cfg(windows)]
fn open_path(path: &Path) {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/C", "start", ""])
        .arg(path)
        .creation_flags(CREATE_NO_WINDOW);
    if let Err(e) = cmd.spawn() {
        eprintln!("打开文件失败：{e}");
    }
}

#[cfg(not(windows))]
fn open_path(_path: &Path) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ext_of_handles_dots_and_missing_extension() {
        assert_eq!(ext_of("课件.PPTX"), "pptx");
        assert_eq!(ext_of("a.b.docx"), "docx");
        assert_eq!(ext_of("noext"), "");
        assert_eq!(ext_of(".gitignore"), "");
        assert_eq!(ext_of("trailing."), "");
    }

    /// 默认策略就是用户拍的那份：办公文档打开、图片不管、其余兜底下载。
    #[test]
    fn default_policy_buckets() {
        let policy = FilePolicy::default();
        assert_eq!(plan("pptx", &policy), FileAction::Open);
        assert_eq!(plan("PPT", &policy), FileAction::Open);
        assert_eq!(plan("docx", &policy), FileAction::Open);
        assert_eq!(plan("jpg", &policy), FileAction::Ignore);
        assert_eq!(plan("webp", &policy), FileAction::Ignore);
        assert_eq!(plan("mp4", &policy), FileAction::Download);
        assert_eq!(plan("unknown", &policy), FileAction::Download);
        assert_eq!(plan("", &policy), FileAction::Download);
    }

    /// 配置里写出 ".PNG" 这种带点带大写的写法也要认得。
    #[test]
    fn policy_entries_are_normalized() {
        let policy = FilePolicy {
            open: vec![".PPTX".to_string()],
            ignore: vec![" PNG ".to_string()],
        };
        assert_eq!(plan("pptx", &policy), FileAction::Open);
        assert_eq!(plan("png", &policy), FileAction::Ignore);
    }

    /// 附件名不能带路径，重名也不能覆盖桌面上已有的文件。
    #[test]
    fn deliver_sanitizes_names_and_avoids_overwrite() {
        let dir = std::env::temp_dir().join(format!("astra-herald-files-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.bin");
        std::fs::write(&source, b"hello").unwrap();

        let first = deliver(&source, "..\\..\\evil.docx", &dir).unwrap();
        assert_eq!(first.file_name().unwrap(), "evil.docx");
        let second = deliver(&source, "evil.docx", &dir).unwrap();
        assert_eq!(second.file_name().unwrap(), "evil (2).docx");
        assert_eq!(std::fs::read(&second).unwrap(), b"hello");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
