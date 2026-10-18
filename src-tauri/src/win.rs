//! 前台窗口状态识别——场景矩阵的另一半输入源。
//!
//! 矩阵的行是「窗口状态」：全屏 / 最大化 / 非桌面焦点 / 桌面。Tauri 只知道自己那几个
//! 窗口，这里直接问 Win32：前台窗口是什么类名、有没有最大化、矩形盖住了多大一块屏幕。
//!
//! 分类逻辑（classify / resolve）是纯函数，和 Win32 调用分开——没有真实窗口也能单测。
//!
//! 前台窗口是本应用自己的时候（提醒窗弹出来了，强提醒窗还会主动抢焦点）不能拿它判定，那会把
//! 场景读成「非桌面焦点」；这时退回上一个「真正属于别人的前台」状态（用户 m06942）。也试过
//! 沿 Z 序往下找第一个不是本进程的窗口，实测不可靠：Z 序里夹着一堆隐藏窗口和任务栏，往下第一个
//! 不是我们的窗口经常就是任务栏（→ 桌面），而用户真正在用的那个窗口可能排在几十个窗口之后。

use crate::config::WindowState;
use std::sync::Mutex;
use std::time::{Duration, Instant};

/// 屏幕坐标下的一块矩形（Win32 的 RECT，左上右下）。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

impl Rect {
    /// `self` 是否完全盖住 `other`。全屏判定就靠它：窗口矩形 ⊇ 显示器矩形。
    pub fn covers(&self, other: &Rect) -> bool {
        self.left <= other.left
            && self.top <= other.top
            && self.right >= other.right
            && self.bottom >= other.bottom
    }
}

/// 把前台窗口的三个事实折成矩阵的一行。
///
/// `zoomed` 优先于矩形比较：任务栏设成自动隐藏时，最大化窗口的矩形就等于显示器矩形，
/// 只看矩形会把「最大化」误判成「全屏」。
pub fn classify(
    class_name: &str,
    zoomed: bool,
    rect: Rect,
    monitor: Rect,
    work: Rect,
) -> WindowState {
    if is_desktop_class(class_name) {
        return WindowState::Desktop;
    }
    if zoomed {
        return WindowState::Maximized;
    }
    if rect.covers(&monitor) {
        return WindowState::Fullscreen;
    }
    if rect.covers(&work) {
        return WindowState::Maximized;
    }
    WindowState::Unfocused
}

/// 桌面壳的类名：Progman = 桌面本体，WorkerW = 换过壁纸后的桌面宿主，
/// Shell_TrayWnd = 任务栏。焦点落在它们身上时，用户看的其实就是桌面。
fn is_desktop_class(class_name: &str) -> bool {
    matches!(class_name, "Progman" | "WorkerW" | "Shell_TrayWnd")
}

/// 上一个「前台不是本应用」时判定的场景，以及记下来的时刻。
static LAST_FOREIGN: Mutex<Option<(WindowState, Instant)>> = Mutex::new(None);

/// 这份记忆多久算过期。每 2 秒的课表轮询会刷新它，所以只有「我们自己的窗口一直占着前台」
/// 才会让年龄变大；真过期了就按桌面处理，不猜。
const LAST_FOREIGN_TTL: Duration = Duration::from_secs(120);

/// 折出这一刻该用的窗口状态。
///
/// `foreign` 是这一刻前台窗口的判定结果；前台是本应用自己的窗口（或压根没有前台窗口）时为
/// None。`cached` 是上一次真正属于别人的前台判定，以及它有多旧。
pub fn resolve(
    foreign: Option<WindowState>,
    cached: Option<(WindowState, Duration)>,
) -> WindowState {
    if let Some(state) = foreign {
        return state;
    }
    match cached {
        Some((state, age)) if age < LAST_FOREIGN_TTL => state,
        _ => WindowState::Desktop,
    }
}

/// 当前场景矩阵该用哪一行。
pub fn current_window_state() -> WindowState {
    let foreign = foreign_foreground_state();
    if let Some(state) = foreign {
        remember(state);
    }
    resolve(foreign, last_foreign())
}

