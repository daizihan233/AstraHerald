//! 上课/下课状态的来源：AstraSchedule/desktop 暴露的本地只读接口
//! （AstraSchedule/desktop PR#92，`GET http://127.0.0.1:3011/status`）。
//!
//! 对方只在渲染进程算出状态后才应答（未就绪返回 503），程序没开或端口被占用就是连不上。
//! 这里每一轮都重连一次，拿到就更新；拿不到就什么都不做，`current_period` 会退回配置里的
//! 「默认状态」（`Config::default_period`）——这正是那个配置项的用武之地。
//!
//! 只做「问一次 /status」这一件事：状态判定（是否在课时内、作息表加一分钟）在对方侧完成，
//! 这边不复制一套课表逻辑，避免两边各自漂移。

use crate::config::{Config, SceneAction, ScenePeriod, WindowState};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::time::Duration;

/// 状态接口地址，与 AstraSchedule/desktop 的 main/status-server.js 保持一致。
const STATUS_ADDR: &str = "127.0.0.1:3011";

/// 真正去连的地址。dev 构建允许用 ASTRA_HERALD_STATUS_ADDR 顶掉它：真课表服务当前是
/// 「下课」时，只有这样才能验「上课 + 延后至下课」那条路。发布版不看这个变量。
fn status_addr() -> String {
    #[cfg(debug_assertions)]
    if let Ok(addr) = std::env::var("ASTRA_HERALD_STATUS_ADDR") {
        let addr = addr.trim();
        if !addr.is_empty() {
            return addr.to_string();
        }
    }
    STATUS_ADDR.to_string()
}
const STATUS_PATH: &str = "/status";
/// 课程清单接口（同一个服务，见 AstraSchedule/desktop 的 main/status-server.js）。
const SUBJECTS_PATH: &str = "/subjects";
/// 轮询周期。对方每秒推一次状态，2s 足够新鲜，也不至于把对方当心跳打。
const POLL_INTERVAL: Duration = Duration::from_secs(2);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(400);
const IO_TIMEOUT: Duration = Duration::from_millis(800);
/// 一屏状态 JSON 只有几百字节，给 256KB 是防呆上限，不是为了正常读到。
const MAX_BODY: u64 = 256 * 1024;

/// 最近一次拿到的上课状态：0 = 没拿到过，1 = 上课，2 = 下课。
static LAST_CODE: AtomicU8 = AtomicU8::new(0);

/// 最近一轮轮询是不是真的打通了 /status。LAST_CODE 只记结论（状态读不懂时也是 0），
/// 「连没连上课表服务」得单独记——配置页顶栏那盏灯看的就是它。
static LIVE: AtomicBool = AtomicBool::new(false);

/// 课表服务当前是否打通（每 2s 更新一次，不存在「当场再问一次」的路径）。
pub fn is_live() -> bool {
    LIVE.load(Ordering::Relaxed)
}

/// 当前该按哪个状态算。有实时快照就用实时的，否则用配置里的默认状态。
pub fn current_period(config: &Config) -> ScenePeriod {
    code_to_period(LAST_CODE.load(Ordering::Relaxed)).unwrap_or(config.default_period)
}

/// 当前场景三元组：上课状态、前台窗口状态、矩阵给出的提醒方式。
pub fn current_scene(config: &Config) -> (ScenePeriod, WindowState, SceneAction) {
    let period = current_period(config);
    let state = crate::win::current_window_state();
    (period, state, config.current_action(period, state))
}

/// 后台线程：每 2 秒问一次状态，然后让延后队列看一眼能不能放行
/// （「延后至下课」的出口不只在状态翻转那一刻，窗口状态变了同样算离开那一格）。
pub fn start_watch() {
    std::thread::spawn(|| {
        // 只在状态翻转时打一行日志（每 2s 一条会把 app.log 淹掉）。
        let mut logged_code = u8::MAX;
        loop {
            let next = fetch_status().and_then(|body| period_from_status(&body));
            let code = period_to_code(next);
            // 先记「通不通」再记「什么状态」：后者要把 next 消费掉。
            LIVE.store(next.is_some(), Ordering::Relaxed);
            LAST_CODE.store(code, Ordering::Relaxed);
            if code != logged_code {
                logged_code = code;
                match code_to_period(code) {
                    Some(p) => log::info!("课表状态：{}（来自 /status）", p.label()),
                    None => log::info!("课表状态：读不到 /status，提醒按配置里的默认状态算"),
                }
            }
            crate::defer::release_ready();
            std::thread::sleep(POLL_INTERVAL);
        }
    });
}

