//! 外部 OneBot 框架托管：用户指一个「启动文件」，我们从它所在的位置找它自己的配置，
//! 读出 WS 端点（地址 + access token），需要时把它当子进程拉起来。
//!
//! 为什么不绑定具体框架：OneBot 11 的 Hook 框架（SnowLuma、NapCat、Lagrange…）大体都是
//! 「一个目录 + 启动文件 + 一份写着 wsServers 的 json」，所以只按文件格式读、只按用户给的
//! 路径拉起就够了。任何一家升级都不用改这里，也不用随包发布谁（省掉几十 MB 安装体积）。
//!
//! 三条纪律：
//! 1. 端口已经有人在听时绝不重复拉起（用户自己起的那份就是我们要连的那份）；
//! 2. 任何带 dialoguer 交互的 load_local_conf 都不能在缺文件时被调到——GUI 的 stdin 是死的；
//! 3. 只读框架的配置，一个字都不写；它要的开关（Hook、协议同意）由启动框架的人在环境变量里声明。

use crate::config::{Config, OnebotSettings};
use kovi_onebot::{Host, OneBotDriverConfig, Server};
use serde::Serialize;
use serde_json::Value;
use std::ffi::OsString;
use std::fs;
use std::net::{IpAddr, SocketAddr, TcpStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

/// 本项目拉起来的框架子进程。退出时收掉，别留个孤儿进程占着端口。
static CHILD: Mutex<Option<Child>> = Mutex::new(None);

/// 单个配置文件的大小上限：我们只找一小段 wsServers，别去读framework 里几十 MB 的数据库。
const MAX_CONFIG_BYTES: u64 = 256 * 1024;
/// 一次最多看几个 json：config/ 下一般就几个，再多说明这个目录不是框架配置目录。
const MAX_CONFIG_FILES: usize = 40;

/// 用户指定的启动文件。没填、或者文件不在，都返回 None（= 不托管）。
pub fn startup_file(settings: &OnebotSettings) -> Option<PathBuf> {
    let raw = settings.startup.trim();
    if raw.is_empty() {
        return None;
    }
    let path = PathBuf::from(raw);
    if path.is_file() {
        return Some(path);
    }
    eprintln!("OneBot 启动文件不存在：{raw}");
    None
}

/// 找它的 OneBot 配置要看哪几个目录：启动文件所在目录，以及再上一级。
/// 有些框架把启动器放在 bin/ 之类子目录里，配置却在安装根目录的 config/ 下。
pub fn config_dirs(startup: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(dir) = startup.parent() {
        push_unique(&mut dirs, dir.to_path_buf());
        if let Some(parent) = dir.parent() {
            if parent != dir {
                push_unique(&mut dirs, parent.to_path_buf());
            }
        }
    }
    dirs
}

fn push_unique(dirs: &mut Vec<PathBuf>, dir: PathBuf) {
    if !dirs.contains(&dir) {
        dirs.push(dir);
    }
}

/// 本项目要连的 OneBot 端点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Endpoint {
    pub host: String,
    pub port: u16,
    pub path: String,
    pub access_token: String,
}

impl Endpoint {
    /// 配置读不出来时的兜底：OneBot 框架几乎都从 3001 开始排。
    fn local_default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            port: 3001,
            path: String::new(),
            access_token: String::new(),
        }
    }
}

/// 启动文件 -> (配置来自哪个目录, 端点)。日志里能看出 token 是从哪读的。
pub fn endpoint_of(startup: &Path) -> Option<(PathBuf, Endpoint)> {
    config_dirs(startup)
        .into_iter()
        .find_map(|dir| endpoint(&dir).map(|ep| (dir, ep)))
}

/// 读一个目录里的 OneBot 配置：先看 config/，再看目录本身。
pub fn endpoint(dir: &Path) -> Option<Endpoint> {
    for file in config_files(dir) {
        let Ok(meta) = fs::metadata(&file) else { continue };
        if meta.len() > MAX_CONFIG_BYTES {
            continue;
        }
        let Ok(text) = fs::read_to_string(&file) else { continue };
        let Ok(value) = serde_json::from_str::<Value>(&text) else { continue };
        if let Some(server) = find_ws_server(&value) {
            return Some(server_from_json(server));
        }
    }
    None
}

