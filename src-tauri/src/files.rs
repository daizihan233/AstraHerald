//! 文件消息：分类（忽略 / 仅下载 / 下载后打开）、下载到桌面、按需打开。
//!
//! 分类只看后缀名，桶来自配置项 `files`（open = 下载后按需打开，ignore = 完全不管，
//! 其余兜底只下载）。取文件优先用文件段自带的 url（框架早就算好直链推过来了），
//! 交给 download_file 落到本机；kovi 没封装这些拓展动作，用
//! RuntimeBot::send_api_return 直接发（kovi-0.13.5/src/bot/runtimebot.rs:167）。
//!
//! 这里每一句诊断都必须走 log 而不是 eprintln!：GUI 版没有控制台，stderr 没人接，
//! 只有 log 才落进 app.log（同 bot.rs:118 的约束）。文件收发最怕「什么都没发生」。
//!
//! 打开要过两道门：窗口状态不是全屏/最大化，且这位好友的 role 命中「当前或下一节课」的科目名。

use crate::config::{Config, FilePolicy, WindowState};
use kovi::RuntimeBot;
use serde_json::{json, Value};
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
    /// 框架已经算好的下载直链（rkey 签名、有时效，所以要当场下）。
    pub url: Option<String>,
    /// 私聊文件哈希；url 过期时拿它重新签一条直链。
    pub file_hash: Option<String>,
}

