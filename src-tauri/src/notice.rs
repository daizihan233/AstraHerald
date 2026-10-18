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

/// 侧通知专用的负载：强提醒那套字段原样带上，另加两个只有侧通知用得上的显示参数。
///
/// 单独一个结构是为了不往 NoticePayload 里塞几个强提醒窗永远用不到的字段；
/// serde 的 flatten 让前端拿到的 JSON 仍然是平的（payload.text / payload.font_px）。
#[derive(Debug, Clone, Serialize)]
pub struct SidePayload {
    #[serde(flatten)]
    pub notice: NoticePayload,
    /// 正文字号（逻辑像素）。页面用它设 --side-font，卡片里的尺寸都按 em 跟着走。
    pub font_px: u32,
    /// 昵称字号（逻辑像素）。单独一份：昵称那行不跟着正文等比缩放，各自调各自的。
    pub nick_font_px: u32,
    /// 自动收起时长（毫秒）。底部那条进度线按它跑，和 schedule_dismiss 是同一个值。
    pub duration_ms: u64,
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

/// 侧通知窗口 label 的前缀：notice-side-1、notice-side-2…… 每个 label 一个常驻窗口，
/// 来回复用，省掉「收到消息才建 WebView」那几百毫秒的空白。
///
/// 池子多大是配置项（config.rs 的 SideSettings::max_count，1..=10）：窗口按需建、按需复用；
/// capability 那边用 notice-side-* 一次覆盖整套（capabilities/notice.json）。
const SIDE_LABEL_PREFIX: &str = "notice-side-";

/// 第 index 个（从 0 数）侧通知窗口的 label。
fn side_label(index: usize) -> String {
    format!("{SIDE_LABEL_PREFIX}{}", index + 1)
}

/// 侧通知加载的页面，同样由 vite 的多页构建产出。
const SIDE_PAGE: &str = "side.html";

/// 窗口已经建好的时候，用这个事件把新内容推进去。
const SIDE_EVENT: &str = "side:new";

/// 收起一条的时候先发这个：页面收到就播退场动画，播完（SIDE_EXIT_MS 之后）才隐藏窗口。
/// 直接 hide 是「啪」地一下消失，和进场动画完全不对称。前端 SideNotification.vue 听这个名字。
const SIDE_EXIT_EVENT: &str = "side:exit";

/// 退场动画的时长（毫秒）：动画播完才真正隐藏窗口。前端 EXIT_MS 是同一个数。
const SIDE_EXIT_MS: u64 = 200;

/// 重排时位置滑动的步数与每步间隔：10 × 16ms ≈ 160ms，带缓出。
const SIDE_MOVE_STEPS: u32 = 10;
const SIDE_MOVE_STEP_MS: u64 = 16;

/// 位置滑动的代次：新的一次重排让上一次没走完的动画立刻收手（否则两个线程互相拽同一个窗口）。
static RESTACK_GEN: AtomicU64 = AtomicU64::new(0);

/// 卡片宽度、距屏幕边沿和条与条之间的留白（逻辑像素）。
/// 宽度跟着字号一起放大过：360 太窄，17px 的正文一行放不下几个字。
const SIDE_WIDTH: f64 = 400.0;
const SIDE_MARGIN: f64 = 16.0;
const SIDE_GAP: f64 = 12.0;

/// 页面量出真实高度之前先按这个高度摆（逻辑像素）。
const SIDE_FALLBACK_HEIGHT: u32 = 120;

/// 页面报高度最迟等到这个时间。到点还没报就按估算高度先显示出来——页面万一出问题，
/// 「永远不显示」比「先裁一刀再长高」更糟。
const SIDE_SHOW_DEADLINE_MS: u64 = 700;

/// 页面报上来的高度会被夹在这个区间：太矮文字挤成一团，太高就不像通知了。
const SIDE_MIN_HEIGHT: f64 = 64.0;
const SIDE_MAX_HEIGHT: f64 = 420.0;

/// 侧通知的显示参数（配置页可改，见 config.rs 的 SideSettings）。
///
/// 一次读齐：时长/字号随负载下发给页面，页面不用再自己读一次配置；条数只有后端用。
#[derive(Clone, Copy)]
struct SideOptions {
    /// 自动收起的毫秒数。
    duration_ms: u64,
    /// 正文字号（逻辑像素）。
    font_px: u32,
    /// 昵称字号（逻辑像素）。
    nick_font_px: u32,
    /// 最多同时在屏幕上挂几条。
    max: usize,
}

impl SideOptions {
    fn load() -> Self {
        let side = crate::config::Config::load_or_create(crate::config::CONFIG_PATH).side;
        Self {
            duration_ms: side.clamped_secs() * 1000,
            font_px: side.clamped_font_px(),
            nick_font_px: side.clamped_nick_font_px(),
            max: side.clamped_count(),
        }
    }
}

/// 一条正在显示的侧通知。
struct SideSlot {
    label: String,
    /// 当前高度（物理像素），页面量好后通过 side_resize 报上来。0 = 还没量到。
    height: u32,
    /// 这一代的卡片显示出来了没有。没量到高度之前窗口一直藏着：先按估算高度画出来、
    /// 过二百来毫秒再长高一次，看着就是卡片被裁一刀又弹开。
    shown: bool,
    payload: SidePayload,
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
    /// 从上到下的显示顺序：第一个贴着屏幕右上角（最早的一条），新来的排在队尾。
    ///
    /// 排到队尾而不是插到队首：插队首会把已经在显示的几张卡整体往下推，每张都要
    /// 重新定位 + 重画一遍——几条侧通知一起到的时候，那一下就是肉眼可见的一卡一卡。
    slots: Vec<SideSlot>,
    /// 每个 label「第几次显示」。自动收起的线程带着创建时的值回来，对不上就说明这条窗口
    /// 已经被复用成新消息了，不能再关。
    generations: HashMap<String, u64>,
    seq: u64,
}

impl SidePanel {
    /// 把一条新内容放到队尾，返回要用的窗口 label 和这一代的编号。
    fn push(
        &mut self,
        text: String,
        sender: String,
        history_id: Option<u64>,
        font_px: u32,
        nick_font_px: u32,
        duration_ms: u64,
        max: usize,
    ) -> Option<(String, u64, Vec<String>)> {
        // 已经在右上角挂着的同一条聊天记录：不再弹第二张卡。
        if let Some(id) = history_id {
            if self.slots.iter().any(|slot| slot.history_id == Some(id)) {
                return None;
            }
        }
        self.seq += 1;
        // replies 恒空：侧通知不给候选回复（真回复请走强提醒窗）。这里留着这个字段是因为
        // 侧通知和强提醒共用 NoticePayload，拆开得不偿失。
        let payload = SidePayload {
            notice: NoticePayload {
                id: self.seq,
                text,
                sender,
                replies: Vec::new(),
            },
            font_px,
            nick_font_px,
            duration_ms,
        };

        // 先找一个空着的窗口 label（池子就是第 1..=max 个）。
        let max = max.max(1);
        let mut label = String::new();
        for index in 0..max {
            let candidate = side_label(index);
            if !self.slots.iter().any(|slot| slot.label == candidate) {
                label = candidate;
                break;
            }
        }

        // 池子满了：把最上面（最早）那条换掉，窗口留着复用、挪到队尾显示新内容。
        //
        // 复用要「原地换」：这个窗口此刻正显示着上一张卡，别把它藏起来再弹一次
        // （藏掉到重新显示之间那段空白，用户看到的就是闪一下）。高度也留着，页面
        // 报新高度之前先按旧高度摆，免得又走一遍估算高度那样一跳。
        let mut carried = (0u32, false);
        if label.is_empty() {
            let oldest = self.slots.remove(0);
            label = oldest.label;
            carried = (oldest.height, oldest.shown);
        }

        let generation = {
            let generation = self.generations.entry(label.clone()).or_insert(0);
            *generation += 1;
            *generation
        };
        self.slots.push(SideSlot {
            label: label.clone(),
            height: carried.0,
            shown: carried.1,
            payload,
            history_id,
        });

        // 上限被配置改小之后，多出来的旧卡（队首那几张）现在就退役，屏幕上只留最新的
        // max 条。刚推的这张在队尾，不会被收掉。
        let mut dropped = Vec::new();
        while self.slots.len() > max {
            dropped.push(self.slots.remove(0).label);
        }
        Some((label, generation, dropped))
    }
}

/// 侧通知窗口挂载时补拉一次。理由和 notice_current 一样：emit 可能早于页面注册监听。
#[tauri::command]
pub fn side_current(
    window: WebviewWindow,
    state: tauri::State<'_, SideState>,
) -> Option<SidePayload> {
    let panel = state.inner.lock().expect("侧通知状态锁被 poison");
    let payload = panel
        .slots
        .iter()
        .find(|slot| slot.label == window.label())
        .map(|slot| slot.payload.clone());
    payload
}

/// 页面量好高度后报上来：只有高度真的变了才改窗口尺寸、重排这一列。
#[tauri::command]
pub fn side_resize(height: f64, window: WebviewWindow, state: tauri::State<'_, SideState>) {
    let height = height.clamp(SIDE_MIN_HEIGHT, SIDE_MAX_HEIGHT);
    let scale = window.scale_factor().unwrap_or(1.0);
    let physical = (height * scale).round() as u32;
    let first_report = {
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        match panel
            .slots
            .iter_mut()
            .find(|slot| slot.label == window.label())
        {
            // 已经显示出来的卡片，高度没变就别重排：页面每次挂载都会喊一声，白重排一次
            // 就是三个窗口各重画一遍，这是多条侧通知一起到的时候卡顿的另一半原因。
            Some(slot) if slot.shown && slot.height == physical => return,
            Some(slot) => {
                let first = !slot.shown;
                slot.height = physical;
                slot.shown = true;
                first
            }
            // 已经收起来了就别为一条不在栈上的窗口改尺寸。
            None => return,
        }
    };
    let app = window.app_handle().clone();
    restack(&app);
    // 第一份高度就是「卡片画完了」：尺寸已经摆好，这时候显示出来，用户看到的第一帧
    // 就是最终样子。
    if first_report {
        let _ = window.show();
    }
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
    show_side_configured(text, sender, history_id, &SideOptions::load())
}

/// 显示参数已经定下来的那一层：真消息用 `side_settings()` 读出来的值，
/// 配置页的预览把输入框里（可能还没保存）的值直接传进来。
fn show_side_configured(
    text: &str,
    sender: &str,
    history_id: Option<u64>,
    options: &SideOptions,
) -> bool {
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
    let Some((label, generation, dropped)) = ({
        let state = app.state::<SideState>();
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        panel.push(
            text.to_string(),
            sender.trim().to_string(),
            history_id,
            options.font_px,
            options.nick_font_px,
            options.duration_ms,
            options.max,
        )
    }) else {
        return false;
    };

    // 上限改小之后被挤掉的那几张：窗口收起来（留着复用），下面的补上来。
    for stale in dropped {
        dismiss_side(&app, &stale);
    }

    if let Err(error) = ensure_side_window(&app, &label) {
        eprintln!("创建侧通知窗口 {label} 失败：{error}");
        dismiss_side(&app, &label);
        return false;
    }

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
    // 这里**不**显示窗口：等页面把内容高度量出来报回来（side_resize），那一份上报
    // 才是「卡片画完了」，这时 set_size + show 出来的第一帧就是最终样子。
    //
    // 窗口直接摆在终点，滑入动画全部交给页面里的 CSS（SideNotification.vue）：
    // 逐帧挪原生窗口（每 12ms 一次 set_position）在低配机上是一顿一顿的，而窗口
    // 本身透明，摆在哪一帧用户都看不见；入场动画的起点本来就在窗口右沿之外（被裁掉），
    // 所以晚显示这几十毫秒看不出差别。

    schedule_dismiss(app.clone(), label.clone(), generation, options.duration_ms);
    schedule_fallback_show(app, label, generation);
    true
}

/// 配置页「预览侧通知」按钮：拿一条样品走一遍侧通知，不用真的收到私聊。
///
/// 配置页把输入框里的字号和时长一起带过来：预览的就是「保存后马上会生效」的样子，
/// 不必先保存再看效果。
#[tauri::command]
pub async fn notice_preview_side(
    font: Option<u32>,
    nick_font: Option<u32>,
    secs: Option<u64>,
) {
    let defaults = SideOptions::load();
    let options = SideOptions {
        duration_ms: secs
            .map_or(defaults.duration_ms, |s| {
                crate::config::SideSettings::clamp_secs(s) * 1000
            }),
        font_px: font.map_or(defaults.font_px, crate::config::SideSettings::clamp_font),
        nick_font_px: nick_font.map_or(defaults.nick_font_px, crate::config::SideSettings::clamp_font),
        max: defaults.max,
    };
    show_side_configured(
        &format!(
            "这是一条侧通知预览：点右上角的 ✕，或者等 {} 秒它自己收起来。",
            options.duration_ms / 1000
        ),
        "预览 · 侧通知",
        None,
        &options,
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
///
/// 位置是「滑过去」的：一条收起之后，下面的卡片带缓出地挪上来，不是瞬间跳到新位置。
/// 尺寸没法插值（页面已经按新高度画好了），所以 set_size 立即生效，只有位置走动画。
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
    // 这一趟要滑的窗口：(label, 起点 x/y, 终点 x/y)，都是物理像素。
    let mut moves: Vec<(String, i32, i32, i32, i32)> = Vec::new();

    for (label, height) in slots {
        if let Some(window) = app.get_webview_window(&label) {
            // 尺寸/位置没变就别调 Win32：set_size 会让 WebView2 重新布局 + 重画，
            // set_position 要重新合成。几条通知一起到的时候，「没变也调一遍」
            // 就是主要的开销。
            // height == 0：页面还没报高度，先按估算高度摆。
            let height = if height == 0 {
                (SIDE_FALLBACK_HEIGHT as f64 * scale).round() as u32
            } else {
                height
            };
            let size = PhysicalSize::new(width, height);
            if window.inner_size().ok() != Some(size) {
                if let Err(error) = window.set_size(size) {
                    eprintln!("设置侧通知窗口尺寸失败：{error}");
                }
            }
            let position = PhysicalPosition::new(x.round() as i32, y.round() as i32);
            let current = window.outer_position().ok();
            if current != Some(position) {
                match current {
                    // 已经在屏幕上显示的：滑过去（下面统一走动画）。
                    Some(from) if window.is_visible().unwrap_or(false) => {
                        moves.push((label, from.x, from.y, position.x, position.y));
                    }
                    // 还没显示出来的（这一轮刚建好、还没 show）直接摆到位：它本来就是隐藏的，
                    // 让它从系统给的默认位置滑过来只会看到一张卡飞进来。
                    _ => {
                        if let Err(error) = window.set_position(position) {
                            eprintln!("设置侧通知窗口位置失败：{error}");
                        }
                    }
                }
            }
        }
        y += height as f64 + SIDE_GAP * scale;
    }

    if moves.is_empty() {
        return;
    }
    // 新的一次重排作废上一次没走完的动画。
    let generation = RESTACK_GEN.fetch_add(1, Ordering::SeqCst) + 1;
    let app = app.clone();
    std::thread::spawn(move || {
        for step in 1..=SIDE_MOVE_STEPS {
            if RESTACK_GEN.load(Ordering::SeqCst) != generation {
                return;
            }
            let t = step as f64 / SIDE_MOVE_STEPS as f64;
            // 缓出：起步快、收尾慢，像是被推上去的。
            let ease = 1.0 - (1.0 - t).powi(3);
            for (label, from_x, from_y, to_x, to_y) in &moves {
                if let Some(window) = app.get_webview_window(label) {
                    let x = (*from_x as f64 + (*to_x - *from_x) as f64 * ease).round() as i32;
                    let y = (*from_y as f64 + (*to_y - *from_y) as f64 * ease).round() as i32;
                    let _ = window.set_position(PhysicalPosition::new(x, y));
                }
            }
            std::thread::sleep(Duration::from_millis(SIDE_MOVE_STEP_MS));
        }
    });
}


/// 收起一条侧通知：从栈里摘掉、让页面播完退场动画、再隐藏窗口。
///
/// 记账立刻做（下面的卡片马上开始往上补），只有 hide 推迟到退场动画之后：
/// 直接 hide 的话卡片是「啪」地一下消失，看起来生硬。
fn dismiss_side(app: &AppHandle, label: &str) {
    let generation = {
        let state = app.state::<SideState>();
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        panel.slots.retain(|slot| slot.label != label);
        // 代次 +1：还在睡的自动收起线程醒来会发现对不上，不会再动这条窗口。
        let generation = panel.generations.entry(label.to_string()).or_insert(0);
        *generation += 1;
        *generation
    };
    // 发不出去也不影响：下面照样按时隐藏，只是少了退场动画。
    if let Err(error) = app.emit_to(label, SIDE_EXIT_EVENT, ()) {
        eprintln!("侧通知退场事件发不出去：{error}");
    }
    restack(app);
    // 动画这段时间里这条窗口可能已经被下一条消息复用（代次变了），那就交给新消息显示，
    // 这里不许再把它藏起来。
    let app = app.clone();
    let label = label.to_string();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SIDE_EXIT_MS));
        let reused = {
            let state = app.state::<SideState>();
            let panel = state.inner.lock().expect("侧通知状态锁被 poison");
            panel.generations.get(&label).copied().unwrap_or(0) != generation
        };
        if !reused {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.hide();
            }
        }
    });
}

