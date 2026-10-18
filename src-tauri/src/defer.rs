//! 「延后至下课」的滞留队列。
//!
//! 只有内存队列：延后的语义是「等这节课结束」，重启后按 `Config::default_period` 重新判断，
//! 不做持久化——磁盘上留一份就要考虑去重、过期和清理，收益不抵复杂度。
//!
//! 「延后」不会永久滞留：`release_ready` 每 2 秒被 schedule.rs 的轮询线程叫一次，
//! 只要当前场景不再是「延后至下课」（下课了，或者窗口状态换了格子），攒着的消息立刻提醒。

use crate::config::{CONFIG_PATH, Config, SceneAction};
use crate::notice::{self, ReplyTarget};
use std::sync::Mutex;

/// 队列上限。到顶后新的延后消息不再入队（打日志），防止连不上数据源时无限堆积。
const MAX_PENDING: usize = 100;

/// 一条被延后的消息：原样留着提醒所需的一切，放行时和刚到一样弹。
pub struct Pending {
    pub text: String,
    pub sender: String,
    pub replies: Vec<String>,
    pub target: ReplyTarget,
    /// 对应的聊天记录 id：放行时继续带着，重放去重才不会重复弹。
    pub history_id: Option<u64>,
}

static QUEUE: Mutex<Vec<Pending>> = Mutex::new(Vec::new());

/// 攒一条消息，等离开「延后至下课」那一格再提醒。
pub fn push(
    text: String,
    sender: String,
    replies: Vec<String>,
    target: ReplyTarget,
    history_id: Option<u64>,
) {
    let mut queue = QUEUE.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
    if queue.len() >= MAX_PENDING {
        eprintln!("延后队列已满（{MAX_PENDING} 条），这条消息不再延后：{text}");
        return;
    }
    queue.push(Pending {
        text,
        sender,
        replies,
        target,
        history_id,
    });
}

/// 取出全部滞留消息清空队列——调用方已经决定好要用哪种方式提醒了。
pub fn take_all() -> Vec<Pending> {
    std::mem::take(&mut *QUEUE.lock().unwrap_or_else(|poisoned| poisoned.into_inner()))
}

/// 队列里还有没有东西（给测试和日志用，避免没事就去读一遍 config.json）。
pub fn is_empty() -> bool {
    QUEUE
        .lock()
        .map(|queue| queue.is_empty())
        .unwrap_or(true)
}

/// 每轮状态刷新后调用：当前场景不再是「延后至下课」，就把攒着的消息放出来。
pub fn release_ready() {
    if is_empty() {
        return;
    }

    let config = Config::load_or_create(CONFIG_PATH);
    let (period, state, action) = crate::schedule::current_scene(&config);
    if !should_release(action) {
        return;
    }

    eprintln!(
        "延后队列放行：{} / {} → {}",
        period.label(),
        state.label(),
        action.label()
    );
    release_now(action, &announce);
}

/// 该不该现在放行：只有「还是延后至下课」那一格继续攒着，其余（强提醒/侧通知）都放。
///
/// 单独抽出来是为了能测——`release_ready` 里读配置、算当前场景、真弹窗那几步都要跑到真窗口。
fn should_release(action: SceneAction) -> bool {
    action != SceneAction::Postpone
}

/// 按当前动作把队列里的消息全提醒一遍，返回发了几条（先到的先提醒）。
///
/// `announce` 是「怎么提醒一条」，做成参数是为了让单测塞个假的进来：
/// 「延后不放行」「顺序」「发完必须清空」这三条关键性质不用真窗口就能验。
fn release_now(action: SceneAction, announce: &dyn Fn(Pending, SceneAction)) -> usize {
    // 门就放在这里——调用方漏判也不能把「延后」那一格的消息提前弹出来
    // （单测 postpone_holds_every_message 盯的就是这条）。
    if !should_release(action) {
        return 0;
    }
    let items = take_all();
    let count = items.len();
    for item in items {
        announce(item, action);
    }
    count
}

