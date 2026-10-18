//! 私聊提醒窗口：收到私聊消息时弹出来，用大字号显示消息内容。

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use kovi::{Message, RuntimeBot};
use kovi_onebot::{MessageRegistrar as _, OnebotTrait};
use serde::Serialize;
use tauri::{
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

/// 提醒窗口的 label，和配置页窗口（main）分开。
pub const NOTICE_WINDOW_LABEL: &str = "notice";

/// 提醒窗口加载的页面，由 vite 的多页构建产出（见 vite.config.js 的 rollupOptions.input）。
const NOTICE_PAGE: &str = "notice.html";

/// 窗口已经在的时候，用这个事件把新消息推进去。
const NOTICE_EVENT: &str = "notice:new";

#[derive(Debug, Clone, Serialize)]
pub struct NoticePayload {
    /// 单调递增，且跨会话不重置：前端拿它区分「这条是不是新的」，
    /// 点候选回复（notice_reply）也用它指明回的是哪一条。
    pub id: u64,
    pub text: String,
    /// 顶栏显示「备注 [用户名] （QQ号）」，由 utils::friend_info 查好（见 bot.rs）。
    pub sender: String,
    /// 这条消息命中的关键词对应的候选回复。前端渲染成一排浮动按钮，点一下发出去。
    pub replies: Vec<String>,
}

/// 点候选回复时要知道发给谁、引用哪条消息。
///
/// 消息事件里现成就有（MsgEvent 的 user_id / message_id），随提醒一起存下来，
/// 免得点按钮时再去查一次。
#[derive(Debug, Clone, Copy)]
pub struct ReplyTarget {
    pub sender_id: i64,
    /// 0 表示没有原始消息（聊天记录重放出来的），回复时不带引用段。
    pub message_id: i32,
}

/// 一次「强提醒会话」里的一条消息：给前端看的负载 + 回复要用的地址。
#[derive(Clone)]
struct Current {
    payload: NoticePayload,
    target: ReplyTarget,
    /// 这条消息对应聊天记录里的哪一条（真收到的消息也有）。重放同一条时据此去重。
    history_id: Option<u64>,
}

/// 当前这一段强提醒会话。
///
/// 语义：窗口从弹出来到被主动关掉之间算一段时间，这段时间里收到的消息都堆在同一个
/// 窗口里用分割布局并排显示——而不是每来一条就把上一条顶掉（被顶掉的那条连候选回复
/// 和回复目标一起丢）。窗口重新显示（见 `show` 里的 `is_visible` 判断）就清空，
/// 所以「关掉之后再收到消息」自然是一段新会话。
#[derive(Default)]
pub struct NoticeState {
    next_id: AtomicU64,
    session: Mutex<Vec<Current>>,
}

impl NoticeState {
    /// 锁只在读/改这个 Vec 时短暂持有，绝不跨 await。
    fn lock(&self) -> std::sync::MutexGuard<'_, Vec<Current>> {
        self.session.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn push(
        &self,
        text: String,
        sender: String,
        replies: Vec<String>,
        target: ReplyTarget,
        history_id: Option<u64>,
    ) -> Option<NoticePayload> {
        let mut session = self.lock();
        // 同一条聊天记录已经在窗口里了（重放按钮被连点、或者选中里混了正在显示的那条）：
        // 当没发生，别把同一条消息叠成两栏。
        if let Some(id) = history_id {
            if session.iter().any(|current| current.history_id == Some(id)) {
                return None;
            }
        }
        let payload = NoticePayload {
            id: self.next_id.fetch_add(1, Ordering::SeqCst) + 1,
            text,
            sender,
            replies,
        };
        session.push(Current {
            payload: payload.clone(),
            target,
            history_id,
        });
        Some(payload)
    }

    fn snapshot(&self) -> Vec<NoticePayload> {
        self.lock()
            .iter()
            .map(|current| current.payload.clone())
            .collect()
    }

    /// 取走某条消息（回复用）：取出来就不再留在会话里了。
    fn take(&self, id: u64) -> Option<ReplyTarget> {
        let mut session = self.lock();
        let index = session.iter().position(|current| current.payload.id == id)?;
        Some(session.remove(index).target)
    }

    /// 关掉一条消息 / 某个发送者的整列 / 整段会话，返回剩下的。
    fn close(&self, id: Option<u64>, sender: Option<&str>) -> Vec<NoticePayload> {
        {
            let mut session = self.lock();
            match (id, sender) {
                (Some(id), _) => session.retain(|current| current.payload.id != id),
                (None, Some(sender)) => session.retain(|current| current.payload.sender != sender),
                (None, None) => session.clear(),
            }
        }
        self.snapshot()
    }

    fn clear(&self) {
        self.lock().clear();
    }
}

/// 提醒窗口挂载时补拉一次：当前会话里的全部消息，按到达顺序。
#[tauri::command]
pub fn notice_current(state: tauri::State<'_, NoticeState>) -> Vec<NoticePayload> {
    state.snapshot()
}

/// 提醒窗口上点了候选回复：按「引用那条消息」的形式私聊发回去。
///
/// `id` 是消息在会话里的编号——一个窗口里可能并排着好几条消息，回复必须点哪条发哪条。
/// 返回剩下的消息；空了就把窗口收起来。
#[tauri::command]
pub fn notice_reply(
    id: u64,
    reply: String,
    state: tauri::State<'_, NoticeState>,
) -> Result<Vec<NoticePayload>, String> {
    let reply = reply.trim();
    if reply.is_empty() {
        return Err("回复内容为空".to_string());
    }

    let bot = bot_handle().ok_or_else(|| "bot 还没就绪".to_string())?;
    let target = state
        .take(id)
        .ok_or_else(|| "这条消息已经不在提醒窗口里了".to_string())?;

    // 和 kovi 自己的 reply_and_quote 一样：先加引用段再发
    // （kovi-onebot-0.13.2/src/event/private_msg_event.rs:259-275）。
    // message_id 0 是聊天记录重放出来的消息：没有原始消息可引用，直接发。
    let message = if target.message_id == 0 {
        Message::from(reply)
    } else {
        Message::from(reply).add_reply(target.message_id)
    };
    bot.send_private_msg(target.sender_id, message);

    let rest = state.snapshot();
    if rest.is_empty() {
        hide_notice();
    }
    Ok(rest)
}

/// 关掉一条消息 / 某个发送者的一整列 / 整段会话（都不给就是整段），返回剩下的消息。
#[tauri::command]
pub fn notice_close(
    id: Option<u64>,
    sender: Option<String>,
    state: tauri::State<'_, NoticeState>,
) -> Vec<NoticePayload> {
    let rest = state.close(id, sender.as_deref());
    if rest.is_empty() {
        hide_notice();
    }
    rest
}

/// 发回复用的 bot。
///
/// 消息回调里拿得到（bot.rs 的 plugin_main），提醒窗口的 IPC 拿不到——存一份。
/// 断线要能清掉（bot.rs 的无限重连每轮都会换一个新 bot），所以不是 OnceLock：
/// 状态灯和「点候选回复」都得知道此刻到底连上没有。锁只短暂持有，绝不跨 await。
static BOT: std::sync::Mutex<Option<Arc<RuntimeBot>>> = std::sync::Mutex::new(None);

/// 在 bot.rs 的插件入口里调一次。
pub fn set_bot(bot: Arc<RuntimeBot>) {
    if let Ok(mut guard) = BOT.lock() {
        *guard = Some(bot);
    }
}

/// 连接断开时由 bot.rs 调：句柄作废，界面上的状态灯立刻转红。
pub fn clear_bot() {
    if let Ok(mut guard) = BOT.lock() {
        *guard = None;
    }
}

/// 别处要发消息（例如 roles.rs 拉好友列表）时从这里取 bot。
pub fn bot_handle() -> Option<Arc<RuntimeBot>> {
    BOT.lock().ok().and_then(|guard| guard.clone())
}

/// 提醒窗口要用的 AppHandle。
///
/// kovi 的插件入口是 `#[kovi::plugin]` 生成的固定无参函数（见 lib.rs），消息回调里拿不到
/// Tauri 的 AppHandle——只能在这里存一份。
static APP: OnceLock<AppHandle> = OnceLock::new();

/// 在 Tauri setup 里调一次。
pub fn init(app: &AppHandle) {
    app.manage(NoticeState::default());
    app.manage(SideState::default());
    let _ = APP.set(app.clone());

    // 窗口预建好、藏起来待命。
    //
    // 以前是「收到消息才建窗」：建 WebView + 加载页面要好几百毫秒，那期间窗口是空白的，
    // 看起来就是「卡一下才出来」。预建之后收到消息只剩 emit + show 两下。
    if let Err(e) = build_window(app, false) {
        eprintln!("预建提醒窗口失败：{e}");
    }

    // 关闭 = 隐藏（和配置页同款）：窗口留着，下一条消息 show 一下就行，不用重建。
    if let Some(window) = app.get_webview_window(NOTICE_WINDOW_LABEL) {
        let window_to_hide = window.clone();
        let handle = app.clone();
        window.on_window_event(move |event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                // 关窗 = 这一段时间结束：清掉会话，下次来消息从空窗口开始堆。
                handle.state::<NoticeState>().clear();
                let _ = window_to_hide.hide();
            }
        });
    }
}