/// 配置页把「最多几条」改小之后，把多出来的那几张立刻收掉。
///
/// 只在收到下一条消息时才收敛的话，用户刚把 3 改成 1，屏幕上那两条还会一直挂到自动收起。
pub fn apply_side_limit() {
    let Some(app) = APP.get().cloned() else {
        return;
    };
    let max = SideOptions::load().max;
    let stale: Vec<String> = {
        let state = app.state::<SideState>();
        let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
        let mut stale = Vec::new();
        // 收队首（最早那几条），留着最新的 —— 和 push 挤掉时的取舍一致。
        while panel.slots.len() > max {
            stale.push(panel.slots.remove(0).label);
        }
        stale
    };
    for label in stale {
        dismiss_side(&app, &label);
    }
}

/// 页面没在 SIDE_SHOW_DEADLINE_MS 内报高度就兜底显示出来（否则这条通知永远藏着）。
fn schedule_fallback_show(app: AppHandle, label: String, generation: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(SIDE_SHOW_DEADLINE_MS));
        let show = {
            let state = app.state::<SideState>();
            let mut panel = state.inner.lock().expect("侧通知状态锁被 poison");
            // 代次对不上说明这条窗口已经被复用成下一条了；shown 已经是 true 说明页面
            // 早在时限内报过高度了。
            let same_generation = panel.generations.get(&label).copied().unwrap_or(0) == generation;
            match panel.slots.iter_mut().find(|slot| slot.label == label) {
                Some(slot) if same_generation && !slot.shown => {
                    slot.shown = true;
                    true
                }
                _ => false,
            }
        };
        if show {
            if let Some(window) = app.get_webview_window(&label) {
                let _ = window.show();
            }
        }
    });
}