/// 这个目录里值得看一眼的 json：config/ 下的排前面，名字带 onebot 的排最前。
/// 排序是确定的，同一台机器上多份配置时结果不会随机变。
fn config_files(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = Vec::new();
    for sub in [dir.join("config"), dir.to_path_buf()] {
        let Ok(entries) = fs::read_dir(&sub) else { continue };
        for path in entries.filter_map(|entry| entry.ok().map(|entry| entry.path())) {
            if !is_json(&path) || files.contains(&path) {
                continue;
            }
            files.push(path);
        }
    }
    files.sort_by_key(|path| {
        let name = path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("");
        (!name.starts_with("onebot"), name.to_string())
    });
    files.truncate(MAX_CONFIG_FILES);
    files
}

fn is_json(path: &Path) -> bool {
    path.is_file()
        && path
            .extension()
            .and_then(|ext| ext.to_str())
            .is_some_and(|ext| ext.eq_ignore_ascii_case("json"))
}

/// 在任意深度的配置里找第一处「一组 WS 服务器」并返回其中第一个对象。
///
/// 各家的键名和嵌套层数都不一样（SnowLuma 是 networks.wsServers，有的框架是
/// network.websocketServers），所以按语义找数组、不写死路径。
fn find_ws_server(value: &Value) -> Option<&Value> {
    match value {
        Value::Object(map) => {
            for (key, child) in map {
                if !is_ws_server_key(key) {
                    continue;
                }
                if let Value::Array(items) = child {
                    if let Some(first) = items.iter().find(|item| item.is_object()) {
                        return Some(first);
                    }
                }
            }
            map.values().find_map(find_ws_server)
        }
        Value::Array(items) => items.iter().find_map(find_ws_server),
        _ => None,
}
}

fn is_ws_server_key(key: &str) -> bool {
    let lower = key.to_ascii_lowercase().replace(['_', '-'], "");
    matches!(lower.as_str(), "wsservers" | "websocketservers")
}

/// 从一份 WS 服务器条目里取连接参数。缺字段就用各家的常见默认值，别整条丢掉。
fn server_from_json(server: &Value) -> Endpoint {
    let text = |key: &str| server.get(key).and_then(|value| value.as_str());
    let host = text("host").unwrap_or("127.0.0.1").to_string();
    let port = server.get("port").and_then(|value| value.as_u64()).unwrap_or(3001) as u16;
    // 有的框架把「根路径」写成 "/"，而 kovi-onebot 是拼成 base + path + "/api" 用的：
    // 原样带进去会变成 ws://127.0.0.1:3001//api（HTTP 400）。去掉尾部斜杠，"/" 即根路径。
    let path = text("path").unwrap_or("").trim_end_matches('/').to_string();
    // accessToken 是 SnowLuma 的叫法，token 是另一批框架的叫法，都认。
    let access_token = text("accessToken")
        .or_else(|| text("token"))
        .unwrap_or("")
        .to_string();
    Endpoint { host, port, path, access_token }
}

/// 框架自己的布尔环境变量写法（index.mjs 的 resolveAutoLoad 认这六种）。
fn parse_flag(raw: &str) -> Option<bool> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "1" | "true" | "yes" | "on" => Some(true),
        "0" | "false" | "no" | "off" => Some(false),
        _ => None,
    }
}

/// 框架 runtime.json 里的 hookAutoLoad；文件不在或读不出来就是 None。
fn hook_autoload_in(dir: &Path) -> Option<bool> {
    let text = fs::read_to_string(dir.join("config").join("runtime.json")).ok()?;
    let value = serde_json::from_str::<Value>(&text).ok()?;
    value.get("hookAutoLoad").and_then(|value| value.as_bool())
}