fn code_to_period(code: u8) -> Option<ScenePeriod> {
    match code {
        1 => Some(ScenePeriod::InClass),
        2 => Some(ScenePeriod::AfterClass),
        _ => None,
    }
}

fn period_to_code(period: Option<ScenePeriod>) -> u8 {
    match period {
        Some(ScenePeriod::InClass) => 1,
        Some(ScenePeriod::AfterClass) => 2,
        None => 0,
    }
}

/// 只读地取一次 /status。任何一步失败都是 None——调用方只关心「有没有拿到」。
fn fetch_status() -> Option<String> {
    http_get(STATUS_PATH)
}

fn http_get(path: &str) -> Option<String> {
    let host = status_addr();
    let addr = host.to_socket_addrs().ok()?.next()?;
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).ok()?;
    stream.set_read_timeout(Some(IO_TIMEOUT)).ok()?;
    stream.set_write_timeout(Some(IO_TIMEOUT)).ok()?;

    // status-server.js 固定带 Content-Length 且这里要求 Connection: close，
    // 所以 read_to_end 会正常收尾，不用自己解分块编码。
    write!(
        stream,
        "GET {path} HTTP/1.1\r\nHost: {host}\r\nAccept: application/json\r\nConnection: close\r\n\r\n"
    )
    .ok()?;

    let mut raw = Vec::new();
    stream.take(MAX_BODY).read_to_end(&mut raw).ok()?;
    let text = String::from_utf8_lossy(&raw);
    let (head, body) = text.split_once("\r\n\r\n")?;
    if !head.starts_with("HTTP/1.1 200") {
        // 503（快照未就绪）在这里就被丢掉了，不会把错误响应体当成状态。
        return None;
    }
    Some(body.to_string())
}

/// 课表里的科目清单，元素形如 `{shortName, fullName}`。
///
/// `/subjects` 会给两份：`subjects`（今天课表上的科目）和 `catalog`（配置里的全部科目）。
/// 判断「这位老师教哪一科」要看全部科目，所以两份合起来、去重后返回。
/// 连不上课表服务时返回空表——调用方按「没匹配上」处理，不会因此不干活。
pub fn subject_names() -> Vec<(String, String)> {
    http_get(SUBJECTS_PATH)
        .map(|body| subjects_from_json(&body))
        .unwrap_or_default()
}

fn subjects_from_json(body: &str) -> Vec<(String, String)> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut seen = HashSet::new();
    let mut subjects = Vec::new();
    for key in ["subjects", "catalog"] {
        let Some(items) = value.get(key).and_then(|list| list.as_array()) else {
            continue;
        };
        for item in items {
            let short = item
                .get("shortName")
                .and_then(|name| name.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            let full = item
                .get("fullName")
                .and_then(|name| name.as_str())
                .unwrap_or("")
                .trim()
                .to_string();
            if short.is_empty() && full.is_empty() {
                continue;
            }
            if seen.insert((short.clone(), full.clone())) {
                subjects.push((short, full));
            }
        }
    }
    subjects
}

/// 从 /status 的 JSON 里读状态。优先 `state`，读不到再退化到 `inClass` 布尔。
fn period_from_status(body: &str) -> Option<ScenePeriod> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    match value.get("state").and_then(|state| state.as_str()) {
        Some("in_class") => Some(ScenePeriod::InClass),
        Some("break") => Some(ScenePeriod::AfterClass),
        _ => value
            .get("inClass")
            .and_then(|in_class| in_class.as_bool())
            .map(|in_class| if in_class { ScenePeriod::InClass } else { ScenePeriod::AfterClass }),
    }
}

/// 当前与下一节课的科目名（用来判断「这位老师的课是不是正在上或即将上」）。
///
/// 只在收到文件消息时问一次，不复用 2s 轮询的缓存：文件消息很少，多一次本地 GET
/// 换实现简单。`fullName` 优先，没有再用 `shortName`。
pub fn current_and_next_subjects() -> Vec<String> {
    http_get(STATUS_PATH)
        .map(|body| lessons_from_json(&body))
        .unwrap_or_default()
}