/// 到点自动收起。只是让一个线程睡一觉，不值得为它引一个异步运行时。
fn schedule_dismiss(app: AppHandle, label: String, generation: u64, duration_ms: u64) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(duration_ms));
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

#[cfg(test)]
mod tests {
    use super::*;

    fn texts(panel: &SidePanel) -> Vec<&str> {
        panel
            .slots
            .iter()
            .map(|slot| slot.payload.notice.text.as_str())
            .collect()
    }

    /// 默认池子 3 条（配置里的默认值）。
    fn push(
        panel: &mut SidePanel,
        text: &str,
        history_id: Option<u64>,
    ) -> Option<(String, u64, Vec<String>)> {
        push_max(panel, text, history_id, 3)
    }

    fn push_max(
        panel: &mut SidePanel,
        text: &str,
        history_id: Option<u64>,
        max: usize,
    ) -> Option<(String, u64, Vec<String>)> {
        panel.push(
            text.to_string(),
            "某人".to_string(),
            history_id,
            17,
            15,
            1000,
            max,
        )
    }

    /// 侧通知从上往下的顺序是「老的在上、新来的在下」，满三条时挤掉最早那条。
    /// 这个顺序是刻意的：新消息插到队首会把已经在显示的卡整体往下推（见 push）。
    #[test]
    fn new_side_notice_joins_the_tail_and_evicts_the_oldest() {
        let mut panel = SidePanel::default();
        for (index, text) in ["一", "二", "三"].iter().enumerate() {
            let (label, generation, _) =
                push(&mut panel, text, Some(index as u64)).expect("前三条应该都进得去");
            assert_eq!(label, side_label(index));
            assert_eq!(generation, 1);
        }
        assert_eq!(texts(&panel), ["一", "二", "三"]);

        // 最早那条已经量到高度、也就是正显示在屏幕上。
        panel.slots[0].height = 208;
        panel.slots[0].shown = true;

        // 第四条：最早的「一」被挤掉，它那个窗口挪到队尾显示新内容。
        let (label, generation, dropped) =
            push(&mut panel, "四", Some(9)).expect("第四条应该挤掉最早那条");
        assert_eq!(label, side_label(0));
        assert!(dropped.is_empty(), "三条满时是原地复用，不用额外收谁");
        assert_eq!(generation, 2);
        assert_eq!(texts(&panel), ["二", "三", "四"]);
        // 高度和「已经显示」的状态跟着窗口一起复用：原地换内容，不会藏一下再弹出来。
        assert_eq!(panel.slots[2].height, 208);
        assert!(panel.slots[2].shown);
    }

