mod autostart;
pub mod bot;
mod config;
mod defer;
mod files;
mod history;
mod notice;
mod roles;
mod schedule;
mod onebot;
mod preset;
mod status;
mod update;
mod utils;
mod win;

/// 给 bot-only 这个独立二进制用：它不经过 GUI 的 run()，得自己钉工作目录。
pub use config::anchor_to_data_dir;

use config::{Config, CONFIG_PATH, DEFAULT_WEBUI_URL};
use tauri::Manager;

/// 配置页窗口的 label，必须和 tauri.conf.json 里 app.windows[0].label 一致。
const CONFIG_WINDOW_LABEL: &str = "main";

/// 连接配置窗口的 label。和配置页一样，第二次点菜单时用它找回原来的窗口。
const CONNECTION_WINDOW_LABEL: &str = "connection";

/// 聊天记录窗口的 label。同样只有一个实例：再点菜单就是叫到前台。
const HISTORY_WINDOW_LABEL: &str = "history";

/// 托盘菜单项的 id。建菜单和处理点击都用这几个常量，
/// 免得两处各写一遍字符串、某天改了一处就静默失效。
const TRAY_OPEN_CONFIG: &str = "open_config";
const TRAY_OPEN_CONNECTION: &str = "open_connection";
const TRAY_OPEN_HISTORY: &str = "open_history";
const TRAY_QUIT: &str = "quit";

/// kovi 插件入口。
///
/// `#[kovi::plugin]` 生成的 `get_plugin()` / `__kovi_build_plugin()` 内部写死了
/// `crate::__kovi_get_plugin_info()` 和 `crate::__kovi_run_async_plugin`，也就是说这个
/// 属性只能贴在 **crate 根部**；贴进 `mod bot` 就会 E0425（cannot find function
/// `__kovi_get_plugin_info` in the crate root）。所以这里只放一个薄壳，插件逻辑在
/// `bot::plugin_main`。
#[kovi::plugin]
async fn bot_plugin() {
    bot::plugin_main().await
}

/// 配置页读配置。文件不存在或解析失败时 `load_or_create` 会先落盘一份默认配置再返回。
#[tauri::command]
fn get_config() -> Result<Config, String> {
    Ok(Config::load_or_create(CONFIG_PATH))
}

/// 配置页写配置。先校验再落盘：形状不对（例如名单里混进非数字号码）直接报错，
/// 不去动磁盘上那一份。
#[tauri::command]
async fn save_config(config: Config) -> Result<(), String> {
    config.validate()?;
    config
        .save(CONFIG_PATH)
        .map_err(|e| format!("写入 {} 失败：{e}", CONFIG_PATH))?;
    // 自启状态跟着配置一起落地：调 reg.exe 会阻塞，所以这个命令是 async 的，
    // 跑在 tokio 线程池上，不会卡住窗口。
    autostart::sync(config.autostart);
    // 名单改完立刻生效：ACL 是挂在 bot 上的，只在插件启动时设一次的话，用户保存后
    // 得等下一次重连才认新名单。bot 还没连上就跳过——重连时 plugin_main 会再设一遍。
    if let Some(bot) = notice::bot_handle() {
        utils::set_access_control(bot);
    }
    // 侧通知「最多几条」改小之后立刻生效：把多出来的那几张收掉，不用等下一条消息才收敛。
    notice::apply_side_limit();
    Ok(())
}

/// 把配置页窗口叫到前台。
///
/// 窗口在关闭时只是隐藏（见 `run` 里的 CloseRequested 处理），不会被销毁，
/// 所以这里总是找得到它——找不到就是 label 对错了。
fn show_config_window(app: &tauri::AppHandle) {
    match app.get_webview_window(CONFIG_WINDOW_LABEL) {
        Some(window) => {
            let _ = window.unminimize();
            let _ = window.show();
            let _ = window.set_focus();
        }
        None => eprintln!("找不到配置页窗口 {CONFIG_WINDOW_LABEL}"),
    }
}

