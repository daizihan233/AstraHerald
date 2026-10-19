//! 文本聊天记录：落盘一份，供托盘「聊天记录」窗口翻看与重放。
//!
//! 只记「会被提醒的文字消息」（黑白名单拦掉的、图片文件表情一概不记），也就是
//! 用户在提醒窗里看到过的东西——它存在的意义是「刚才那条弹窗手滑关了，再翻出来看」，
//! 不是做完整的聊天归档。
//!
//! 存储就是 exe 旁边一个 JSONL：一行一条，追加写，超过上限时整体重写一次。
//! 够简单，坏了也只丢坏掉的那一行。

use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

/// 聊天记录文件，和 config.json 一样在 exe 目录（启动时工作目录已钉好）。
pub const HISTORY_PATH: &str = "history.jsonl";

/// 只留最近这么多条。再多用户也翻不过来，还要一直占磁盘。
const MAX_ENTRIES: usize = 500;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Entry {
    /// 单调递增：重放去重和「选中哪几条」都靠它。
    pub id: u64,
    /// unix 秒。
    pub at: u64,
    /// 发送者的 QQ 号（存字符串，重放时再解析回 i64）。
    pub qq: String,
    /// 「备注 [用户名] （QQ号）」，提醒窗顶栏显示的就是它。
    pub name: String,
    pub text: String,
}

/// 内存镜像：None 表示还没读过盘。读盘只做一次。
static ENTRIES: Mutex<Option<Vec<Entry>>> = Mutex::new(None);

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|elapsed| elapsed.as_secs())
        .unwrap_or(0)
}

fn lock() -> std::sync::MutexGuard<'static, Option<Vec<Entry>>> {
    ENTRIES.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// 第一次用的时候把盘上的记录读进来。
fn load(slot: &mut Option<Vec<Entry>>) -> &mut Vec<Entry> {
    if slot.is_none() {
        let mut items = Vec::new();
        match fs::read_to_string(HISTORY_PATH) {
            Ok(text) => {
                for line in text.lines() {
                    if line.trim().is_empty() {
                        continue;
                    }
                    // 坏行（上次写到一半断电之类）只丢这一行，别把整份记录作废。
                    if let Ok(entry) = serde_json::from_str::<Entry>(line) {
                        items.push(entry);
                    }
                }
            }
            // 文件不存在就是第一次运行，正常。
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => eprintln!("读聊天记录失败：{error}"),
        }
        *slot = Some(items);
    }
    slot.as_mut().expect("刚刚填过")
}

fn append_line(entry: &Entry) -> std::io::Result<()> {
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(HISTORY_PATH)?;
    let line = serde_json::to_string(entry)
        .map_err(|error| std::io::Error::new(std::io::ErrorKind::InvalidData, error))?;
    writeln!(file, "{line}")
}

/// 超过上限时整体重写：内存里已经裁好，落盘跟着对齐。
fn rewrite(items: &[Entry]) {
    let file = match fs::File::create(HISTORY_PATH) {
        Ok(file) => file,
        Err(error) => {
            eprintln!("重写聊天记录失败：{error}");
            return;
        }
    };
    let mut out = std::io::BufWriter::new(file);
    for entry in items {
        match serde_json::to_string(entry) {
            Ok(line) => {
                if let Err(error) = writeln!(out, "{line}") {
                    eprintln!("重写聊天记录失败：{error}");
                    return;
                }
            }
            Err(error) => eprintln!("序列化聊天记录失败：{error}"),
        }
    }
}

/// 记一条文字消息，返回它的 id（重放去重要用）。
pub fn record(qq: &str, name: &str, text: &str) -> u64 {
    let mut guard = lock();
    let items = load(&mut guard);
    let id = items.last().map(|entry| entry.id).unwrap_or(0) + 1;
    let entry = Entry {
        id,
        at: now(),
        qq: qq.to_string(),
        name: name.to_string(),
        text: text.to_string(),
    };
    if let Err(error) = append_line(&entry) {
        eprintln!("写聊天记录失败：{error}");
    }
    items.push(entry);
    if items.len() > MAX_ENTRIES {
        let overflow = items.len() - MAX_ENTRIES;
        items.drain(..overflow);
        rewrite(items);
    }
    id
}

/// 最近的在前面：窗口一打开就先看到刚才那条。
pub fn list() -> Vec<Entry> {
    let mut guard = lock();
    let mut items = load(&mut guard).clone();
    items.reverse();
    items
}

/// 按 id 取，保持调用方给的顺序（重放按时间正序），未知 id 跳过。
pub fn pick(ids: &[u64]) -> Vec<Entry> {
    let mut guard = lock();
    let items = load(&mut guard);
    ids.iter()
        .filter_map(|id| items.iter().find(|entry| entry.id == *id).cloned())
        .collect()
}

/// 给「聊天记录」窗口：最近的在前。
#[tauri::command]
pub fn history_list() -> Vec<Entry> {
    list()
}

/// 重放选中的记录：效果和「这些消息刚刚又收到了」一样（走同一套场景矩阵）。
///
/// 已经在提醒窗口里显示着的那条会被去重跳过，所以返回真正又展示了几条。
// 必须是 async：重放一律走强提醒（bot::replay 传 force_strong），提醒窗可能还没
// 建出来、还要置顶抢焦点；同步命令跑在 Tauri 主线程上，在主线程里等窗口操作会把
// 主线程等死（按钮一直转圈、后面几条重放全卡住）。异步命令跑在工作线程上，和真
// 收到消息的路径一样，窗口操作由主线程的事件循环去处理。
#[tauri::command(async)]
pub fn history_replay(ids: Vec<u64>) -> Result<usize, String> {
    let picked = pick(&ids);
    if picked.is_empty() {
        return Err("没有选中任何记录".to_string());
    }
    let mut shown = 0;
    for entry in picked {
        if crate::bot::replay(&entry) {
            shown += 1;
        }
    }
    Ok(shown)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry(id: u64, text: &str) -> Entry {
        Entry {
            id,
            at: id,
            qq: "1".to_string(),
            name: "甲".to_string(),
            text: text.to_string(),
        }
    }

    #[test]
    fn pick_keeps_caller_order_and_skips_unknown() {
        // 不碰真实文件：pick 读的是内存镜像，这里先塞一条进去。
        {
            let mut guard = lock();
            *guard = Some(vec![entry(7, "先"), entry(9, "后")]);
        }
        let picked = pick(&[9, 404, 7]);
        assert_eq!(picked.len(), 2);
        assert_eq!(picked[0].id, 9);
        assert_eq!(picked[1].id, 7);
    }
}
