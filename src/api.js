import { invoke } from "@tauri-apps/api/core";

/// 读配置。文件不存在或解析失败时，后端会先落盘一份默认配置再返回。
export function loadConfig() {
  return invoke("get_config");
}

/// 写配置。后端会先用 serde 解析 + 校验再落盘，形状不对直接报错，不会写坏磁盘上那一份。
export function saveConfig(config) {
  return invoke("save_config", { config });
}

/// 「一键配置」好友身份：后端拉一次好友列表，用备注名去撞课表里的课程名
/// （课表服务 127.0.0.1:3011 的 /subjects），撞不上落「课程」。
/// 返回 { QQ号: 身份 }；bot 还没连上或没好友时抛错。
export function autoFillRoles() {
  return invoke("auto_fill_roles");
}

/// 连接状态（配置页顶栏那两盏灯）：与 OneBot 框架的通道、与 AstraSchedule
/// 的状态接口，各自 { ok, detail }，外加当前生效的上课状态。后端不缓存，问一次查一次。
/// 「连接配置」卡片的「自动发现」：从启动文件附近读框架自己的 OneBot 配置。
/// 返回 { dir, host, port, path, access_token }；路径不对或读不出来时抛错。
export function discoverOnebot(startup) {
  return invoke("discover_onebot", { startup });
}

export function connectionStatus() {
  return invoke("connection_status");
}

/// 让后端弹一条侧通知样品（「侧通知」卡片里那个按钮）：
/// 不经过 bot，也用不着真的收到一条私聊。
/// font（正文）/ nickFont（昵称）/ secs 传配置页里当前的值（可能还没保存），
/// 预览的就是马上要生效的样子；不传就用落盘配置里的值。
export function previewSideNotice({ font, nickFont, secs } = {}) {
  return invoke("notice_preview_side", { font, nick_font: nickFont, secs });
}
/// 聊天记录：最近 500 条文字消息，最新的在前。
export function historyList() {
  return invoke("history_list");
}

/// 重放选中的那几条：效果和「这些消息刚刚又收到了」一样（同一套场景矩阵）。
/// 已经在提醒窗口里显示着的那条会被去重跳过，所以返回的是真正又展示了几条。
export function historyReplay(ids) {
  return invoke("history_replay", { ids });
}

/// 「好友身份」下拉的候选项：全量好友，value 是 QQ 号（存盘的就是它），
/// label 是「备注 [用户名] （QQ号）」。bot 还没连上时抛错。
export function friendOptions() {
  return invoke("friend_options");
}