/// 打开「连接配置」窗口；已经开着就把它叫到前台，而不是再开一个。
///
/// 这里加载的是连接服务自己的网页端（外部 http 地址），不是本应用的前端资源，
/// 所以走 WebviewUrl::External；窗口自身不需要 IPC 权限。
fn show_connection_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    if let Some(window) = app.get_webview_window(CONNECTION_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    // 地址是配置项（配置页「连接配置」卡片里可改）；万一是填坏的，退回默认地址，别拒开窗口。
    let configured = Config::load_or_create(CONFIG_PATH).onebot.webui_url().to_string();
    let url: tauri::Url = configured.parse().or_else(|_| {
        eprintln!("连接服务网页端地址不是合法 URL（{configured}），改用 {DEFAULT_WEBUI_URL}");
        DEFAULT_WEBUI_URL.parse()
    })
    .map_err(tauri::Error::InvalidUrl)?;
    WebviewWindowBuilder::new(app, CONNECTION_WINDOW_LABEL, WebviewUrl::External(url))
        .data_directory(utils::webview_data_dir(app))
        .title("连接配置")
        .inner_size(1000.0, 760.0)
        .min_inner_size(640.0, 480.0)
        .build()?;
    Ok(())
}

/// 打开「聊天记录」窗口；已经开着就把它叫到前台，绝不开第二个。
///
/// 窗口关闭只是隐藏（下面挂了 CloseRequested），所以反复点菜单不会攒出一堆窗口，
/// 也不会因为重建窗口而丢掉滚动位置和已勾选的行。
fn show_history_window(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::{WebviewUrl, WebviewWindowBuilder};

    if let Some(window) = app.get_webview_window(HISTORY_WINDOW_LABEL) {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
        return Ok(());
    }

    let window = WebviewWindowBuilder::new(
        app,
        HISTORY_WINDOW_LABEL,
        WebviewUrl::App("history.html".into()),
    )
    .data_directory(utils::webview_data_dir(app))
    .title("聊天记录")
    .inner_size(760.0, 620.0)
    .min_inner_size(520.0, 400.0)
    .build()?;

    // 和配置页一样：关闭 = 隐藏。真销毁了就再也叫不出来（托盘是唯一入口）。
    let window_to_hide = window.clone();
    window.on_window_event(move |event| {
        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
            api.prevent_close();
            let _ = window_to_hide.hide();
        }
    });
    Ok(())
}

/// 任务栏（托盘）图标：左键点开就是这几个入口。
fn build_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;

    let open_config = MenuItem::with_id(app, TRAY_OPEN_CONFIG, "应用配置", true, None::<&str>)?;
    let open_connection =
        MenuItem::with_id(app, TRAY_OPEN_CONNECTION, "连接配置", true, None::<&str>)?;
    let open_history = MenuItem::with_id(app, TRAY_OPEN_HISTORY, "聊天记录", true, None::<&str>)?;
    // 退出是不可逆的，跟上面几个常用项之间隔一条线，少点串一次。
    let separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, TRAY_QUIT, "退出", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[&open_config, &open_connection, &open_history, &separator, &quit],
    )?;

    let mut builder = TrayIconBuilder::with_id("astraherald-tray")
        // Windows 上左键点图标直接展开菜单，不要求用户猜右键。
        .show_menu_on_left_click(true)
        .menu(&menu)
        .tooltip("AstraHerald")
        .on_menu_event(|app, event| match event.id().as_ref() {
            TRAY_OPEN_CONFIG => show_config_window(app),
            TRAY_OPEN_CONNECTION => {
                if let Err(e) = show_connection_window(app) {
                    eprintln!("打开连接配置窗口失败：{e}");
                }
            }
            TRAY_OPEN_HISTORY => {
                if let Err(e) = show_history_window(app) {
                    eprintln!("打开聊天记录窗口失败：{e}");
                }
            }
            // 窗口的「关闭」是隐藏，所以进程只能从这里结束。
            TRAY_QUIT => {
                // 只收本项目拉起的那个 OneBot 框架；用户自己起的那份不碰。
                onebot::shutdown();
                app.exit(0)
            }
            other => eprintln!("未处理的托盘菜单项：{other}"),
        });

    // Windows 下就是 embed 进 exe 的应用图标；拿不到也照样建，只是图标可能不显示。
    match app.default_window_icon().cloned() {
        Some(icon) => builder = builder.icon(icon),
        None => eprintln!("没有可用的默认图标，托盘图标可能不显示"),
    }

    builder.build(app)?;
    Ok(())
}