    /// 新卡片在页面报高度之前是「没量到、也没显示」的：窗口要等 side_resize 才显示，
    /// 免得先按估算高度画出来、过一会儿再长高一次（看着就是卡片被裁一刀又弹开）。
    #[test]
    fn new_side_notice_waits_for_measured_height_before_showing() {
        let mut panel = SidePanel::default();
        push(&mut panel, "一", None).expect("第一条应该进得去");
        assert_eq!(panel.slots[0].height, 0);
        assert!(!panel.slots[0].shown);
    }

    /// 同一条聊天记录已经挂在右上角时不再弹第二张卡。
    #[test]
    fn same_history_entry_is_not_shown_twice() {
        let mut panel = SidePanel::default();
        assert!(push(&mut panel, "一", Some(7)).is_some());
        assert!(push(&mut panel, "又来", Some(7)).is_none());
    }

    /// 「最多展示几条」= 1 时：每条新消息都用同一个窗口原地换，不是叠上去。
    #[test]
    fn single_slot_reuses_the_only_window() {
        let mut panel = SidePanel::default();
        let (label, _, dropped) = push_max(&mut panel, "一", Some(1), 1).expect("第一条应该进得去");
        assert_eq!(label, side_label(0));
        assert!(dropped.is_empty());

        panel.slots[0].height = 208;
        panel.slots[0].shown = true;

        let (label, generation, dropped) =
            push_max(&mut panel, "二", Some(2), 1).expect("第二条应该换掉第一条");
        assert_eq!(label, side_label(0));
        assert_eq!(generation, 2);
        assert_eq!(texts(&panel), ["二"]);
        assert!(dropped.is_empty(), "只有一条时是原地换内容，不是额外收掉");
        assert_eq!(panel.slots[0].height, 208);
        assert!(panel.slots[0].shown);
    }

    /// 上限从 3 改成 2 之后再来一条：多出来的旧卡收掉，屏幕上只留最新的两条。
    #[test]
    fn lowering_the_limit_retires_the_oldest_cards() {
        let mut panel = SidePanel::default();
        for (index, text) in ["一", "二", "三"].iter().enumerate() {
            push_max(&mut panel, text, Some(index as u64), 3).expect("三条都该进得去");
        }

        let (_, _, dropped) = push_max(&mut panel, "四", Some(9), 2).expect("第四条应该进得去");
        assert_eq!(texts(&panel), ["三", "四"]);
        // 被收掉的是第二条（最早的「一」那个窗口被复用成「四」了）。
        assert_eq!(dropped, vec![side_label(1)]);
    }

    /// 没有聊天记录编号（配置页预览就是这种）时不去重，点几次看几次。
    #[test]
    fn previews_without_history_id_always_show() {
        let mut panel = SidePanel::default();
        assert!(push(&mut panel, "预览一", None).is_some());
        assert!(push(&mut panel, "预览二", None).is_some());
    }
}