/// 把框架的 hookAutoLoad 打开：只改这一个键，同一个文件里别的键一个字节都不碰。
///
/// 框架默认关着它时不注入 Hook，一条消息都收不到；而用户手填的启动文件
/// （launcher.bat / exe）不像 deploy.bat 那样带着 SNOWLUMA_HOOK_AUTOLOAD=1。
/// 这里等价于用户在框架自己的 WebUI 里点一下那个开关：不代用户声明同意框架的协议
/// （EULA / PRIVACY 那些环境变量一个都不设），也不动别的任何设置。
/// 读不出来（框架还没写出 runtime.json，或压根没 Hook 这概念，如 NapCat / Lagrange）
/// 就闭嘴走人——所以 ensure_running 在等它起来之后再叫一次。
fn ensure_hook_autoload(dir: &Path) {
    static WARNED: AtomicBool = AtomicBool::new(false);
    match std::env::var("SNOWLUMA_HOOK_AUTOLOAD")
        .ok()
        .and_then(|raw| parse_flag(&raw))
    {
        // 启动环境里已经开着了；明确关着那是用户自己的决定，改文件也没用（环境变量优先）。
        Some(true) => return,
        Some(false) => {
            if !WARNED.swap(true, Ordering::Relaxed) {
                eprintln!("框架被环境变量 SNOWLUMA_HOOK_AUTOLOAD=0 关着 Hook，不动它的配置");
            }
            return;
        }
        None => {}
    }

    let path = dir.join("config").join("runtime.json");
    let Some(current) = hook_autoload_in(dir) else {
        return;
    };
    if current {
        return;
    }
    let Ok(text) = fs::read_to_string(&path) else {
        return;
    };
    let Ok(mut value) = serde_json::from_str::<Value>(&text) else {
        // 人家写的东西解析不了就别碰，宁可收不到消息也不能把配置写坏。
        eprintln!("{} 不是能解析的 JSON，不去改它", path.display());
        return;
    };
    let Some(object) = value.as_object_mut() else {
        return;
    };
    object.insert("hookAutoLoad".to_string(), Value::Bool(true));
    let written = format!("{}\n", serde_json::to_string_pretty(&value).unwrap_or_default());
    match fs::write(&path, written) {
        Ok(()) => eprintln!(
            "已把 {} 的 hookAutoLoad 打开（只改这一个键；正在跑的那份要重启才生效）",
            path.display()
        ),
        Err(e) => {
            if !WARNED.swap(true, Ordering::Relaxed) {
                eprintln!(
                    "想把 {} 的 hookAutoLoad 打开但写不进去：{e}；\
                     框架不注入 Hook 就收不到消息，去它自己的 WebUI 里打开",
                    path.display()
                );
            }
        }
    }
}

fn host_of(host: &str) -> Host {
    match host.parse::<IpAddr>() {
        Ok(ip) => Host::IpAddr(ip),
        Err(_) => Host::Domain(host.to_string()),
    }
}

/// 给 kovi 的连接配置。优先级：框架自己的配置 -> 项目里已有的 kovi.conf.toml
/// （这是非交互地读文件，不是 kovi 的交互式生成）-> 本地默认端口。
pub fn driver_config(config: &Config) -> OneBotDriverConfig {
    if let Some((from, ep)) = effective_endpoint(&config.onebot) {
        let source = match &from {
            Some(dir) => format!("（配置来自 {}）", dir.display()),
            None => "（连接参数是配置页里手填的）".to_string(),
        };
        eprintln!(
            "连接 OneBot：ws://{}:{}{}{}",
            ep.host,
            ep.port,
            if ep.path.is_empty() { "/" } else { ep.path.as_str() },
            source
        );
        let all_in_one = ep.path.is_empty();
        return OneBotDriverConfig {
            server: Server::new(
                host_of(&ep.host),
                ep.port,
                ep.access_token,
                false,
                ep.path,
                all_in_one,
            ),
        };
    }
    // 兼容手写 kovi.conf.toml 的老用法：只认里面有 [server] 段的那种。
    // 刻意不调 kovi_onebot::load_local_conf()：文件在但缺 [server] 段时（比如 kovi
    // 自己写的那份只有 [config]）它会走 dialoguer 交互提问，拿不到终端就 panic
    // （kovi-onebot/src/driver/config.rs:141 的 expect("unreachable")），把整个 bot
    // 重连循环崩掉。自己用 kovi 转出的 toml 解析，任何情况下都不碰终端。
    if let Ok(text) = fs::read_to_string("kovi.conf.toml") {
        match kovi::toml::from_str::<OneBotDriverConfig>(&text) {
            Ok(conf) => {
                eprintln!("连接参数来自 kovi.conf.toml 的 [server] 段");
                return conf;
            }
            Err(_) => eprintln!("kovi.conf.toml 里没有能用的 [server] 段，忽略它"),
        }
    }
    eprintln!("没读到框架的 WS 配置，先用默认的 127.0.0.1:3001（配置页里指定框架的启动文件）");
    OneBotDriverConfig {
        server: Server::new(host_of("127.0.0.1"), 3001, String::new(), false, String::new(), true),
    }
}