/// 按当前动作提醒一条：侧通知只显示文字，其余走强提醒（带候选回复）。
fn announce(item: Pending, action: SceneAction) {
    // 两条路径都返回「发出去没有」，这里不看：发不出去（窗口还没起来）时已经各自打了日志。
    let _ = match action {
        SceneAction::Side => notice::show_side(&item.text, &item.sender, item.history_id),
        // 上课时延后、下课时发现那一格改成了「忽略」：这条就不弹了（聊天记录里还在）。
        SceneAction::Ignore => false,
        _ => notice::show(
            &item.text,
            &item.sender,
            item.replies,
            item.target,
            item.history_id,
        ),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 这些测试共用同一个静态队列，而 cargo test 默认多线程：串行化，
    /// 否则一个测试的 take_all 会把另一个刚 push 的条目吃掉。
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    fn lock() -> std::sync::MutexGuard<'static, ()> {
        TEST_LOCK
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    fn item(text: &str) -> Pending {
        Pending {
            text: text.to_string(),
            sender: "测试昵称".to_string(),
            replies: vec!["收到".to_string()],
            target: ReplyTarget {
                sender_id: 3791513967,
                message_id: 9001,
            },
            history_id: Some(1),
        }
    }

    fn enqueue(pending: Pending) {
        push(
            pending.text,
            pending.sender,
            pending.replies,
            pending.target,
            pending.history_id,
        );
    }

    #[test]
    fn queue_drains_completely() {
        // 单测之间共享静态队列，先清空，别让上一条测试的顺序影响结果。
        let _guard = lock();
        let _ = take_all();
        assert!(is_empty());
        for pending in [item("一"), item("二")] {
            enqueue(pending);
        }
        assert!(!is_empty());
        let drained = take_all();
        assert_eq!(drained.len(), 2);
        assert_eq!(drained[0].text, "一");
        assert_eq!(drained[1].target.message_id, 9001);
        assert!(is_empty(), "take_all 必须清空，否则消息会重复提醒");
    }

    #[test]
    fn queue_caps_at_max_pending() {
        let _guard = lock();
        let _ = take_all();
        for index in 0..MAX_PENDING + 5 {
            let mut pending = item("满了");
            pending.text = format!("第 {index} 条");
            enqueue(pending);
        }
        let drained = take_all();
        assert_eq!(drained.len(), MAX_PENDING);
        // 保留的是先到的那批，超出的被丢掉并打了日志。
        assert_eq!(drained[0].text, "第 0 条");
        assert_eq!(drained[MAX_PENDING - 1].text, format!("第 {} 条", MAX_PENDING - 1));
    }

    /// 还在「延后至下课」那一格：一条都不许弹，队列也得原样留着。
    #[test]
    fn postpone_holds_every_message() {
        let _guard = lock();
        let _ = take_all();
        enqueue(item("一"));
        enqueue(item("二"));

        let sent = release_now(SceneAction::Postpone, &|_, _| panic!("还在上课就不该提醒"));
        assert_eq!(sent, 0);
        assert!(!is_empty(), "被拦下的消息必须留在队列里，出了那一格才弹");
        assert_eq!(take_all().len(), 2);
    }

    /// 离开那一格：按到达顺序全发一遍，而且发完必须清空——
    /// release_ready 每 2 秒被叫一次，不清空就会每 2 秒重复提醒同一批消息。
    #[test]
    fn release_sends_in_order_and_only_once() {
        let _guard = lock();
        let _ = take_all();
        for text in ["一", "二", "三"] {
            enqueue(item(text));
        }

        let seen = Mutex::new(Vec::new());
        let sent = release_now(SceneAction::Strong, &|pending, action| {
            assert!(matches!(action, SceneAction::Strong));
            seen.lock().unwrap().push(pending.text);
        });
        assert_eq!(sent, 3);
        assert_eq!(*seen.lock().unwrap(), ["一", "二", "三"]);
        assert!(is_empty(), "放行后必须清空");

        // 2 秒后的下一轮：队列已经空了，什么都不该再发。
        assert_eq!(release_now(SceneAction::Strong, &|_, _| panic!("队列已空")), 0);
    }

    /// 放行时要原样带上回复目标和聊天记录 id：前者决定强提醒能不能直接回复，
    /// 后者决定重放同一条历史消息时不再重复弹。
    #[test]
    fn release_keeps_reply_target_and_history_id() {
        let _guard = lock();
        let _ = take_all();
        enqueue(item("带目标的一条"));

        let sent = release_now(SceneAction::Side, &|pending, action| {
            assert!(matches!(action, SceneAction::Side));
            assert_eq!(pending.target.sender_id, 3791513967);
            assert_eq!(pending.target.message_id, 9001);
            assert_eq!(pending.history_id, Some(1));
        });
        assert_eq!(sent, 1);
        assert!(is_empty());
    }
}