fn lessons_from_json(body: &str) -> Vec<String> {
    let Ok(value) = serde_json::from_str::<serde_json::Value>(body) else {
        return Vec::new();
    };
    let mut names = Vec::new();
    for key in ["currentLesson", "nextLesson"] {
        let Some(lesson) = value.get(key) else {
            continue;
        };
        if lesson.is_null() {
            continue;
        }
        for field in ["fullName", "shortName"] {
            let Some(name) = lesson.get(field).and_then(|v| v.as_str()) else {
                continue;
            };
            let name = name.trim();
            if !name.is_empty() {
                names.push(name.to_string());
                break;
            }
        }
    }
    names
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 形状取自 AstraSchedule/desktop PR#92 的 getScheduleStatus()：
    /// 上课时 state = "in_class"，课间/放学 state = "break"。
    #[test]
    fn period_reads_astra_schedule_status() {
        let in_class = r#"{"reportedAt":"2026-10-17T08:01:02.000Z","time":"08:01:02","inClass":true,"state":"in_class","currentLesson":{"fullName":"语文","shortName":"语","startAt":"08:00","endAt":"08:46"},"nextLesson":null,"nextChange":{"type":"class_end","at":"08:46","inSeconds":2698}}"#;
        let at_break = r#"{"reportedAt":"2026-10-17T08:50:00.000Z","time":"08:50:00","inClass":false,"state":"break","currentLesson":null,"nextLesson":{"fullName":"数学","shortName":"数","startAt":"09:00","endAt":"09:46"},"nextChange":{"type":"class_start","at":"09:00","inSeconds":600}}"#;
        assert_eq!(period_from_status(in_class), Some(ScenePeriod::InClass));
        assert_eq!(period_from_status(at_break), Some(ScenePeriod::AfterClass));
    }

    /// 未就绪的 503 响应体、以及任何读不懂的应答，都必须当成「没拿到」，
    /// 否则会拿默认状态去覆盖一个其实还活着的连接。
    #[test]
    fn unreadable_status_is_none() {
        let unavailable = r#"{"error":"status_unavailable","message":"课表状态尚未就绪"}"#;
        assert_eq!(period_from_status(unavailable), None);
        assert_eq!(period_from_status("not json"), None);
        // 只有 inClass、没有 state 的老形状也能读。
        assert_eq!(period_from_status(r#"{"inClass":true}"#), Some(ScenePeriod::InClass));
        assert_eq!(period_from_status(r#"{"inClass":false}"#), Some(ScenePeriod::AfterClass));
        assert_eq!(period_from_status(r#"{"state":"unknown"}"#), None);
    }

    /// 文件消息自动打开要问「当前/下一节是哪门课」：正在上课时取 currentLesson，
    /// 课间取 nextLesson；空课表两边都给 null。
    #[test]
    fn lessons_come_from_current_and_next_lesson() {
        let in_class = r#"{"state":"in_class","currentLesson":{"fullName":"数学","shortName":"数"},"nextLesson":{"fullName":"语文","shortName":"语"}}"#;
        assert_eq!(lessons_from_json(in_class), vec!["数学", "语文"]);
        let at_break = r#"{"state":"break","currentLesson":null,"nextLesson":{"fullName":"物理","shortName":"物"}}"#;
        assert_eq!(lessons_from_json(at_break), vec!["物理"]);
        let nothing = r#"{"state":"break","currentLesson":null,"nextLesson":null}"#;
        assert!(lessons_from_json(nothing).is_empty());
        assert!(lessons_from_json("not json").is_empty());
    }

    #[test]
    fn code_round_trips() {
        assert_eq!(code_to_period(period_to_code(None)), None);
        assert_eq!(code_to_period(period_to_code(Some(ScenePeriod::InClass))), Some(ScenePeriod::InClass));
        assert_eq!(code_to_period(period_to_code(Some(ScenePeriod::AfterClass))), Some(ScenePeriod::AfterClass));
        // 没拿到过快照时用配置里的默认状态。
        let mut config = Config::default();
        config.default_period = ScenePeriod::InClass;
        assert_eq!(current_period(&config), ScenePeriod::InClass);
    }
}
