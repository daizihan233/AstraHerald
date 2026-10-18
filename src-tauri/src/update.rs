//! 自动更新：查有没有新版本、下载并安装。
//!
//! 走 tauri-plugin-updater 的 Rust API 而不是 JS 插件：更新源要能在配置页里改
//! （config.rs 的 UpdaterSettings），而插件配置里的 endpoints 是编译期固定的。
//!
//! 签名校验由插件负责：公钥写死 in tauri.conf.json 的 plugins.updater.pubkey，私钥在 CI。

use serde::Serialize;
use tauri::{AppHandle, Runtime, Url};
use tauri_plugin_updater::{Update, UpdaterExt};

use crate::config::{Config, CONFIG_PATH};

/// 给配置页看的更新信息。
#[derive(Debug, Clone, Serialize)]
pub struct UpdateInfo {
    pub version: String,
    /// 当前版本，页面上显示「现在 -> 可用」。
    pub current: String,
    /// 更新说明（latest.json 的 notes，就是那条提交的标题）。
    pub notes: String,
}

/// 查一次更新。已经是最新时返回 None。
///
/// 手动检查不看 updater.enabled——那个开关只管「启动时自动查一次」。
#[tauri::command]
pub async fn check_update(app: AppHandle) -> Result<Option<UpdateInfo>, String> {
    Ok(find_update(&app).await?.map(|update| UpdateInfo {
        version: update.version.clone(),
        current: update.current_version.clone(),
        notes: update.body.clone().unwrap_or_default(),
    }))
}

/// 下载并安装。Windows 上装完这个进程就被插件自己结束了，
/// 所以前端 invoke 之后通常等不到返回值；失败（下载失败、签名不对）才会回来。
#[tauri::command]
pub async fn install_update(app: AppHandle) -> Result<(), String> {
    let Some(update) = find_update(&app).await? else {
        return Err("已经是最新版本了".to_string());
    };
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|error| format!("下载或安装更新失败：{error}"))
}

/// 按当前配置建一个更新器，查一次。
async fn find_update<R: Runtime>(app: &AppHandle<R>) -> Result<Option<Update>, String> {
    let source = Config::load_or_create(CONFIG_PATH)
        .updater
        .source
        .trim()
        .to_string();

    let mut builder = app.updater_builder();
    if !source.is_empty() {
        // 更新源来自配置页，不是网络输入，但填错了也得给一句能看懂的日志。
        let url: Url = source
            .parse()
            .map_err(|error| format!("更新源不是合法地址：{source}（{error}）"))?;
        builder = builder
            .endpoints(vec![url])
            .map_err(|error| format!("更新源用不了：{error}"))?;
    }
    let updater = builder
        .build()
        .map_err(|error| format!("初始化更新器失败：{error}"))?;
    updater
        .check()
        .await
        .map_err(|error| format!("检查更新失败：{error}"))
}
