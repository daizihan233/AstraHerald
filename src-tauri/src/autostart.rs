//! 开机自启：在 HKCU 的 Run 键里写一条指向本程序的值。
//!
//! 没引 tauri-plugin-autostart：需求就是「开机把本程序拉起来」，reg.exe 三行能干完，
//! 插件要多一层平台适配、多一个随版本升级的依赖。
//!
//! 用 HKCU 而不是 HKLM：不需要管理员权限，装到 Program Files 里也能开。

use std::path::Path;

/// 自启项的位置。用户能在任务管理器的「启动」页看到它。
const RUN_KEY: &str = r"HKCU\Software\Microsoft\Windows\CurrentVersion\Run";

/// 注册表值名：统一用应用名。
const VALUE_NAME: &str = "AstraHerald";

/// 开发构建（target/debug、target/release）不写注册表。
///
/// 否则跑一次 `cargo run` 就把调试版塞进开机启动，而且那个路径随时会被
/// 重新编译覆盖或删掉，留下的是一条开机必报错的死项。
pub fn is_dev_build(exe: &Path) -> bool {
    let dir = exe.parent();
    let under_dev_dir = dir.and_then(|d| d.file_name()).is_some_and(|name| {
        name.eq_ignore_ascii_case("debug") || name.eq_ignore_ascii_case("release")
    });
    let under_target = dir
        .and_then(|d| d.parent())
        .and_then(|p| p.file_name())
        .is_some_and(|name| name.eq_ignore_ascii_case("target"));
    under_dev_dir && under_target
}

/// 注册表里该写的数据：带引号的完整路径。
///
/// Run 键的值会被当命令行解析，路径含空格时不套引号会被拆成两个参数，
/// 于是开机时报「找不到文件」。
fn value_data(exe: &Path) -> String {
    format!("\"{}\"", exe.display())
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::process::{Command, Output};

    /// CREATE_NO_WINDOW：这是个 GUI 程序，不能因为查一次注册表闪一个黑框。
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;

    /// 跑一次 reg.exe。参数直接给，不做 shell 拼接。
    pub(super) fn reg(args: &[&str]) -> Result<Output, String> {
        use std::os::windows::process::CommandExt;
        Command::new("reg.exe")
            .args(args)
            .creation_flags(CREATE_NO_WINDOW)
            .output()
            .map_err(|e| format!("执行 reg.exe 失败：{e}"))
    }

    /// 注册表里现在有没有这条自启项。
    pub fn is_enabled() -> bool {
        reg(&["query", RUN_KEY, "/v", VALUE_NAME])
            .map(|out| out.status.success())
            .unwrap_or(false)
    }

    pub fn set(enabled: bool, exe: &Path) -> Result<(), String> {
        let out = if enabled {
            let data = value_data(exe);
            reg(&[
                "add", RUN_KEY, "/v", VALUE_NAME, "/t", "REG_SZ", "/d", &data, "/f",
            ])?
        } else {
            reg(&["delete", RUN_KEY, "/v", VALUE_NAME, "/f"])?
        };

        // 关闭时条目本来就不存在：reg delete 会报错，但这就是我们要的结果。
        if out.status.success() || !enabled {
            return Ok(());
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        Err(format!(
            "reg.exe 退出码 {:?}：{}",
            out.status.code(),
            stderr.trim()
        ))
    }
}

#[cfg(not(windows))]
mod platform {
    use super::*;

    pub fn is_enabled() -> bool {
        false
    }

    pub fn set(_enabled: bool, _exe: &Path) -> Result<(), String> {
        Err("开机自启目前只实现了 Windows".to_string())
    }
}

/// 把配置里的期望状态落到注册表。
///
/// 失败只留日志，不往上抛：自启是「顺手打开」的体验项，配置页保存
/// 不该因为注册表被组策略锁死而整体失败。
pub fn sync(enabled: bool) {
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => {
            eprintln!("拿不到当前可执行文件路径，跳过开机自启设置：{e}");
            return;
        }
    };
    if is_dev_build(&exe) {
        eprintln!(
            "当前是开发构建（{}），跳过开机自启设置；打包安装后才会生效（注册表现状：{}）",
            exe.display(),
            if platform::is_enabled() { "已开启" } else { "未开启" }
        );
        return;
    }
    match platform::set(enabled, &exe) {
        Ok(()) => eprintln!(
            "开机自启{}（{}）",
            if enabled { "已开启" } else { "已关闭" },
            exe.display()
        ),
        Err(e) => eprintln!("设置开机自启失败：{e}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dev_build_paths_are_recognized() {
        assert!(is_dev_build(Path::new(
            r"E:\astra-herald\src-tauri\target\debug\astra-herald.exe"
        )));
        assert!(is_dev_build(Path::new(
            r"E:\astra-herald\src-tauri\target\RELEASE\AstraHerald.exe"
        )));
        // 打包安装后的路径：要管。
        assert!(!is_dev_build(Path::new(
            r"C:\Program Files\AstraHerald\AstraHerald.exe"
        )));
        // 只有一段叫 target 的目录不算开发构建。
        assert!(!is_dev_build(Path::new(r"C:\target\AstraHerald.exe")));
    }

    #[test]
    fn value_data_quotes_the_path() {
        assert_eq!(
            value_data(Path::new(r"C:\Program Files\AstraHerald\AstraHerald.exe")),
            "\"C:\\Program Files\\AstraHerald\\AstraHerald.exe\""
        );
    }

    /// 真跑一遍 reg.exe：带空格的安装路径写进去再读回来，必须还是同一个带引号的字符串。
    ///
    /// 写的是自建的 HKCU\\Software\\AstraHerald\\selftest 键，不是开机启动项本身，
    /// 所以这个测试即使失败也不会给机器留下一条开机自启。
    #[cfg(windows)]
    #[test]
    fn reg_round_trip_keeps_quoted_path() {
        const KEY: &str = r"HKCU\Software\AstraHerald\selftest";
        const VAL: &str = "PathProbe";
        let probe = Path::new(r"C:\Program Files\Astra Herald\AstraHerald.exe");
        let data = value_data(probe);

        let added =
            platform::reg(&["add", KEY, "/v", VAL, "/t", "REG_SZ", "/d", &data, "/f"]).unwrap();
        assert!(
            added.status.success(),
            "reg add 失败：{}",
            String::from_utf8_lossy(&added.stderr)
        );

        let queried = platform::reg(&["query", KEY, "/v", VAL]).unwrap();
        let text = String::from_utf8_lossy(&queried.stdout);
        // 不管断言过不过，先把测试键删掉。
        let _ = platform::reg(&["delete", KEY, "/f"]);

        assert!(
            queried.status.success(),
            "reg query 失败：{}",
            String::from_utf8_lossy(&queried.stderr)
        );
        assert!(text.contains(&data), "读回的带引号路径和写入的不一致：{text}");
    }
}