/// 收到一条私聊消息时调用。
///
/// 黑白名单不在这里再判一次：kovi 在分发事件给插件之前就做过 ACL
/// （kovi-0.13.5/src/bot/handler.rs:145 的 is_access），能走到这里的都是允许的发送者。
pub fn show(
    text: &str,
    sender: &str,
    replies: Vec<String>,
    target: ReplyTarget,
    history_id: Option<u64>,
) -> bool {
    let Some(app) = APP.get() else {
        eprintln!("提醒窗口尚未初始化，跳过这条消息");
        return false;
    };

    let text = text.trim();
    if text.is_empty() {
        // 没有任何可显示的内容（空消息）。弹一个空窗口只会让人困惑。
        return false;
    }

    let state = app.state::<NoticeState>();
    let window = app.get_webview_window(NOTICE_WINDOW_LABEL);

    // 窗口不可见 = 上一段会话已经结束了（被关掉、或回复完自动收起），这条消息开一段新的。
    // 「从弹出来到被主动关掉」就是用户定义的那一段时间。
    if !window
        .as_ref()
        .is_some_and(|window| window.is_visible().unwrap_or(false))
    {
        state.clear();
    }

    // 先落快照再推事件：事件可能没人接（页面还没挂上），快照能兜住。
    // 同一条聊天记录已经在窗口里时 push 返回 None：不重复弹，也不打断当前会话。
    let Some(payload) = state.push(
        text.to_string(),
        sender.trim().to_string(),
        replies,
        target,
        history_id,
    ) else {
        return false;
    };

    match window {
        Some(window) => {
            if let Err(e) = window.emit(NOTICE_EVENT, payload) {
                eprintln!("推送提醒到窗口失败：{e}");
            }
            let _ = window.unminimize();
            // 强提醒的定义就是压在别的窗口上面：每次显示都把置顶重新压实一遍，
            // 这样用户手动取消过置顶、或者别的程序动过 z-order，下一条消息还能立起来。
            if let Err(error) = window.set_always_on_top(true) {
                eprintln!("设置提醒窗口置顶失败：{error}");
            }
            let _ = window.show();
            let _ = window.set_focus();
        }
        // 正常走不到这里（init 里预建过）。万一把窗口弄丢了，补一个出来。
        None => {
            if let Err(e) = build_window(app, true) {
                eprintln!("创建提醒窗口失败：{e}");
            }
        }
    }
    true
}

