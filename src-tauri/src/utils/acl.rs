//! 访问控制（ACL）：把 config.json 的名单挂到 bot 上。

use std::sync::Arc;

use kovi::RuntimeBot;
use kovi::event::id::ID;
use kovi::event::MessageEventTrait;
use kovi::log::{Level, log};
use kovi::plugin::{AccessControlMode, SetAccessControlList};
use kovi_onebot::MsgEvent;
use crate::utils::{get_config, get_friend_info};

/// config.json 里的 ID 只能写成字符串（kovi 0.13.5 的 ID 反序列化没有 visit_u64，
/// 写数字会直接解析失败），而事件侧的 ID 是整数；ID 与 RefID 的相等判断是变体严格相等
/// （id.rs:120-128 的 _ => false），所以名单必须先归一化成 ID::Int，否则白名单永远命中不了。
fn normalize_id(id: &ID) -> ID {
    match id.try_as_str().and_then(|s| s.parse::<i64>().ok()) {
        Some(n) => ID::new(n),
        None => id.clone(),
    }
}

/// 插件名由 #[kovi::plugin] 宏用 env!("CARGO_PKG_NAME") 生成：旧仓库的插件是独立 crate
/// `hello`，搬进 Tauri 后包名变成 `astra-herald`。写死字符串会在搬 crate 时静默错位
/// ——set_* 找不到插件会返回 Err(BotError::PluginNotFound)（kovi_api.rs:77-80），
/// 于是 .expect 直接 panic。
///
/// 这里用同一个表达式而不是 `PluginBuilder::get_plugin_name()`：后者读的是插件 main()
/// 期间的 task-local，在保存配置的 tauri 命令线程里调会 panic「Using PluginBuilder in
/// wrong place」（plugin_builder.rs:115）。
const PLUGIN_NAME: &str = env!("CARGO_PKG_NAME");

pub fn set_access_control(bot: Arc<RuntimeBot>) {
    let name = PLUGIN_NAME;
    bot.set_plugin_access_control(&name, true)
        .expect("获取 bot 实例时发生了意外");
    let config = get_config();
    if config.whitelist.is_empty() {  // 黑名单
        bot.set_plugin_access_control_mode(&name, AccessControlMode::BlackList)
            .expect("调用 bot 实例以设置 AC 模式时发生了意外");
        let acl: Vec<ID> = config.blacklist.iter().map(normalize_id).collect();
        bot.set_plugin_access_control_list(
            &name, false, SetAccessControlList::Changes(acl)
        ).expect("调用 bot 实例以设置 ACL 时发生了意外");
        log!(Level::Info, "ACL 成功：黑名单")
    } else { // 白名单
        bot.set_plugin_access_control_mode(&name, AccessControlMode::WhiteList)
            .expect("调用 bot 实例以设置 AC 模式时发生了意外");
        let acl: Vec<ID> = config.whitelist.iter().map(normalize_id).collect();
        bot.set_plugin_access_control_list(
            &name, false, SetAccessControlList::Changes(acl)
        ).expect("调用 bot 实例以设置 ACL 时发生了意外");
        log!(Level::Info, "ACL 成功：白名单")
    }
}

pub async fn need_to_solve(bot: Arc<RuntimeBot>, event: Arc<MsgEvent>) -> bool {
    let usr_info = get_friend_info(bot, event.get_sender_id().to_id()).await;
    if !event.is_private() {
        log!(Level::Debug, "{} 由于不是私聊消息被忽略", usr_info);
        return false;
    }
    true
}