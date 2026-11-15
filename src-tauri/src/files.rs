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

/// 桌面目录：直接问系统要，不猜。
///
/// 桌面**不一定**是用户目录下的 Desktop。OneDrive 的「备份文件夹」（Known Folder Move）
/// 会把桌面整个搬走——一般是 OneDrive 目录下的 Desktop，企业版还会叫「OneDrive - 公司名」；
/// 同时注册表 User Shell Folders\Desktop 被改写。所以列候选目录永远列不全，
/// SHGetKnownFolderPath(FOLDERID_Desktop) 才是那份权威答案，重定向、改名、多语言它都替我们处理了。
#[cfg(windows)]
pub fn desktop_dir() -> Option<PathBuf> {
    use std::os::windows::ffi::OsStringExt;

    /// FOLDERID_Desktop = {B4BFCC3A-DB2C-424C-B029-7FE99A87C641}
    const FOLDERID_DESKTOP: Guid = Guid {
        data1: 0xB4BF_CC3A,
        data2: 0xDB2C,
        data3: 0x424C,
        data4: [0xB0, 0x29, 0x7F, 0xE9, 0x9A, 0x87, 0xC6, 0x41],
    };

    /// COINIT_APARTMENTTHREADED：桌面是 shell 对象，走 STA 最稳。
    const COINIT_APARTMENTTHREADED: u32 = 0x2;

    #[repr(C)]
    struct Guid {
        data1: u32,
        data2: u16,
        data3: u16,
        data4: [u8; 8],
    }

    #[link(name = "shell32")]
    unsafe extern "system" {
        fn SHGetKnownFolderPath(
            id: *const Guid,
            flags: u32,
            token: isize,
            path: *mut *mut u16,
        ) -> i32;
    }

    #[link(name = "ole32")]
    unsafe extern "system" {
        fn CoTaskMemFree(block: *mut core::ffi::c_void);
        fn CoInitializeEx(reserved: *mut core::ffi::c_void, coinit: u32) -> i32;
        fn CoUninitialize();
    }

    unsafe {
        // 微软文档明确要求：调用 SHGetKnownFolderPath 的线程必须先初始化 COM。
        // desktop_dir() 是在 handle() 里 .await 之后调的，可能落在没初始化过的 tokio
        // worker 上——不初始化就可能查失败，然后静默退回用户目录下的 Desktop，
        // 把「跟着 OneDrive 重定向走」这件事整个抵消掉。
        // S_OK(0) / S_FALSE(1) 都算这次初始化成功，退出时必须配对 CoUninitialize；
        // RPC_E_CHANGED_MODE(0x80010106，负数) 表示已被别的公寓初始化过，不能 uninit。
        let owned = CoInitializeEx(std::ptr::null_mut(), COINIT_APARTMENTTHREADED) >= 0;

        let mut raw: *mut u16 = std::ptr::null_mut();
        // 0 = KF_FLAG_DEFAULT；token 传 0 表示当前用户。
        let hr = SHGetKnownFolderPath(&FOLDERID_DESKTOP, 0, 0, &mut raw);
        let path = if hr < 0 || raw.is_null() {
            None
        } else {
            let mut len = 0usize;
            while *raw.add(len) != 0 {
                len += 1;
            }
            // 读和 free 的先后不能反：raw 是系统分配的，CoTaskMemFree 之后就成活野指针。
            let desktop = PathBuf::from(std::ffi::OsString::from_wide(
                std::slice::from_raw_parts(raw, len),
            ));
            CoTaskMemFree(raw as *mut core::ffi::c_void);
            Some(desktop)
        };

        if owned {
            CoUninitialize();
        }
        if path.is_some() {
            return path;
        }
        log::error!("SHGetKnownFolderPath 拿不到桌面目录，退回用户目录下的 Desktop");
        std::env::var_os("USERPROFILE").map(|home| PathBuf::from(home).join("Desktop"))
    }
}
#[cfg(not(windows))]
pub fn desktop_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Desktop"))
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
///
/// 占位必须原子：现在每条文件消息各跑一个 blocking 任务，两条都叫 notes.pdf 的消息
/// 会同时走这里。先 exists() 再 copy 的写法，两个任务可能都判定「不存在」，
/// 后完成的那份把先写好的覆盖掉——附件就这么静默丢了。所以用 create_new 独占创建
/// 占住名字（已存在会直接返回 AlreadyExists），再往这个已经属于自己的文件里写。
pub fn deliver(source: &Path, name: &str, dir: &Path) -> std::io::Result<PathBuf> {
    let safe = sanitize(name);
    let stem = Path::new(&safe)
        .file_stem()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("文件");
    let ext = Path::new(&safe)
        .extension()
        .and_then(std::ffi::OsStr::to_str)
        .unwrap_or("");

    let mut counter = 1;
    loop {
        let candidate = if counter == 1 {
            safe.clone()
        } else if ext.is_empty() {
            format!("{stem} ({counter})")
        } else {
            format!("{stem} ({counter}).{ext}")
        };
        let target = dir.join(candidate);
        // ponytail: 上限只是防死循环（目录里真有几千份同名附件时才会碰到），
        // 碰到了就报错，绝不覆盖。
        if counter > 1000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::AlreadyExists,
                "同名附件太多，找不到可用文件名",
            ));
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&target)
        {
            Ok(mut reserved) => {
                let mut src = std::fs::File::open(source)?;
                if let Err(e) = std::io::copy(&mut src, &mut reserved) {
                    drop(reserved);
                    // 别在桌面上留一个半截文件。
                    let _ = std::fs::remove_file(&target);
                    return Err(e);
                }
                return Ok(target);
            }
            // 名字被占了（别的任务先占到，或桌面上本来就有）：换个后缀重来。
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                counter += 1;
            }
            Err(e) => return Err(e),
        }
    }
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
        // 复制和删除都是阻塞 IO，直接在 async 里跑会堵住 tokio 的 worker
        // （sonar rust:S7493），所以整段挪进 spawn_blocking。中转文件的清理也一起丢进去，
        // 它本来就是「拷完就删」的收尾动作：download_file 落的那份是临时件，
        // get_file 给的可能是框架自己的缓存，不能删。
        let name = file.name.clone();
        let cleanup = staged;
        let task = kovi::tokio::task::spawn_blocking(move || {
            let outcome = deliver(&source, &name, &dir);
            if cleanup && let Err(e) = std::fs::remove_file(&source) {
                log::warn!("删不掉中转文件 {}：{e}", source.display());
            }
            outcome
        });
        match task.await {
            Ok(Ok(path)) => {
                log::info!("文件已下载到桌面：{}", path.display());
                if action == FileAction::Open && should_open(user_id, config) {
                    open_path(&path);
                }
            }
            Ok(Err(e)) => log::error!("写文件到桌面失败：{e}"),
            Err(e) => log::error!("下载任务被取消或 panic：{e}"),
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
    // 逐级降级，任何一级失败都继续试下一级——段内直链带 rkey，有时效，
    // 过期是常见情况，不能让它的失败把后面两次机会一起吃掉。
    if let Some(url) = file.url.as_deref().filter(|url| url.starts_with("http")) {
        if let Some(path) = download(bot, url, &file.name).await {
            return Some((path, true));
        }
        log::warn!("段内直链下载失败，试试重新签名：{}", file.name);
    }
    if let (Some(id), Some(hash)) = (&file.file_id, &file.file_hash) {
        let params = json!({ "user_id": user_id, "file_id": id, "file_hash": hash });
        match api_data(bot, "get_private_file_url", params).await {
            Some(data) => match field(&data, "url").filter(|url| url.starts_with("http")) {
                Some(url) => {
                    log::info!("重新签了一条直链：{}", file.name);
                    if let Some(path) = download(bot, &url, &file.name).await {
                        return Some((path, true));
                    }
                    log::warn!("重签的直链也下不下来，退回 get_file：{}", file.name);
                }
                None => log::warn!("get_private_file_url 没给可用的 url：{}", file.name),
            },
            None => log::warn!("重新签名失败，退回 get_file：{}", file.name),
        }
    }
    get_file_source(bot, file).await.map(|path| (path, false))
}