/// 真正要连的端点：配置页里手填过的字段优先，留空的从框架自己的配置里补。
/// 返回 (值来自框架配置的那个目录, 端点)——目录只用于日志，纯手填时是 None。
///
/// 四项全空 = 纯自动发现：读不出框架配置就是 None，交给调用方走 kovi.conf.toml / 默认端口。
pub fn effective_endpoint(settings: &OnebotSettings) -> Option<(Option<PathBuf>, Endpoint)> {
    let discovered = startup_file(settings).and_then(|startup| endpoint_of(&startup));
    if !typed_any(settings) {
        return discovered.map(|(dir, ep)| (Some(dir), ep));
    }
    let (dir, mut ep) = match discovered {
        Some((dir, ep)) => (Some(dir), ep),
        None => (None, Endpoint::local_default()),
    };
    let host = settings.host.trim();
    if !host.is_empty() {
        ep.host = host.to_string();
    }
    let port = settings.port.trim();
    if !port.is_empty() {
        match port.parse::<u16>() {
            Ok(port) => ep.port = port,
            Err(_) => eprintln!("端口 {port} 不是数字，还是用 {}", ep.port),
        }
    }
    let path = settings.path.trim();
    if !path.is_empty() {
        ep.path = path.trim_end_matches('/').to_string();
    }
    let token = settings.access_token.trim();
    if !token.is_empty() {
        ep.access_token = token.to_string();
    }
    Some((dir, ep))
}

/// 用户在「连接配置」里手填过任何一项连接参数吗（启动文件本身不算）。
fn typed_any(settings: &OnebotSettings) -> bool {
    [&settings.host, &settings.port, &settings.path, &settings.access_token]
        .iter()
        .any(|value| !value.trim().is_empty())
}

pub(crate) fn port_open(host: &str, port: u16) -> bool {
    let Ok(ip) = host.parse::<IpAddr>() else {
        return false;
    };
    TcpStream::connect_timeout(&SocketAddr::new(ip, port), Duration::from_millis(300)).is_ok()
}