/// 起 Tauri 窗口，并在后台把 kovi bot 拉起来。
///
/// bot 是「一直跑」的服务：`Bot::run()` 会阻塞到连接断开（kovi/src/bot/run.rs:34），
/// 所以只能在 `setup` 里 spawn，不能在这里 await，否则窗口永远起不来。
#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // 先把工作目录钉到数据目录：配置、聊天记录、日志都是相对路径，开机自启时工作目录
    // 可能是 C:\Windows\System32（HKCU\...\Run 不设工作目录），装在只读位置时又写不进
    // 去——两种情况都在 anchor_to_data_dir 里处理（见 config.rs）。
    config::anchor_to_data_dir();

    // OwlGuard 要在整个进程期间活着：Drop 时它会关掉日志线程、丢掉缓冲区里的日志
    // （owl-logger/src/guard.rs:38 的 must_use 就是提醒这件事）。
    let _logger_guard = owl_logger::init();

    // 无人值守安装：安装器可能把命令行参数摊成 install-preset.ini 放在安装目录里
    // （scripts/sync-nsis-template.mjs 生成的 NSIS 段落），开机首次启动时并进配置再删掉。
    // 必须赶在 first_run 之前——并进去了就不算「第一次运行」，也就不会弹配置页。
    preset::apply_if_any();

    // bot 和配置页都会用 load_or_create 把缺失的配置补出来，所以「是不是第一次运行」
    // 必须在 spawn bot / 显示窗口之前问——晚一步问，文件已经被补出来了，永远是 false。
    let first_run = !std::path::Path::new(CONFIG_PATH).exists();

    tauri::Builder::default()
        // 单实例：第二个进程在这儿就被拦下——插件（命名互斥体 + WM_COPYDATA）把参数转给
        // 已经在跑的那个实例，然后自己 cleanup_before_exit + exit(0)（插件源码
        // platform_impl\windows.rs:129-130）。回调跑在**第一个**实例里，所以顺手把配置页
        // 叫出来：用户双击图标想要的就是那个窗口。
        // 必须排在其它插件之前：第二个实例得在别的 setup 干活之前就退出去（插件文档要求）。
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            show_config_window(app);
        }))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        // 自动更新：查 OSS 上的 latest.json（tauri.conf.json 的 plugins.updater），
        // 安装包用 minisign 签名，验不过就不装。
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .invoke_handler(tauri::generate_handler![
            get_config,
            save_config,
            notice::notice_current,
            notice::notice_reply,
            notice::notice_close,
            notice::notice_preview_side,
            notice::side_current,
            notice::side_resize,
            notice::side_close,
            roles::auto_fill_roles,
            roles::friend_options,
            history::history_list,
            history::history_replay,
            status::connection_status,
            onebot::discover_onebot,
            update::check_update,
            update::install_update,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // 主窗口不在 tauri.conf.json 里自动创建（那里 dataDirectory 只能写成相对 %LOCALAPPDATA% 的路径），
            // 手动建一次，好把 WebView2 的缓存也放到程序旁边——见 utils::webview_data_dir。
            if let Some(config) = app
                .config()
                .app
                .windows
                .iter()
                .find(|w| w.label == CONFIG_WINDOW_LABEL)
                .cloned()
            {
                tauri::WebviewWindowBuilder::from_config(&handle, &config)?
                    .data_directory(utils::webview_data_dir(&handle))
                    .build()?;
            }

            // 私聊提醒窗口：状态先塞进 Tauri 的全局 state，AppHandle 存一份给 bot 回调用。
            notice::init(&handle);

            // 场景识别：每 2s 问一次 AstraSchedule/desktop 的 /status（见 schedule.rs），
            // 顺带让「延后至下课」的队列看一眼现在能不能放行。
            schedule::start_watch();

            // 开机自启：让注册表跟配置对齐（开发构建会自己跳过）。
            autostart::sync(Config::load_or_create(CONFIG_PATH).autostart);

            // OneBot 框架（Hook QQ 转 OneBot 11）：用户在「连接配置」里指定启动文件后，
            // 端口没人听才由本项目拉起子进程；框架自己的配置一个字都不写（见 onebot.rs）。
            // 必须排在 bot 之前——bot 要连的就是它的 WS 端点。
            onebot::ensure_running(&Config::load_or_create(CONFIG_PATH));

            // 关闭 = 隐藏。配置页只有托盘这一条入口，窗口真被销毁就再也叫不出来了。
            if let Some(window) = app.get_webview_window(CONFIG_WINDOW_LABEL) {
                let window_to_hide = window.clone();
                window.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = window_to_hide.hide();
                    }
                });
            }

            build_tray(&handle)?;

            // 只有第一次运行（配置文件还不存在）才自动弹配置页；之后开机就待在托盘里。
            if first_run {
                show_config_window(&handle);
            }

            // 连不上会自己一直重试（bot::start 内部就是循环），这里只管起一次。
            tauri::async_runtime::spawn(bot::start());
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
