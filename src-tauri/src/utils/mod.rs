mod acl;
pub mod friend_info;

use std::collections::{HashMap, HashSet};
pub use acl::set_access_control;
pub use acl::need_to_solve;
pub use friend_info::get_friend_info;
use crate::config::{Config, CONFIG_PATH};

/// WebView2 的内核缓存目录（实测 160 MB+）。默认在 `%LOCALAPPDATA%\<identifier>`，
/// 用户要求尽量别往 APPDATA 里塞，所以放到程序自己旁边的 `.webview2`；
/// 那里不可写（比如装进 Program Files）再退回 Tauri 的默认位置。
pub fn webview_data_dir(app: &tauri::AppHandle) -> std::path::PathBuf {
    use tauri::Manager as _;
    let beside_exe = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|dir| dir.join(".webview2")));
    if let Some(dir) = beside_exe {
        if std::fs::create_dir_all(&dir).is_ok() {
            return dir;
        }
    }
    app.path()
        .app_local_data_dir()
        .unwrap_or_else(|_| std::env::temp_dir().join("astra-herald-webview"))
}

/// 命中关键词对应的候选回复；命中多个 Key 时取 Value 的并集（重复项只留一份）。
pub fn find_matched_keywords<'a>(text: &str, keywords: &'a HashMap<String, Vec<String>>) -> Vec<&'a String> {
    let mut seen: HashSet<&str> = HashSet::new();
    let mut matched: Vec<&'a String> = Vec::new();
    for (keyword, replies) in keywords {
        if !text.contains(keyword.as_str()) {
            continue;
        }
        for reply in replies {
            if seen.insert(reply.as_str()) {
                matched.push(reply);
            }
        }
    }
    matched
}

pub fn get_config() -> Config {
    Config::load_or_create(CONFIG_PATH)
}