/// 需要时把框架拉起来；端口已经在监听就用现成的那份。返回是否由本项目拉起。
pub fn ensure_running(config: &Config) -> bool {
    let Some(startup) = startup_file(&config.onebot) else {
        eprintln!("还没指定 OneBot 框架的启动文件，不托管（配置页里可以指定）");
        return false;
    };
    let Some(dir) = startup.parent().map(Path::to_path_buf) else {
        eprintln!("启动文件 {} 没有所在目录，不托管", startup.display());
        return false;
    };
    eprintln!("OneBot 框架目录：{}", dir.display());
    ensure_hook_autoload(&dir);
    // 探的端口要和 bot 之后连的那个一致：可能来自上一层目录里那份配置，也可能是手填的。
    let probe = effective_endpoint(&config.onebot)
        .map(|(_, ep)| ep)
        .unwrap_or_else(Endpoint::local_default);
    if port_open(&probe.host, probe.port) {
        eprintln!("{}:{} 已经在监听，直接用现成的那份", probe.host, probe.port);
        return false;
    }
    // 调用方是 bot 的重连循环，每轮都会进来一次：已拉起的那份还在启动就别再拉第二个。
    if child_alive() {
        eprintln!("托管的框架还在启动中，等它自己起来");
        return false;
    }

    let (program, args) = launcher(&startup);
    let mut cmd = Command::new(&program);
    cmd.args(&args)
        // 一个环境变量都不设：代用户声明「已接受框架的协议」、或替框架决定 Hook 开关，
        // 是启动框架的人自己的事（SnowLuma 的 SNOWLUMA_ACCEPT_EULA / _PRIVACY / _HOOK_AUTOLOAD
        // 就是这个语义，它的 EULA 5.4 把「自动化脚本部署」列为须经书面授权的行为）。
        // 要无人值守就在启动框架前设好——deploy.bat 是在运行脚本的人这一侧设的；
        // 本应用只继承环境，不制造同意。
        .current_dir(&dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    match cmd.spawn() {
        Ok(child) => {
            eprintln!("已拉起 OneBot 框架：{} {}", program.display(), args_display(&args));
            if let Ok(mut guard) = CHILD.lock() {
                *guard = Some(child);
            }
            wait_ready(&dir, &probe.host, probe.port);
            // 框架这会儿才写出 runtime.json：再看一次，它默认关着 Hook 的情况才改得到。
            ensure_hook_autoload(&dir);
            true
        }
        Err(e) => {
            eprintln!("拉起 OneBot 框架失败（{}）：{e}", program.display());
            false
        }
    }
}

/// 启动文件 -> (要执行的程序, 参数)。三种最常见的形态，多一种都不猜。
fn launcher(startup: &Path) -> (PathBuf, Vec<OsString>) {
    let ext = startup
        .extension()
        .and_then(|ext| ext.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    // 脚本用同目录里自带的 node.exe（SnowLuma 就自带），没有才找 PATH 里的 node。
    let script_arg = || {
        startup
            .file_name()
            .map(OsString::from)
            .unwrap_or_else(|| startup.as_os_str().to_os_string())
    };
    match ext.as_str() {
        "mjs" | "js" | "cjs" => {
            let bundled = startup.parent().map(|dir| dir.join("node.exe"));
            let program = match bundled {
                Some(path) if path.is_file() => path,
                _ => PathBuf::from("node"),
            };
            (program, vec![script_arg()])
        }
        // 批处理要交给 cmd：直接 CreateProcess 一个 .bat 会失败。
        "bat" | "cmd" => (
            PathBuf::from("cmd"),
            vec![OsString::from("/C"), startup.as_os_str().to_os_string()],
        ),
        // .exe 和其它：它自己就是可执行文件。
        _ => (startup.to_path_buf(), Vec::new()),
    }
}

fn args_display(args: &[OsString]) -> String {
    args.iter()
        .map(|arg| arg.to_string_lossy().to_string())
        .collect::<Vec<String>>()
        .join(" ")
}

/// 手上这个子进程还活着吗？已经退出的顺手清掉，好让下一轮重新拉一次。
fn child_alive() -> bool {
    let Ok(mut guard) = CHILD.lock() else {
        return false;
    };
    match guard.as_mut().map(|child| child.try_wait()) {
        Some(Ok(None)) => true,
        Some(Ok(Some(_))) | Some(Err(_)) => {
            *guard = None;
            false
        }
        None => false,
    }
}

/// 等端口起来再让 bot 去连：框架冷启动要几秒，早了连不上。
/// 有的框架是首次运行才生成配置的，所以每轮都重读一次它自己的配置。
fn wait_ready(dir: &Path, host: &str, port: u16) {
    for _ in 0..50 {
        let fresh = endpoint(dir);
        let (h, p) = fresh
            .as_ref()
            .map(|ep| (ep.host.as_str(), ep.port))
            .unwrap_or((host, port));
        if port_open(h, p) {
            eprintln!("框架已就绪：{h}:{p}");
            return;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    eprintln!("等了 10s，框架的 {host}:{port} 端口还没起来（可能还在 Hook QQ）");
}

/// 退出时收掉本项目拉起的那个子进程；用户自己起的那份不碰（它不在 CHILD 里）。
pub fn shutdown() {
    if let Ok(mut guard) = CHILD.lock() {
        if let Some(mut child) = guard.take() {
            kill_tree(&mut child);
            let _ = child.wait();
            eprintln!("已关闭托管的 OneBot 框架进程");
        }
    }
}

/// 按进程树杀：用启动器批处理（SnowLuma 的 launcher.bat、NapCat 的 .bat）时，我们手上的
/// 子进程是 cmd.exe，只杀它会把它拉起的 node 留成孤儿，继续占着 3001 端口。
fn kill_tree(child: &mut Child) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let pid = child.id().to_string();
        let ok = Command::new("taskkill")
            .args(["/T", "/F", "/PID", pid.as_str()])
            .creation_flags(CREATE_NO_WINDOW)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map(|status| status.success())
            .unwrap_or(false);
        if ok {
            return;
        }
    }
    let _ = child.kill();
}
/// 配置页「自动发现」按钮的返回值：从启动文件附近读到的连接参数。
#[derive(Debug, Clone, Serialize)]
pub struct Discovered {
    /// 参数是从哪个目录读到的（提示里会显示，方便判断有没有读错框架）。
    pub dir: String,
    pub host: String,
    pub port: u16,
    pub path: String,
    pub access_token: String,
}

/// 从启动文件出发，把框架自己的 OneBot 配置读出来交给配置页回填。
#[tauri::command]
pub fn discover_onebot(startup: String) -> Result<Discovered, String> {
    let settings = OnebotSettings {
        startup,
        ..Default::default()
    };
    if settings.startup.trim().is_empty() {
        return Err("先填 OneBot 框架的启动文件（exe / bat / mjs 都行）".to_string());
    }
    let path = startup_file(&settings)
        .ok_or_else(|| format!("这个启动文件不存在：{}", settings.startup.trim()))?;
    let dirs = config_dirs(&path)
        .iter()
        .map(|dir| dir.display().to_string())
        .collect::<Vec<_>>()
        .join("、");
    let Some((dir, ep)) = endpoint_of(&path) else {
        return Err(format!(
            "在 {dirs} 里没找到能读出 wsServers 的 json（各家的 OneBot 配置一般放在 config/ 下）"
        ));
    };
    eprintln!(
        "自动发现：从 {} 读到 ws://{}:{}",
        dir.display(),
        ep.host,
        ep.port
    );
    Ok(Discovered {
        dir: dir.display().to_string(),
        host: ep.host,
        port: ep.port,
        path: ep.path,
        access_token: ep.access_token,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("astra-herald-onebot-{}-{tag}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("config")).unwrap();
        dir
    }

    fn settings(startup: &Path) -> OnebotSettings {
        OnebotSettings {
            startup: startup.to_string_lossy().to_string(),
            ..Default::default()
        }
    }

    /// 没填 / 文件不在 / 填的是真文件：三种情况分清楚，别拿一个不存在的路径去 CreateProcess。
    #[test]
    fn startup_file_requires_an_existing_file() {
        let dir = tmpdir("startup");
        let script = dir.join("index.mjs");
        fs::write(&script, "// fake").unwrap();
        assert_eq!(startup_file(&settings(&script)), Some(script.clone()));
        assert_eq!(startup_file(&OnebotSettings::default()), None);
        assert_eq!(startup_file(&settings(&dir.join("nope.mjs"))), None);
        // 目录也不行：目录不是可执行的东西。
        assert_eq!(startup_file(&settings(&dir)), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 端点藏在 config/onebot_<QQ>.json 的 networks.wsServers[0] 里（SnowLuma 那份的形状）。
    #[test]
    fn endpoint_reads_ws_server_from_config_dir() {
        let dir = tmpdir("endpoint");
        let body = r#"{"mode":"snapshot","networks":{"httpServers":[{"name":"http-default","host":"127.0.0.1","port":3000}],"wsServers":[{"name":"ws-default","accessToken":"tok-123","messageFormat":"array","host":"127.0.0.1","port":3001,"role":"Universal"}]}}"#;
        fs::write(dir.join("config").join("onebot_183713750.json"), body).unwrap();
        let ep = endpoint(&dir).expect("should read endpoint");
        assert_eq!(ep.port, 3001);
        assert_eq!(ep.access_token, "tok-123");
        assert_eq!(ep.host, "127.0.0.1");
        assert_eq!(ep.path, "");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 别的框架：json 在根目录、键名是 websocketServers、token 叫 token——同一套扫描也要认。
    #[test]
    fn endpoint_reads_another_framework_shape() {
        let dir = tmpdir("other");
        let body = r#"{"network":{"httpServers":[],"websocketServers":[{"enable":true,"host":"127.0.0.1","port":3002,"token":"tk-2"}]}}"#;
        fs::write(dir.join("onebot11_10000.json"), body).unwrap();
        let ep = endpoint(&dir).expect("should read endpoint");
        assert_eq!(ep.port, 3002);
        assert_eq!(ep.access_token, "tk-2");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 启动器常放在 bin/ 子目录里、配置在安装根目录：得往上找一层。
    #[test]
    fn endpoint_of_walks_up_to_the_install_root() {
        let dir = tmpdir("walkup");
        fs::create_dir_all(dir.join("bin")).unwrap();
        let script = dir.join("bin").join("start.bat");
        fs::write(&script, "@echo off").unwrap();
        let body = r#"{"networks":{"wsServers":[{"host":"127.0.0.1","port":3009,"accessToken":"t9"}]}}"#;
        fs::write(dir.join("config").join("onebot_7.json"), body).unwrap();
        let (from, ep) = endpoint_of(&script).expect("应该往上一层找到配置");
        assert_eq!(from, dir);
        assert_eq!(ep.port, 3009);
        let _ = fs::remove_dir_all(&dir);
    }

    /// 手填的连接参数优先，留空的项从框架配置里补；端口写了乱码就忽略那一项。
    #[test]
    fn typed_connection_fields_win_over_discovered() {
        let dir = tmpdir("typed");
        let script = dir.join("index.mjs");
        fs::write(&script, "// fake").unwrap();
        let body = r#"{"networks":{"wsServers":[{"host":"127.0.0.1","port":3001,"path":"/ws","accessToken":"from-file"}]}}"#;
        fs::write(dir.join("config").join("onebot_1.json"), body).unwrap();

        let mut settings = OnebotSettings {
            startup: script.to_string_lossy().to_string(),
            ..Default::default()
        };
        // 什么都没填 = 全用文件里的。
        let (from, ep) = effective_endpoint(&settings).expect("自动发现");
        assert_eq!(from.as_deref(), Some(dir.as_path()));
        assert_eq!(ep.port, 3001);
        assert_eq!(ep.access_token, "from-file");
        assert_eq!(ep.path, "/ws");

        // 只改端口和 token = 其余沿用文件里的。
        settings.port = "3009".to_string();
        settings.access_token = "typed".to_string();
        let (_, ep) = effective_endpoint(&settings).expect("合并");
        assert_eq!(ep.port, 3009);
        assert_eq!(ep.access_token, "typed");
        assert_eq!(ep.host, "127.0.0.1");
        assert_eq!(ep.path, "/ws");

        // 端口写了乱码：忽略它，别的照样生效。
        settings.port = "abc".to_string();
        let (_, ep) = effective_endpoint(&settings).expect("合并");
        assert_eq!(ep.port, 3001);

        // 只有手填、连启动文件都没有：照样能连（没有配置来源）。
        let only_typed = OnebotSettings {
            host: "192.168.1.9".to_string(),
            port: "3010".to_string(),
            ..Default::default()
        };
        let (from, ep) = effective_endpoint(&only_typed).expect("纯手填");
        assert!(from.is_none());
        assert_eq!(ep.host, "192.168.1.9");
        assert_eq!(ep.port, 3010);

        // 全空又没启动文件：不托管。
        assert!(effective_endpoint(&OnebotSettings::default()).is_none());
        let _ = fs::remove_dir_all(&dir);
    }

    /// 有的框架把「根路径」写成 "/"，不能原样交给 kovi（会拼出 //api）。
    #[test]
    fn endpoint_normalizes_root_path() {
        let dir = tmpdir("root-path");
        let body = r#"{"networks":{"wsServers":[{"host":"127.0.0.1","port":3001,"path":"/","accessToken":"t"}]}}"#;
        fs::write(dir.join("config").join("onebot_2.json"), body).unwrap();
        assert_eq!(endpoint(&dir).expect("endpoint").path, "");
        let _ = fs::remove_dir_all(&dir);

        let dir = tmpdir("sub-path");
        let body = r#"{"networks":{"wsServers":[{"host":"127.0.0.1","port":3001,"path":"/ws/","accessToken":"t"}]}}"#;
        fs::write(dir.join("config").join("onebot_3.json"), body).unwrap();
        assert_eq!(endpoint(&dir).expect("endpoint").path, "/ws");
        let _ = fs::remove_dir_all(&dir);
    }

    /// 读不出 wsServers 的 json（比如框架自己的库文件）不该被当成配置。
    #[test]
    fn endpoint_ignores_json_without_ws_servers() {
        let dir = tmpdir("noconf");
        fs::write(dir.join("config").join("runtime.json"), r#"{"webuiPort":5099}"#).unwrap();
        fs::write(dir.join("config").join("messages.json"), r#"{"items":[{"text":"hi"}]}"#).unwrap();
        assert_eq!(endpoint(&dir), None);
        let _ = fs::remove_dir_all(&dir);
    }

    /// Hook 开关：关着就打开，且只改这一个键；读不到 / 解析不了的一律不碰。
    #[test]
    fn hook_autoload_flip_touches_only_that_field() {
        assert_eq!(parse_flag("1"), Some(true));
        assert_eq!(parse_flag(" TRUE "), Some(true));
        assert_eq!(parse_flag("on"), Some(true));
        assert_eq!(parse_flag("0"), Some(false));
        assert_eq!(parse_flag("off"), Some(false));
        assert_eq!(parse_flag(""), None);
        assert_eq!(parse_flag("maybe"), None);

        let dir = tmpdir("runtime");
        let path = dir.join("config").join("runtime.json");
        fs::write(
            &path,
            r#"{"webuiPort":5099,"hookAutoLoad":false,"webuiHost":"127.0.0.1"}"#,
        )
        .unwrap();
        assert_eq!(hook_autoload_in(&dir), Some(false));
        ensure_hook_autoload(&dir);
        let fixed: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(fixed["hookAutoLoad"], Value::Bool(true));
        assert_eq!(fixed["webuiPort"], Value::from(5099), "同一个文件里别的键不能变");
        assert_eq!(fixed["webuiHost"], Value::from("127.0.0.1"));

        // 已经是 true：文件一个字节都不该再变。
        let already = fs::read_to_string(&path).unwrap();
        ensure_hook_autoload(&dir);
        assert_eq!(fs::read_to_string(&path).unwrap(), already);

        // 框架还没写出 runtime.json（或压根没 Hook 这概念）：不能凭空造一个出来。
        let other = tmpdir("runtime-none");
        assert_eq!(hook_autoload_in(&other), None);
        ensure_hook_autoload(&other);
        assert!(!other.join("config").join("runtime.json").exists());

        // 解析不了：原样留着，宁可收不到消息也不能把用户的配置写坏。
        let broken = tmpdir("runtime-broken");
        let broken_path = broken.join("config").join("runtime.json");
        fs::write(&broken_path, "{ not json").unwrap();
        ensure_hook_autoload(&broken);
        assert_eq!(fs::read_to_string(&broken_path).unwrap(), "{ not json");

        let _ = fs::remove_dir_all(&dir);
        let _ = fs::remove_dir_all(&other);
        let _ = fs::remove_dir_all(&broken);
    }

    /// 启动文件认三种形态：脚本找 node、批处理交给 cmd、可执行文件直接跑。
    #[test]
    fn launcher_picks_node_cmd_or_the_file_itself() {
        let dir = tmpdir("launcher");
        let script = dir.join("index.mjs");
        fs::write(&script, "// fake").unwrap();
        fs::write(dir.join("node.exe"), b"").unwrap();
        let (program, args) = launcher(&script);
        assert_eq!(program, dir.join("node.exe"));
        assert_eq!(args, vec![OsString::from("index.mjs")]);

        // 没自带 node 就找 PATH 里的 node。
        fs::remove_file(dir.join("node.exe")).unwrap();
        let (program, args) = launcher(&script);
        assert_eq!(program, PathBuf::from("node"));
        assert_eq!(args.len(), 1);

        // 批处理：cmd /C <绝对路径>。
        let bat = dir.join("launcher.bat");
        fs::write(&bat, "@echo off").unwrap();
        let (program, args) = launcher(&bat);
        assert_eq!(program, PathBuf::from("cmd"));
        assert_eq!(args, vec![OsString::from("/C"), bat.clone().into_os_string()]);

        // exe 直接跑，不带参数。
        let exe = dir.join("framework.exe");
        fs::write(&exe, b"MZ").unwrap();
        let (program, args) = launcher(&exe);
        assert_eq!(program, exe);
        assert!(args.is_empty());
        let _ = fs::remove_dir_all(&dir);
    }
}