/// 读段里一个非空字符串字段（框架有时给空串，当作没给）。
fn field(data: &Value, key: &str) -> Option<String> {
    data.get(key)
        .and_then(|value| value.as_str())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

/// 挑出消息里的文件段。
pub fn incoming(message: &kovi::message::Message) -> Vec<IncomingFile> {
    message
        .iter()
        .filter(|segment| segment.kind == "file")
        .filter_map(|segment| {
            let name = field(&segment.data, "file").unwrap_or_default();
            let file_id = field(&segment.data, "file_id");
            if name.is_empty() && file_id.is_none() {
                return None;
            }
            let name = if name.is_empty() { "文件".to_string() } else { name };
            Some(IncomingFile {
                name,
                file_id,
                url: field(&segment.data, "url"),
                file_hash: field(&segment.data, "file_hash"),
            })
        })
        .collect()
}

/// 桌面目录。桌面被重定向到 OneDrive、或中文系统那份叫「桌面」的都在这些位置。
///
/// ponytail: 严格做法是 SHGetKnownFolderPath(FOLDERID_Desktop)（win.rs:154 那种裸
/// extern 就够，不用加依赖）；本机注册表确认指向 %USERPROFILE%\Desktop，先按候选目录试。
pub fn desktop_dir() -> Option<PathBuf> {
    let home = std::env::var_os("USERPROFILE").map(PathBuf::from)?;
    let desktop = home.join("Desktop");
    if desktop.is_dir() {
        return Some(desktop);
    }
    let onedrive = home.join("OneDrive");
    for candidate in [onedrive.join("Desktop"), onedrive.join("桌面"), home.join("桌面")] {
        if candidate.is_dir() {
            return Some(candidate);
        }
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
    while target.exists() {
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
            log::info!("文件消息按策略忽略：{}", file.name);
            continue;
        }
        let Some((source, staged)) = local_source(&bot, user_id, &file).await else {
            log::error!("取文件失败（没拿到本地文件）：{}", file.name);
            continue;
        };
        let Some(dir) = desktop_dir() else {
            log::error!("找不到桌面目录，放弃下载：{}", file.name);
            continue;
        };
        match deliver(&source, &file.name, &dir) {
            Ok(path) => {
                log::info!("文件已下载到桌面：{}", path.display());
                if action == FileAction::Open && should_open(user_id, config) {
                    open_path(&path);
                }
            }
            Err(e) => log::error!("写文件到桌面失败：{e}"),
        }
        // download_file 落的那份是中转，拷完就删，别在框架的数据目录里堆垃圾。
        // get_file 给的路径可能是框架自己的缓存，不能删。
        if staged && let Err(e) = std::fs::remove_file(&source) {
            log::warn!("删不掉中转文件 {}：{e}", source.display());
        }
    }
}

/// 窗口状态是否一票否决自动打开。判定本身是纯函数，方便单测。
///
/// 最大化也算：用户可能正把别的窗口铺满屏幕看东西，弹窗抢焦点同样打扰。
fn blocked_by_window_state(state: WindowState) -> bool {
    matches!(state, WindowState::Fullscreen | WindowState::Maximized)
}

/// 窗口状态放行后，再看当前或下一节课是不是这位好友对应的那门课。
fn should_open(user_id: i64, config: &Config) -> bool {
    let state = crate::win::current_window_state();
    if blocked_by_window_state(state) {
        // 用户明确要求：全屏 / 最大化时（不论上课还是下课）都只下载到桌面，不抢焦点。
        log::info!("当前窗口是{}，只下载不打开", state.label());
        return false;
    }
    let Some(role) = config.roles.get(&user_id.to_string()) else {
        log::info!("这位好友没有配身份（role），只下载不打开");
        return false;
    };
    let lessons = crate::schedule::current_and_next_subjects();
    let hit = lessons
        .iter()
        .any(|lesson| lesson.contains(role.as_str()) || role.contains(lesson.as_str()));
    if !hit {
        log::info!("当前/下一节（{}）都不是「{}」的课，只下载不打开", lessons.join("、"), role);
    }
    hit
}

/// 发一个拓展动作；失败就把 status/retcode 记进日志。
///
/// 回包 status 不是 ok 时 kovi 走 Err 分支（kovi-onebot api_cnt.rs:200-204），
/// SnowLuma 的失败原因写在 wording 字段，kovi 的 ApiReturn 没这个字段，所以这里只剩
/// retcode 可看——框架自己那行 warn 里带着原始报文，会一起落进 app.log。
async fn api_data(bot: &RuntimeBot, action: &str, params: Value) -> Option<Value> {
    match bot.send_api_return(action, params).await {
        Ok(ret) => Some(ret.data),
        Err(ret) => {
            log::error!("{action} 调用失败：status={} retcode={}", ret.status, ret.retcode);
            None
        }
    }
}

/// 搞到文件的本地路径，返回（路径，是不是这次下的中转文件）。
///
/// 顺序：段里自带的 url → download_file；只有 file_id + file_hash → 先
/// get_private_file_url 重签一条直链再 download_file；都没有才退回 get_file。
/// get_file 在 SnowLuma 那边只解析图片/语音缓存（index.mjs 的 get_file 定义写明
/// 「群文件请用 get_group_file_url」），普通私聊文件问它拿不到本地路径。
async fn local_source(bot: &RuntimeBot, user_id: i64, file: &IncomingFile) -> Option<(PathBuf, bool)> {
    let url = match &file.url {
        Some(url) if url.starts_with("http") => Some(url.clone()),
        _ => match (&file.file_id, &file.file_hash) {
            (Some(id), Some(hash)) => {
                let params = json!({ "user_id": user_id, "file_id": id, "file_hash": hash });
                let data = api_data(bot, "get_private_file_url", params).await?;
                let url = field(&data, "url")?;
                if url.starts_with("http") {
                    log::info!("直链过期了，重新签了一条：{}", file.name);
                    Some(url)
                } else {
                    None
                }
            }
            _ => None,
        },
    };
    if let Some(url) = url {
        // download_file 只取 basename，名字里不能带路径分隔符，先过一遍 sanitize。
        let params = json!({ "url": url, "name": sanitize(&file.name) });
        let data = api_data(bot, "download_file", params).await?;
        let staged = file_url_to_path(data.get("file").and_then(|v| v.as_str())?);
        if staged.is_file() {
            return Some((staged, true));
        }
        log::error!("download_file 说文件在 {}，本机却没有", staged.display());
        return None;
    }
    get_file_source(bot, file).await.map(|path| (path, false))
}

/// 旧路子：问 get_file 要本地路径，留着兜底（换个框架可能真给路径）。
async fn get_file_source(bot: &RuntimeBot, file: &IncomingFile) -> Option<PathBuf> {
    let mut params = json!({ "file": file.name });
    if let Some(id) = &file.file_id {
        params["file_id"] = json!(id);
    }
    let data = api_data(bot, "get_file", params).await?;
    let raw = data.get("file").and_then(|v| v.as_str()).unwrap_or("").trim();
    if raw.is_empty() {
        log::warn!("get_file 没给路径，段里也没有 url：{}", file.name);
        return None;
    }
    let path = file_url_to_path(raw);
    if path.is_file() {
        return Some(path);
    }
    log::warn!("get_file 给的路径不存在：{raw}");
    None
}

/// OneBot 的 file 字段可能是本地路径，也可能是 file:///C:/... 形式。
///
/// ponytail: 不做百分号解码，中文名或带空格的 file:/// URL 会被当普通字符串。
/// 主路径（download_file）给的是绝对路径，走不到这里，所以先不引 percent-encoding。
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
        log::error!("打开文件失败：{e}");
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

    /// 全屏和最大化都拦，其余状态交给课程判定。
    #[test]
    fn window_state_blocks_auto_open() {
        assert!(blocked_by_window_state(WindowState::Fullscreen));
        assert!(blocked_by_window_state(WindowState::Maximized));
        assert!(!blocked_by_window_state(WindowState::Desktop));
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

    /// 真机上报过的私聊文件段（SnowLuma 那份 data）：直链和哈希必须在段里就读到。
    #[test]
    fn incoming_reads_the_real_file_segment() {
        let message = kovi::message::Message::from_value(json!([
            {
                "type": "file",
                "data": {
                    "file": "Test.MP3",
                    "file_id": "f7ac2c7b_ad4e9bca",
                    "file_size": 470254,
                    "name": "Test.MP3",
                    "size": 470254,
                    "url": "http://49.86.42.34:80/asn.com/qqdownload?rkey=abc",
                    "file_hash": "D6EATZ6OCPfZstkHEhRm5LMn876MmLfK"
                }
            }
        ]))
        .unwrap();
        let files = incoming(&message);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].name, "Test.MP3");
        assert_eq!(files[0].file_id.as_deref(), Some("f7ac2c7b_ad4e9bca"));
        assert_eq!(
            files[0].url.as_deref(),
            Some("http://49.86.42.34:80/asn.com/qqdownload?rkey=abc")
        );
        assert_eq!(files[0].file_hash.as_deref(), Some("D6EATZ6OCPfZstkHEhRm5LMn876MmLfK"));
    }

    /// 只有名字、url 是空串的旧格式也要收下，退回 get_file 那条兜底路。
    #[test]
    fn incoming_tolerates_segments_without_url() {
        let message = kovi::message::Message::from_value(json!([
            { "type": "file", "data": { "file": "课件.docx", "url": "", "file_hash": "" } }
        ]))
        .unwrap();
        let files = incoming(&message);
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].file_id, None);
        assert_eq!(files[0].url, None);
        assert_eq!(files[0].file_hash, None);
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