/// 会话结束（消息都回复完、关完了）就把窗口收起来。
///
/// 窗口留着复用，不销毁——下一条消息 show 一下就行，不用重建 WebView。
/// 下一次 `show` 会因为窗口不可见而清掉上一段会话，所以这里只管收窗。
fn hide_notice() {
    let Some(app) = APP.get() else {
        return;
    };
    if let Some(window) = app.get_webview_window(NOTICE_WINDOW_LABEL) {
        let _ = window.hide();
    }
}

fn build_window(app: &AppHandle, visible: bool) -> tauri::Result<()> {
    // 先按兜底尺寸建出来，设好尺寸和位置再显示：
    // 否则窗口会先以默认尺寸闪一下、再跳成屏幕的 70%。
    let window = WebviewWindowBuilder::new(app, NOTICE_WINDOW_LABEL, WebviewUrl::App(NOTICE_PAGE.into()))
        .data_directory(crate::utils::webview_data_dir(app))
        .title("收到私聊消息")
        .inner_size(640.0, 520.0)
        // 底部按钮列固定吃 228px（见 NoticeWindow.vue 的 padding-bottom），窗口不能比它矮，
        // 否则候选菜单一展开就得自己滚。
        .min_inner_size(420.0, 360.0)
        // 提醒窗被别的窗口压住就失去意义了。
        .always_on_top(true)
        .visible(false)
        .build()?;

    fit_to_screen(&window);

    if visible {
        window.show()?;
    }
    Ok(())
}