/// 把一条直链落到本机，返回本地路径（这一份是中转文件，用完要删）。
async fn download(bot: &RuntimeBot, url: &str, name: &str) -> Option<PathBuf> {
    // download_file 只取 basename，名字里不能带路径分隔符，先过一遍 sanitize。
    let params = json!({ "url": url, "name": sanitize(name) });
    let data = api_data(bot, "download_file", params).await?;
    let path = file_url_to_path(data.get("file").and_then(|v| v.as_str())?);
    if path.is_file() {
        return Some(path);
    }
    log::error!("download_file 说文件在 {}，本机却没有", path.display());
    None
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

    /// 桌面目录必须真实存在且是绝对路径——SHGetKnownFolderPath 失败时这条会红。
    #[cfg(windows)]
    #[test]
    fn desktop_dir_is_a_real_directory() {
        let dir = desktop_dir().expect("拿不到桌面目录");
        assert!(dir.is_absolute(), "桌面目录不是绝对路径: {}", dir.display());
        assert!(dir.is_dir(), "桌面目录不存在: {}", dir.display());
    }
    /// 并发落盘同名附件不能互相覆盖：模拟两条 notes.pdf 同时到。
    #[test]
    fn concurrent_deliver_keeps_both_files() {
        let dir = std::env::temp_dir().join(format!("astra-herald-race-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let source = dir.join("source.bin");
        std::fs::write(&source, b"payload").unwrap();

        let handles: Vec<_> = (0..8)
            .map(|_| {
                let (source, dir) = (source.clone(), dir.clone());
                std::thread::spawn(move || deliver(&source, "notes.pdf", &dir))
            })
            .collect();
        let paths: Vec<_> = handles
            .into_iter()
            .map(|h| h.join().unwrap().unwrap())
            .collect();

        // 8 个任务必须拿到 8 个互不相同的文件名，一个都不能丢。
        let mut names: Vec<String> = paths
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().to_string())
            .collect();
        names.sort();
        names.dedup();
        assert_eq!(names.len(), 8, "文件名重复了: {names:?}");
        for path in &paths {
            assert_eq!(std::fs::read(path).unwrap(), b"payload", "{} 内容不对", path.display());
        }

        let _ = std::fs::remove_dir_all(&dir);
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