/// 每个轮询周期调一次：前台不是我们的时候就更新记忆，没人发消息时也保持新鲜。
pub fn remember_foreign_foreground() {
    if let Some(state) = foreign_foreground_state() {
        remember(state);
    }
}

fn remember(state: WindowState) {
    if let Ok(mut slot) = LAST_FOREIGN.lock() {
        *slot = Some((state, Instant::now()));
    }
}

fn last_foreign() -> Option<(WindowState, Duration)> {
    LAST_FOREIGN
        .lock()
        .ok()
        .and_then(|slot| slot.map(|(state, at)| (state, at.elapsed())))
}

#[cfg(windows)]
fn foreign_foreground_state() -> Option<WindowState> {
    platform::foreign_foreground_state(platform::own_pid())
}

#[cfg(not(windows))]
fn foreign_foreground_state() -> Option<WindowState> {
    None
}

#[cfg(windows)]
mod platform {
    use super::{classify, Rect, WindowState};

    #[repr(C)]
    #[derive(Clone, Copy, Default)]
    struct RawRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct MonitorInfo {
        cb_size: u32,
        monitor: RawRect,
        work: RawRect,
        flags: u32,
    }

    // HWND / HMONITOR 都是指针宽度的句柄，这里直接用 isize 接。
    #[link(name = "user32")]
    unsafe extern "system" {
        fn GetForegroundWindow() -> isize;
        fn GetWindowThreadProcessId(hwnd: isize, pid: *mut u32) -> u32;
        fn GetClassNameW(hwnd: isize, buf: *mut u16, max: i32) -> i32;
        fn GetWindowRect(hwnd: isize, rect: *mut RawRect) -> i32;
        fn IsZoomed(hwnd: isize) -> i32;
        fn MonitorFromWindow(hwnd: isize, flags: u32) -> isize;
        fn GetMonitorInfoW(monitor: isize, info: *mut MonitorInfo) -> i32;
    }

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentProcessId() -> u32;
    }

    /// 最近的显示器，见 MSDN 的 MONITOR_DEFAULTTONEAREST。
    const MONITOR_DEFAULTTONEAREST: u32 = 2;

    fn rect(raw: RawRect) -> Rect {
        Rect {
            left: raw.left,
            top: raw.top,
            right: raw.right,
            bottom: raw.bottom,
        }
    }

    pub(super) fn own_pid() -> u32 {
        unsafe { GetCurrentProcessId() }
    }

    /// 前台窗口的状态；取不到前台窗口、或前台窗口是本进程自己的时候返回 None。
    pub(super) fn foreign_foreground_state(own_pid: u32) -> Option<WindowState> {
        unsafe {
            let hwnd = GetForegroundWindow();
            if hwnd == 0 {
                return None;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(hwnd, &mut pid);
            if pid == own_pid {
                return None;
            }
            facts(hwnd)
        }
    }

    /// 读一个窗口的判定素材并折成状态；问不出矩形/显示器信息时返回 None。
    ///
    /// 主体包在 unsafe 块里：Rust 2024 的 unsafe fn 不再隐式放开整个函数体，unsafe 的要写在
    /// 眼皮子底下（不然 unsafe_op_in_unsafe_fn 会报警告）。
    fn facts(hwnd: isize) -> Option<WindowState> {
        unsafe {
            let mut window = RawRect::default();
            if GetWindowRect(hwnd, &mut window) == 0 {
                return None;
            }

            let mut buf = [0u16; 64];
            let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32).max(0) as usize;
            let class_name = String::from_utf16_lossy(&buf[..len]);

            let mut info = MonitorInfo {
                cb_size: std::mem::size_of::<MonitorInfo>() as u32,
                monitor: RawRect::default(),
                work: RawRect::default(),
                flags: 0,
            };
            let monitor = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
            if GetMonitorInfoW(monitor, &mut info) == 0 {
                return None;
            }

            Some(classify(
                &class_name,
                IsZoomed(hwnd) != 0,
                rect(window),
                rect(info.monitor),
                rect(info.work),
            ))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect_of(left: i32, top: i32, right: i32, bottom: i32) -> Rect {
        Rect {
            left,
            top,
            right,
            bottom,
        }
    }

    #[test]
    fn classify_uses_window_class_geometry_and_zoom() {
        let monitor = rect_of(0, 0, 1920, 1080);
        let work = rect_of(0, 0, 1920, 1040);
        let windowed = rect_of(100, 100, 800, 600);

        assert_eq!(
            classify("Progman", false, monitor, monitor, work),
            WindowState::Desktop
        );
        assert_eq!(
            classify("WorkerW", false, monitor, monitor, work),
            WindowState::Desktop
        );
        // 任务栏自动隐藏时最大化窗口也盖满显示器，zoomed 必须先判。
        assert_eq!(
            classify("Chrome_WidgetWin_1", true, monitor, monitor, monitor),
            WindowState::Maximized
        );
        // 盖满显示器（含任务栏）且没有最大化标志 → 全屏。
        assert_eq!(
            classify("Chrome_WidgetWin_1", false, monitor, monitor, work),
            WindowState::Fullscreen
        );
        // 只盖住工作区 → 最大化。
        assert_eq!(
            classify("ApplicationFrameWindow", false, work, monitor, work),
            WindowState::Maximized
        );
        assert_eq!(
            classify("ApplicationFrameWindow", false, windowed, monitor, work),
            WindowState::Unfocused
        );
    }

    #[test]
    fn covers_needs_all_four_edges() {
        let monitor = rect_of(0, 0, 1920, 1080);
        assert!(monitor.covers(&monitor));
        assert!(!rect_of(1, 0, 1920, 1080).covers(&monitor));
        assert!(!rect_of(0, 0, 1919, 1080).covers(&monitor));
    }

    #[test]
    fn borderless_fullscreen_on_secondary_monitor_is_fullscreen() {
        // 副屏挂主屏左边，坐标是负的：无边框全屏窗口贴着这块屏幕，一个像素都不能少。
        let monitor = rect_of(-1920, 0, 0, 1080);
        let work = rect_of(-1920, 0, 0, 1040);

        assert_eq!(
            classify("Chrome_WidgetWin_1", false, monitor, monitor, work),
            WindowState::Fullscreen
        );
        // 少 1px 就不算盖住整块屏幕；工作区也没盖住 → 只能是非桌面焦点。
        assert_eq!(
            classify("Chrome_WidgetWin_1", false, rect_of(-1920, 0, -1, 1080), monitor, work),
            WindowState::Unfocused
        );
        // 同样无边框、只盖住工作区 → 最大化。
        assert_eq!(
            classify("Chrome_WidgetWin_1", false, work, monitor, work),
            WindowState::Maximized
        );
    }

    #[test]
    fn foreign_foreground_wins_over_the_memory() {
        assert_eq!(
            resolve(
                Some(WindowState::Fullscreen),
                Some((WindowState::Maximized, Duration::from_secs(1)))
            ),
            WindowState::Fullscreen
        );
    }

    #[test]
    fn our_own_window_falls_back_to_the_last_foreign_state() {
        // 用户 m06942：全屏下弹了提醒，提醒窗自己跑到前台（强提醒窗还会主动抢焦点）。这时不能
        // 拿我们自己的窗口判定，得沿用上一个真正属于别人的前台状态。
        assert_eq!(
            resolve(None, Some((WindowState::Fullscreen, Duration::from_secs(3)))),
            WindowState::Fullscreen
        );
        assert_eq!(
            resolve(None, Some((WindowState::Maximized, Duration::from_secs(30)))),
            WindowState::Maximized
        );
    }

    #[test]
    fn stale_or_missing_memory_means_desktop() {
        assert_eq!(
            resolve(
                None,
                Some((WindowState::Fullscreen, LAST_FOREIGN_TTL + Duration::from_secs(1)))
            ),
            WindowState::Desktop
        );
        assert_eq!(resolve(None, None), WindowState::Desktop);
    }
}