/// 把提醒窗设成主显示器尺寸的 70%，并居中到屏幕中央。
///
/// 只在建窗时做一次：用户自己挪过、拉过窗口之后，再来消息不会被拽回原位。
fn fit_to_screen(window: &tauri::WebviewWindow) {
    const RATIO: f64 = 0.7;
    /// 不会小于 min_inner_size 定的下限，避免小屏幕上算出比下限还小的尺寸。
    const MIN_WIDTH: f64 = 420.0;
    const MIN_HEIGHT: f64 = 360.0;

    let monitor = match window.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => {
            eprintln!("找不到主显示器，提醒窗口沿用默认尺寸");
            return;
        }
        Err(error) => {
            eprintln!("查询显示器失败，提醒窗口沿用默认尺寸：{error}");
            return;
        }
    };

    let screen = monitor.size();
    let origin = monitor.position();
    let width = (screen.width as f64 * RATIO).round().max(MIN_WIDTH);
    let height = (screen.height as f64 * RATIO).round().max(MIN_HEIGHT);
    let x = (origin.x as f64 + (screen.width as f64 - width) / 2.0).round();
    let y = (origin.y as f64 + (screen.height as f64 - height) / 2.0).round();

    if let Err(error) = window.set_size(PhysicalSize::new(width as u32, height as u32)) {
        eprintln!("设置提醒窗口尺寸失败：{error}");
        return;
    }
    if let Err(error) = window.set_position(PhysicalPosition::new(x as i32, y as i32)) {
        eprintln!("设置提醒窗口位置失败：{error}");
    }
}

// ---------------------------------------------------------------------------
// 侧通知（侧提醒）：屏幕右上角的小卡片，到点自己收起来 —— 类似 naive-ui 的 Notification。
// ---------------------------------------------------------------------------

/// 侧通知窗口的 label 池。每个 label 一个常驻（隐藏）窗口，来回复用，
/// 省掉「收到消息才建 WebView」那几百毫秒的空白。
const SIDE_LABELS: [&str; 3] = ["notice-side-1", "notice-side-2", "notice-side-3"];

/// 侧通知加载的页面，同样由 vite 的多页构建产出。
const SIDE_PAGE: &str = "side.html";

/// 窗口已经建好的时候，用这个事件把新内容推进去。
const SIDE_EVENT: &str = "side:new";

/// 卡片宽度、距屏幕边沿和条与条之间的留白（逻辑像素）。
/// 宽度跟着字号一起放大过：360 太窄，17px 的正文一行放不下几个字。
const SIDE_WIDTH: f64 = 400.0;
const SIDE_MARGIN: f64 = 16.0;
const SIDE_GAP: f64 = 12.0;

/// 页面量出真实高度之前先按这个高度摆。
const SIDE_FALLBACK_HEIGHT: u32 = 120;

/// 页面报上来的高度会被夹在这个区间：太矮文字挤成一团，太高就不像通知了。
const SIDE_MIN_HEIGHT: f64 = 64.0;
const SIDE_MAX_HEIGHT: f64 = 420.0;

/// 自动收起的时间（配置页可改，见 config.rs 的 SideSettings）。
///
/// 侧通知页面里那条进度线用的是同一个值：它自己 invoke("get_config") 读 side.duration_secs。
fn side_duration_ms() -> u64 {
    let secs = crate::config::Config::load_or_create(crate::config::CONFIG_PATH)
        .side
        .clamped_secs();
    secs * 1000
}

/// 一条正在显示的侧通知。
struct SideSlot {
    label: String,
    /// 当前高度（物理像素），页面量好后通过 side_resize 报上来。
    height: u32,
    payload: NoticePayload,
    /// 对应聊天记录里的哪一条：重放同一条时不重复弹卡片。
    history_id: Option<u64>,
}

/// 侧通知的共享状态。
#[derive(Default)]
pub struct SideState {
    inner: Mutex<SidePanel>,
}

