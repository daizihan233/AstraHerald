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
    if action == SceneAction::Postpone {
        return;
    }

    eprintln!(
        "延后队列放行：{} / {} → {}",
        period.label(),
        state.label(),
        action.label()
    );
    for item in take_all() {
        match action {
            SceneAction::Side => notice::show_side(
                &item.text,
                &item.sender,
                item.replies,
                Some(item.target),
                item.history_id,
            ),
            // Postpone 上面已经 return，剩下的按强提醒处理（Strong 是矩阵里唯一剩下的值）。
            _ => notice::show(
                &item.text,
                &item.sender,
                item.replies,
                item.target,
                item.history_id,
            ),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn queue_drains_completely() {
        // 单测之间共享静态队列，先清空，别让上一条测试的顺序影响结果。
        let _ = take_all();
        assert!(is_empty());
        for pending in [item("一"), item("二")] {
            push(
                pending.text,
                pending.sender,
                pending.replies,
                pending.target,
                pending.history_id,
            );
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
        let _ = take_all();
        for index in 0..MAX_PENDING + 5 {
            let mut pending = item("满了");
            pending.text = format!("第 {index} 条");
            push(
                pending.text,
                pending.sender,
                pending.replies,
                pending.target,
                pending.history_id,
            );
        }
        let drained = take_all();
        assert_eq!(drained.len(), MAX_PENDING);
        // 保留的是先到的那批，超出的被丢掉并打了日志。
        assert_eq!(drained[0].text, "第 0 条");
        assert_eq!(drained[MAX_PENDING - 1].text, format!("第 {} 条", MAX_PENDING - 1));
    }
}