#[derive(Default)]
struct SidePanel {
    /// 从上到下的显示顺序：第一个贴着屏幕右上角。
    slots: Vec<SideSlot>,
    /// 每个 label「第几次显示」。自动收起的线程带着创建时的值回来，对不上就说明这条窗口
    /// 已经被复用成新消息了，不能再关。
    generations: HashMap<String, u64>,
    seq: u64,
}

impl SidePanel {
    /// 把一条新内容放到栈顶，返回要用的窗口 label、这一代的编号、以及被挤掉的那条。
    fn push(
        &mut self,
        text: String,
        sender: String,
        history_id: Option<u64>,
    ) -> Option<(String, u64, Option<String>)> {
        // 已经在右上角挂着的同一条聊天记录：不再弹第二张卡。
        if let Some(id) = history_id {
            if self.slots.iter().any(|slot| slot.history_id == Some(id)) {
                return None;
            }
        }
        self.seq += 1;
        // replies 恒空：侧通知不给候选回复（真回复请走强提醒窗）。这里留着这个字段是因为
        // 侧通知和强提醒共用 NoticePayload，拆开得不偿失。
        let payload = NoticePayload {
            id: self.seq,
            text,
            sender,
            replies: Vec::new(),
        };

        // 先找一个空着的窗口 label。
        let mut label = String::new();
        for candidate in SIDE_LABELS {
            if !self.slots.iter().any(|slot| slot.label == candidate) {
                label = candidate.to_string();
                break;
            }
        }

        // 三条都在显示：把最下面（最早）那条换掉。
        let mut evicted = None;
        if label.is_empty() {
            let oldest = self.slots.pop().expect("侧通知栈不可能为空");
            evicted = Some(oldest.label.clone());
            label = oldest.label;
        }

        let generation = {
            let generation = self.generations.entry(label.clone()).or_insert(0);
            *generation += 1;
            *generation
        };
        self.slots.insert(
            0,
            SideSlot {
                label: label.clone(),
                height: SIDE_FALLBACK_HEIGHT,
                payload,
                history_id,
            },
        );
        Some((label, generation, evicted))
    }
}

/// 侧通知窗口挂载时补拉一次。理由和 notice_current 一样：emit 可能早于页面注册监听。
#[tauri::command]
pub fn side_current(
    window: WebviewWindow,
    state: tauri::State<'_, SideState>,
) -> Option<NoticePayload> {
    let panel = state.inner.lock().expect("侧通知状态锁被 poison");
    let payload = panel
        .slots
        .iter()
        .find(|slot| slot.label == window.label())
        .map(|slot| slot.payload.clone());
    payload
}

/// 页面量好高度后报上来：改窗口尺寸，再把这一列重排一遍。
#[tauri::command]
pub fn side_resize(height: f64, window: WebviewWindow, state: tauri::State<'_, SideState>) {
    let height = height.clamp(SIDE_MIN_HEIGHT, SIDE_MAX_HEIGHT);
    let scale = window.scale_factor().unwrap_or(1.0);
    {
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        match panel
            .slots
            .iter_mut()
            .find(|slot| slot.label == window.label())
        {
            Some(slot) => slot.height = (height * scale).round() as u32,
            // 已经收起来了就别为一条不在栈上的窗口改尺寸。
            None => return,
        }
    }
    let app = window.app_handle().clone();
    restack(&app);
}

/// 卡片右上角的 ✕。
#[tauri::command]
pub fn side_close(window: WebviewWindow) {
    let app = window.app_handle().clone();
    dismiss_side(&app, window.label());
}

/// 收到私聊消息时按侧通知形式弹出来（场景矩阵里「侧通知」那一格走的就是这个入口）。
///
/// 侧通知**不给**候选回复：它是一条从屏幕边上滑过的轻提醒，真回复请用强提醒窗
/// （notice_reply）。这里只显示文字，所以不需要 ReplyTarget。
pub fn show_side(text: &str, sender: &str, history_id: Option<u64>) -> bool {
    let text = text.trim();
    if text.is_empty() {
        return false;
    }
    let Some(app) = APP.get().cloned() else {
        eprintln!("提醒窗口还没初始化，侧通知发不出去");
        return false;
    };

    // 锁里只记账：窗口操作（含 restack）都会回来抢这把锁。
    // 同一条聊天记录已经挂在右上角时 push 返回 None：不弹第二张卡。
    let Some((label, generation, evicted)) = ({
        let state = app.state::<SideState>();
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        panel.push(text.to_string(), sender.trim().to_string(), history_id)
    }) else {
        return false;
    };

    if let Some(evicted) = evicted {
        if let Some(window) = app.get_webview_window(&evicted) {
            let _ = window.hide();
        }
    }

    if let Err(error) = ensure_side_window(&app, &label) {
        eprintln!("创建侧通知窗口 {label} 失败：{error}");
        dismiss_side(&app, &label);
        return false;
    }

    if let Some(window) = app.get_webview_window(&label) {
        let payload = {
            let state = app.state::<SideState>();
            let panel = state.inner.lock().expect("侧通知状态锁被 poison");
            panel
                .slots
                .iter()
                .find(|slot| slot.label == label)
                .map(|slot| slot.payload.clone())
        };
        if let Some(payload) = payload {
            // 必须定向 emit：emit 是广播，三条侧通知窗口都会收到同一条 payload，
            // 而前端按 payload.id 递增判断「是不是新的」——晚到的新消息会把
            // 已经在显示的那几条全改成同一条文字（用户报的就是这个）。
            if let Err(error) = app.emit_to(label.as_str(), SIDE_EVENT, payload) {
                eprintln!("把侧通知推给窗口失败：{error}");
            }
        }
        restack(&app);
        // 不等 on_page_load：透明无边框窗口在页面画出第一帧之前本来就是全透明的，
        // 提前显示不会闪白；而隐藏窗口上的 on_page_load 有可能永远不来。
        slide_in(&window);
    }

    schedule_dismiss(app, label, generation);
    true
}

/// 配置页「预览侧通知」按钮：拿一条样品走一遍侧通知，不用真的收到私聊。
#[tauri::command]
pub async fn notice_preview_side() {
    show_side(
        &format!(
            "这是一条侧通知预览：点右上角的 ✕，或者等 {} 秒它自己收起来。",
            side_duration_ms() / 1000
        ),
        "预览 · 侧通知",
        None,
    );
}

/// 建侧通知窗口；已经有了就直接复用。
fn ensure_side_window(app: &AppHandle, label: &str) -> tauri::Result<()> {
    if app.get_webview_window(label).is_some() {
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(app, label, WebviewUrl::App(SIDE_PAGE.into()))
        .data_directory(crate::utils::webview_data_dir(app))
        .title("新消息")
        .inner_size(SIDE_WIDTH, SIDE_FALLBACK_HEIGHT as f64)
        // 无边框 + 透明 + 关系统阴影：圆角卡片是页面自己画的，
        // DWM 那个方角阴影会露在圆角外面。
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        // 侧通知的意义就是不打断手头的事：不抢焦点（强提醒窗才 set_focus）。
        .always_on_top(true)
        .focused(false)
        .visible(false)
        .build()?;

    // 关闭 = 收起（隐藏 + 从栈里摘掉），窗口留着复用。
    let window_to_dismiss = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let app = window_to_dismiss.app_handle().clone();
            dismiss_side(&app, window_to_dismiss.label());
        }
    });

    Ok(())
}

/// 把整列侧通知贴着屏幕右上角从上往下排。
fn restack(app: &AppHandle) {
    // 先把要用的数字取出来：窗口操作会绕回来触发 side_resize，那也要抢这把锁。
    let slots: Vec<(String, u32)> = {
        let state = app.state::<SideState>();
        let panel = state.inner.lock().expect("侧通知状态锁被 poison");
        panel
            .slots
            .iter()
            .map(|slot| (slot.label.clone(), slot.height))
            .collect()
    };
    let Some((first, _)) = slots.first() else {
        return;
    };
    let Some(window) = app.get_webview_window(first) else {
        return;
    };
    let scale = window.scale_factor().unwrap_or(1.0);
    let monitor = match window.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        // 查不到显示器就保持原位置：总比把窗口挪到 (0, 0) 强。
        Ok(None) => return,
        Err(error) => {
            eprintln!("查询显示器失败，侧通知保持原位置：{error}");
            return;
        }
    };

    let screen = monitor.size();
    let origin = monitor.position();
    let width = (SIDE_WIDTH * scale).round() as u32;
    let x = (origin.x as f64 + screen.width as f64 - SIDE_MARGIN * scale).round() - width as f64;
    let mut y = origin.y as f64 + SIDE_MARGIN * scale;

    for (label, height) in slots {
        if let Some(window) = app.get_webview_window(&label) {
            if let Err(error) = window.set_size(PhysicalSize::new(width, height)) {
                eprintln!("设置侧通知窗口尺寸失败：{error}");
            }
            if let Err(error) = window
                .set_position(PhysicalPosition::new(x.round() as i32, y.round() as i32))
            {
                eprintln!("设置侧通知窗口位置失败：{error}");
            }
        }
        y += height as f64 + SIDE_GAP * scale;
    }
}

/// 滑入的步数和每步间隔：16 步 × 12ms，约 190ms 走完，和系统通知的手感差不多。
/// 步数比早先多一点：之前匀速 12 步看着像被生硬地「推」进去。
const SIDE_SLIDE_STEPS: u32 = 16;
const SIDE_SLIDE_STEP_MS: u64 = 12;

/// 把一条侧通知从屏幕右沿推进来，到位后就地显示（像系统通知那样滑出来）。
///
/// 动画动的是**窗口**的 x，不是页面里的元素：窗口矩形以外的东西会被裁掉，
/// 卡片自己 translate 到窗口外面就什么都看不见了。收起仍然是直接隐藏，不做退场。
fn slide_in(window: &tauri::WebviewWindow) {
    // restack 已经把它摆到终点了，这里只需要读出来当落点。
    let Ok(target) = window.outer_position() else {
        let _ = window.show();
        return;
    };
    let Some(start_x) = screen_right(window) else {
        let _ = window.show();
        return;
    };
    // 起点比终点还靠左说明这窗口不在右沿上（显示器换了、被用户拖过），别硬滑。
    if start_x <= target.x {
        let _ = window.show();
        return;
    }

    // 先挪到屏幕外再显示：倒过来会让人先看见它在终点闪一帧。
    let _ = window.set_position(PhysicalPosition::new(start_x, target.y));
    let _ = window.show();

    let window = window.clone();
    std::thread::spawn(move || {
        let distance = target.x as f64 - start_x as f64;
        for step in 1..=SIDE_SLIDE_STEPS {
            // ease-out cubic：起步快、收尾慢，滑到位时不再「哐」一下停住。
            let ratio = step as f64 / SIDE_SLIDE_STEPS as f64;
            let eased = 1.0 - (1.0 - ratio).powi(3);
            let x = start_x + (distance * eased).round() as i32;
            // y 每步都重新读：滑动途中又有新通知进来，restack 改过的 y 不能被这里覆盖回去。
            let Ok(now) = window.outer_position() else {
                return;
            };
            if window
                .set_position(PhysicalPosition::new(x, now.y))
                .is_err()
            {
                return;
            }
            if step < SIDE_SLIDE_STEPS {
                std::thread::sleep(Duration::from_millis(SIDE_SLIDE_STEP_MS));
            }
        }
    });
}

/// 主显示器右沿的物理 x（窗口从这儿外滑进来）。
fn screen_right(window: &tauri::WebviewWindow) -> Option<i32> {
    let monitor = match window.primary_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => return None,
        Err(error) => {
            eprintln!("查询显示器失败，侧通知直接出现不滑入：{error}");
            return None;
        }
    };
    Some(monitor.position().x + monitor.size().width as i32)
}

/// 收起一条侧通知：从栈里摘掉、隐藏窗口，再把下面的补上来。
fn dismiss_side(app: &AppHandle, label: &str) {
    {
        let state = app.state::<SideState>();
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        panel.slots.retain(|slot| slot.label != label);
        // 代次 +1：还在睡的自动收起线程醒来会发现对不上，不会再动这条窗口。
        *panel.generations.entry(label.to_string()).or_insert(0) += 1;
    }
    if let Some(window) = app.get_webview_window(label) {
        let _ = window.hide();
    }
    restack(app);
}

/// 到点自动收起。只是让一个线程睡一觉，不值得为它引一个异步运行时。
fn schedule_dismiss(app: AppHandle, label: String, generation: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(side_duration_ms()));
        let still_showing = {
            let state = app.state::<SideState>();
            let panel = state.inner.lock().expect("侧通知状态锁被 poison");
            panel.generations.get(&label).copied().unwrap_or(0) == generation
                && panel.slots.iter().any(|slot| slot.label == label)
        };
        if still_showing {
            dismiss_side(&app, &label);
        }
    });
}
