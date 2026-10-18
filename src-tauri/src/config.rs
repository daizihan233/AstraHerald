use kovi::event::id::ID;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// 配置文件名。bot 侧（bot.rs / utils/mod.rs）和配置页的读写命令共用这一个常量，
/// 免得两边各写各的路径、读写到不同的文件。
pub const CONFIG_PATH: &str = "config.json";

/// 把工作目录锚到「数据目录」，配置、聊天记录、日志都跟着它走。
///
/// 两个坑，都在这一处解决：
///
/// ① Windows 不替我们定工作目录。从开始菜单启动是 exe 目录，但开机自启走的是
/// `HKCU\...\Run`，那时工作目录是 explorer 的（通常是 `C:\Windows\System32`）——
/// 配置会写到一个没人读的地方，日志也建不出来，等于每次开机都是全新状态。
///
/// ② 程序可能被放在只读位置（Program Files、只读共享、受控文件夹）。那里写不进
/// 任何东西，配置会静默丢失。这时退到 `%APPDATA%\AstraHerald`。
///
/// 相对路径全都跟着工作目录走，所以这里把工作目录钉对，其它代码一个字都不用改。
pub fn anchor_to_data_dir() {
    let dir = std::env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(Path::to_path_buf));
    let Some(dir) = dir else {
        eprintln!("取不到可执行文件路径，工作目录保持原样");
        return;
    };

    let target = if writable(&dir) {
        dir
    } else {
        let Some(fallback) = appdata_dir() else {
            eprintln!(
                "{} 不可写，也找不到 %APPDATA%，工作目录保持原样",
                dir.display()
            );
            return;
        };
        eprintln!(
            "{} 不可写（比如装在 Program Files），配置、聊天记录和日志改用 {}",
            dir.display(),
            fallback.display()
        );
        fallback
    };

    if let Err(e) = std::env::set_current_dir(&target) {
        eprintln!("切到数据目录失败：{}：{e}", target.display());
    }
}

/// 这里能不能建文件。真写一个探针文件比解析 ACL 靠谱，也顺带覆盖只读共享、
/// 受控文件夹访问这些查不出来的情况。
fn writable(dir: &Path) -> bool {
    let probe = dir.join(".astra-herald-write-probe");
    let ok = fs::write(&probe, b"").is_ok();
    if ok {
        let _ = fs::remove_file(&probe);
    }
    ok
}

/// 退路：`%APPDATA%\AstraHerald`（用户配置该待的地方，必然可写）。
fn appdata_dir() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("APPDATA")?).join("AstraHerald");
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

#[derive(Debug, Clone, Serialize, Deserialize)]
// 容器级 default：旧配置文件里没有的新字段补默认值，而不是整份解析失败。
// 这条很关键——load_or_create 解析失败时会直接用默认配置覆盖磁盘文件，
// 少一个字段就等于把用户配置清空一遍。
#[serde(default)]
pub struct Config {
    pub quick_replies: HashMap<String, Vec<String>>,
    pub whitelist: Vec<ID>,
    pub blacklist: Vec<ID>,
    pub roles: HashMap<String, String>,
    pub scene: SceneSettings,
    /// 连不上 AstraSchedule 时假定的上课状态（判定见 `schedule.rs`）。
    pub default_period: ScenePeriod,
    /// 开机自启（注册表 HKCU\\...\\Run，见 `autostart.rs`）。
    /// 默认开：这是个要一直挂着的消息托管程序，开机不拉起来等于没装。
    pub autostart: bool,
    /// 外部 OneBot 框架（QQ Hook -> OneBot 11 网关）的托管设置，见 onebot.rs。
    pub onebot: OnebotSettings,
    /// 收到文件消息时的后缀名策略，见 files.rs。
    pub files: FilePolicy,
    /// 侧通知的显示设置，见 notice.rs。
    pub side: SideSettings,
    /// 自动更新设置，见 update.rs。
    pub updater: UpdaterSettings,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            quick_replies: HashMap::from([
                ("收到请回复".to_string(), vec!["收到".to_string()])
            ]),
            whitelist: Vec::new(),
            blacklist: Vec::new(),
            roles: HashMap::new(),
            scene: SceneSettings::default(),
            default_period: ScenePeriod::default(),
            autostart: true,
            onebot: OnebotSettings::default(),
            files: FilePolicy::default(),
            side: SideSettings::default(),
            updater: UpdaterSettings::default(),
        }
    }
}

/// 侧通知（从屏幕右边滑进来的那条）的显示设置。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SideSettings {
    /// 显示多少秒后自动收起。太短看不清、太长会一直挂在屏幕上，两头都夹一下。
    pub duration_secs: u64,
}

impl Default for SideSettings {
    fn default() -> Self {
        Self { duration_secs: 15 }
    }
}

impl SideSettings {
    /// 秒数夹到 3..=300：写 0 会瞬间消失、写 100000 等于永不收起。
    pub const MIN_SECS: u64 = 3;
    pub const MAX_SECS: u64 = 300;

    pub fn clamped_secs(&self) -> u64 {
        self.duration_secs.clamp(Self::MIN_SECS, Self::MAX_SECS)
    }
}

/// 自动更新设置（tauri-plugin-updater，签名见 tauri.conf.json 的 plugins.updater）。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct UpdaterSettings {
    /// 启动时自动查一次更新。默认开；关掉只影响「自动查」，配置页里手动点检查还是能查。
    pub enabled: bool,
    /// 更新源（latest.json 的地址）。留空 = 用内置的官方源。
    /// 给国内网络或自建镜像留的口子：填了自己的源就完全按自己的源查。
    pub source: String,
}

impl Default for UpdaterSettings {
    fn default() -> Self {
        Self {
            enabled: true,
            source: String::new(),
        }
    }
}

/// 外部 OneBot 框架的托管设置。
///
/// 本项目不绑定任何具体的 Hook 框架：用户指一个「启动文件」（exe / mjs / bat 都行），
/// 我们从这个文件的位置去找它自己的 OneBot 配置（大部分框架都放在 config/ 下的 json 里），
/// 读出 WS 地址和 access token，再把它当子进程拉起来——见 onebot.rs。
///
/// 空串 = 不托管：只按 kovi.conf.toml / 默认的 127.0.0.1:3001 去连，用户自己起的框架照样能用。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct OnebotSettings {
    /// OneBot 框架的启动文件完整路径：exe / bat / mjs 都行（`D:\\SnowLuma\\launcher.bat` 这种）。
    pub startup: String,
    /// 下面四项是「连接配置」卡片里的手填值。留空 = 该项自动发现（从启动文件附近读框架自己的配置）；
    /// 四项都空 = 完全不指定，只按 kovi.conf.toml / 默认的 127.0.0.1:3001 去连。
    pub host: String,
    /// 端口，"" 或不是数字时用框架配置里的。
    pub port: String,
    pub path: String,
    pub access_token: String,
}

/// 文件消息策略：三类桶，后缀名小写、不带点。
///
/// open   = 下载后按需自动打开（还要看是不是这位老师当前的课，以及有没有全屏软件）
/// ignore = 完全不管
/// 剩下的   = 兜底：只下载到桌面
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct FilePolicy {
    pub open: Vec<String>,
    pub ignore: Vec<String>,
}

impl Default for FilePolicy {
    fn default() -> Self {
        let list = |items: &[&str]| items.iter().map(|s| s.to_string()).collect::<Vec<String>>();
        Self {
            open: list(&["pptx", "ppt", "docx", "doc"]),
            ignore: list(&["jpg", "png", "jpeg", "webp"]),
        }
    }
}

/// 场景矩阵：`{上课 | 下课}` × `{窗口状态}` → 该格子的提醒方式。
///
/// 键名与配置页 `src/config/schema.js` 的 `SCENE_PERIODS` / `SCENE_STATES` 一一对应，
/// 改名字要两边一起改。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneSettings {
    /// 上课时
    pub in_class: SceneRow,
    /// 下课时
    pub after_class: SceneRow,
}

/// 产品定的默认场景：上课时只有全屏要延后至下课、其余一律侧通知；
/// 下课时全屏侧通知、其余强提醒。
///
/// 这里不能省成 derive(Default)——那会给出八个空格子，等于让用户第一次
/// 打开配置页面对一整张「请选择」。前端 src/config/schema.js 的 DEFAULT_SCENE
/// 是同一份数据，改一边必须改另一边。
impl Default for SceneSettings {
    fn default() -> Self {
        use SceneAction::{Postpone, Side, Strong};
        Self {
            in_class: SceneRow {
                fullscreen: Some(Postpone),
                maximized: Some(Side),
                unfocused: Some(Side),
                desktop: Some(Side),
            },
            after_class: SceneRow {
                fullscreen: Some(Side),
                maximized: Some(Strong),
                unfocused: Some(Strong),
                desktop: Some(Strong),
            },
        }
    }
}

/// 一种上课状态下的四种窗口状态；`None` 表示这一格还没配。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SceneRow {
    /// 全屏
    pub fullscreen: Option<SceneAction>,
    /// 最大化
    pub maximized: Option<SceneAction>,
    /// 非桌面焦点
    pub unfocused: Option<SceneAction>,
    /// 桌面
    pub desktop: Option<SceneAction>,
}

/// 一个格子里可选的提醒方式；serde 用 snake_case 落盘，和前端传的字符串一致。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SceneAction {
    /// 强提醒
    Strong,
    /// 侧通知
    Side,
    /// 延后至下课。只对 `SceneSettings::in_class` 有定义。
    Postpone,
}

impl SceneAction {
    pub fn label(self) -> &'static str {
        match self {
            SceneAction::Strong => "强提醒",
            SceneAction::Side => "侧通知",
            SceneAction::Postpone => "延后至下课",
        }
    }
}

/// 上课状态——场景矩阵的列，也是「默认状态」配置项的取值。
///
/// 实时状态来自 AstraSchedule/desktop 的 `/status`（见 `schedule.rs`）；
/// 连不上时用 `Config::default_period`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScenePeriod {
    /// 上课
    InClass,
    /// 下课
    AfterClass,
}

impl ScenePeriod {
    pub fn label(self) -> &'static str {
        match self {
            ScenePeriod::InClass => "上课",
            ScenePeriod::AfterClass => "下课",
        }
    }
}

/// 默认按「上课」：连不上课表时宁可多提醒（上课那几格是侧通知/全屏延后），
/// 也不要因为当成「下课」而漏掉老师上课期间发来的消息。
impl Default for ScenePeriod {
    fn default() -> Self {
        Self::InClass
    }
}

/// 前台窗口状态——场景矩阵的行。判定见 `win.rs`。
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowState {
    /// 全屏
    Fullscreen,
    /// 最大化
    Maximized,
    /// 非桌面焦点
    Unfocused,
    /// 桌面
    Desktop,
}

impl WindowState {
    pub fn label(self) -> &'static str {
        match self {
            WindowState::Fullscreen => "全屏",
            WindowState::Maximized => "最大化",
            WindowState::Unfocused => "非桌面焦点",
            WindowState::Desktop => "桌面",
        }
    }
}

impl SceneSettings {
    pub fn row(&self, period: ScenePeriod) -> &SceneRow {
        match period {
            ScenePeriod::InClass => &self.in_class,
            ScenePeriod::AfterClass => &self.after_class,
        }
    }
}

impl SceneRow {
    pub fn action(&self, state: WindowState) -> Option<SceneAction> {
        match state {
            WindowState::Fullscreen => self.fullscreen,
            WindowState::Maximized => self.maximized,
            WindowState::Unfocused => self.unfocused,
            WindowState::Desktop => self.desktop,
        }
    }
}

impl Config {
    /// 矩阵查表：这一格配了什么就用什么。
    ///
    /// 空格子（`None`）按强提醒兜底——提醒窗口的配置页允许清空单元格，
    /// 而「悄悄不提醒」是最坏的兜底：用户会以为程序坏了。
    pub fn current_action(&self, period: ScenePeriod, state: WindowState) -> SceneAction {
        self.scene
            .row(period)
            .action(state)
            .unwrap_or(SceneAction::Strong)
    }
}

impl Config {
    pub fn load_or_create(path: &str) -> Self {
        if let Ok(content) = fs::read_to_string(path) {
            match serde_json::from_str::<Config>(&content) {
                Ok(config) => return config,
                Err(e) => {
                    // 解析失败说明这份文件是坏的，但坏文件也是用户的输入：先挪成 .bak 再重写，
                    // 别把用户手写的名单/回复直接抹掉（挪不动才退化成覆盖，日志里也说明了）。
                    eprintln!("配置文件解析失败：{e}");
                    let backup = format!("{path}.bak");
                    match fs::rename(path, &backup) {
                        Ok(()) => eprintln!("坏掉的配置已备份到 {backup}，下面按默认配置重写"),
                        Err(e) => eprintln!("备份 {path} 失败（{e}），仍会按默认配置重写"),
                    }
                }
            }
        } else {
            eprintln!("配置文件不存在");
        }

        eprintln!("使用默认配置并写入 {path}");
        let config = Config::default();
        if let Err(e) = config.save(path) {
            eprintln!("写入配置文件失败：{e}");
        }
        config
    }

    pub fn save(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let json = serde_json::to_string_pretty(self)?;
        fs::write(path, json)?;
        Ok(())
    }

    /// 写入前的最低校验：名单和身份里的号码必须是纯数字。
    ///
    /// 非数字号码会被 `utils/acl.rs` 的 `normalize_id` 原样留成 `ID::Str`，
    /// 而事件侧的 ID 是整数、相等判断又是变体严格相等（kovi id.rs:120-128 的 `_ => false`），
    /// 于是这条名单永远命中不了。宁可在写入时报错，也别让它静默失效。
    pub fn validate(&self) -> Result<(), String> {
        let mut bad: Vec<String> = self
            .whitelist
            .iter()
            .chain(self.blacklist.iter())
            .filter(|id| !is_numeric_id(id))
            .map(|id| id.to_string())
            .collect();
        bad.extend(
            self.roles
                .keys()
                .filter(|qq| qq.parse::<i64>().is_err())
                .cloned(),
        );
        if bad.is_empty() {
            Ok(())
        } else {
            Err(format!("不是纯数字 QQ 号：{}", bad.join("、")))
        }
    }
}

fn is_numeric_id(id: &ID) -> bool {
    // ID 从配置反序列化出来几乎总是 Str 变体；Int 只是防御性覆盖。
    id.try_as_i64().is_some() || id.try_as_str().is_some_and(|s| s.parse::<i64>().is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_config_is_backed_up_before_rewrite() {
        let path = std::env::temp_dir().join(format!(
            "astra-herald-corrupt-{}-{}.json",
            std::process::id(),
            "backup"
        ));
        let path = path.to_string_lossy().to_string();
        let _ = fs::remove_file(format!("{path}.bak"));
        fs::write(&path, "{ 这不是 json").unwrap();

        let config = Config::load_or_create(&path);

        // 默认配置照常可用（坏文件不影响启动）
        assert!(config.autostart);
        // 坏文件原样挪到 .bak，内容一个字节都没丢
        let backup = format!("{path}.bak");
        assert_eq!(fs::read_to_string(&backup).unwrap(), "{ 这不是 json");
        // 原路径被重写成一份能解析的默认配置
        let fresh: Config = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert!(fresh.whitelist.is_empty());

        let _ = fs::remove_file(&path);
        let _ = fs::remove_file(&backup);
    }

    #[test]
    fn writable_rejects_a_directory_that_is_not_there() {
        // 探针写不进去 = 不可写。装到只读位置时走的就是这条分支。
        assert!(writable(&std::env::temp_dir()));
        assert!(!writable(&std::env::temp_dir().join("astra-herald-not-a-dir-xyz")));
    }

    #[test]
    fn default_config_round_trips() {
        let back: Config = serde_json::from_str(&serde_json::to_string(&Config::default()).unwrap()).unwrap();
        assert_eq!(back.scene.in_class.fullscreen, Some(SceneAction::Postpone));
        assert_eq!(back.scene.after_class.desktop, Some(SceneAction::Strong));
        assert_eq!(back.quick_replies, Config::default().quick_replies);
        assert!(back.autostart);
        assert_eq!(back.files.open, vec!["pptx", "ppt", "docx", "doc"]);
        assert_eq!(back.files.ignore, vec!["jpg", "png", "jpeg", "webp"]);
        assert!(back.onebot.startup.is_empty());
    }

    /// 老配置文件（没有 scene / roles）必须能解析出默认值：
    /// 解析失败会让 load_or_create 拿默认配置覆盖磁盘文件，等于丢配置。
    #[test]
    fn config_missing_new_fields_uses_defaults() {
        let legacy = r#"{"quick_replies":{"hi":["你好"]},"whitelist":["3791513967"],"blacklist":[]}"#;
        let config: Config = serde_json::from_str(legacy).unwrap();
        assert!(config.roles.is_empty());
        // 老配置没有 scene 字段 → 补产品默认场景，而不是一片空白。
        assert_eq!(config.scene.after_class.desktop, Some(SceneAction::Strong));
        // 也没有 default_period → 默认按「上课」。
        assert_eq!(config.default_period, ScenePeriod::InClass);
        // 新加的自启开关在旧配置里也该补成「开」。
        assert!(config.autostart);
        // 文件策略与 OneBot 托管同样要能补默认值，否则老配置一解析就整份丢。
        assert!(config.files.open.contains(&"docx".to_string()));
        assert!(config.onebot.startup.is_empty());
        assert_eq!(config.quick_replies["hi"], vec!["你好".to_string()]);
    }

    /// 默认场景是产品拍板的，改这里等于改用户第一次看到的配置，必须显式确认。
    #[test]
    fn scene_defaults_match_spec() {
        let scene = SceneSettings::default();
        assert_eq!(scene.in_class.fullscreen, Some(SceneAction::Postpone));
        assert_eq!(scene.in_class.maximized, Some(SceneAction::Side));
        assert_eq!(scene.in_class.unfocused, Some(SceneAction::Side));
        assert_eq!(scene.in_class.desktop, Some(SceneAction::Side));
        assert_eq!(scene.after_class.fullscreen, Some(SceneAction::Side));
        assert_eq!(scene.after_class.maximized, Some(SceneAction::Strong));
        assert_eq!(scene.after_class.unfocused, Some(SceneAction::Strong));
        assert_eq!(scene.after_class.desktop, Some(SceneAction::Strong));
    }

    #[test]
    fn scene_actions_are_snake_case() {
        let row: SceneRow = serde_json::from_str(
            r#"{"fullscreen":"postpone","maximized":"strong","desktop":null}"#,
        )
        .unwrap();
        assert_eq!(row.fullscreen, Some(SceneAction::Postpone));
        assert_eq!(row.maximized, Some(SceneAction::Strong));
        assert_eq!(row.desktop, None);
        assert_eq!(serde_json::to_string(&SceneAction::Side).unwrap(), "\"side\"");
    }

    /// 矩阵查表：配了的按配置走，空格子按强提醒兜底（不能静默不提醒）。
    #[test]
    fn current_action_reads_matrix_and_falls_back_to_strong() {
        let mut config = Config::default();
        assert_eq!(
            config.current_action(ScenePeriod::InClass, WindowState::Fullscreen),
            SceneAction::Postpone
        );
        assert_eq!(
            config.current_action(ScenePeriod::AfterClass, WindowState::Desktop),
            SceneAction::Strong
        );
        // 用户把这一格清空了 → 强提醒，而不是「不提醒」。
        config.scene.in_class.fullscreen = None;
        assert_eq!(
            config.current_action(ScenePeriod::InClass, WindowState::Fullscreen),
            SceneAction::Strong
        );
    }

    /// 枚举字符串要同时跟前端 schema.js 的 key 和落盘的 JSON 对上。
    #[test]
    fn period_and_state_are_snake_case() {
        assert_eq!(serde_json::to_string(&ScenePeriod::InClass).unwrap(), "\"in_class\"");
        assert_eq!(serde_json::to_string(&ScenePeriod::AfterClass).unwrap(), "\"after_class\"");
        assert_eq!(serde_json::to_string(&WindowState::Unfocused).unwrap(), "\"unfocused\"");
        let period: ScenePeriod = serde_json::from_str("\"in_class\"").unwrap();
        assert_eq!(period, ScenePeriod::InClass);
        assert_eq!(ScenePeriod::default(), ScenePeriod::InClass);
        // 落盘形状里 default_period 就在顶层。
        let json = serde_json::to_string(&Config::default()).unwrap();
        assert!(json.contains("\"default_period\":\"in_class\""), "{json}");
    }

    #[test]
    fn validate_rejects_non_numeric_ids() {
        let mut config = Config::default();
        config.whitelist.push(serde_json::from_str("\"3791513967\"").unwrap());
        assert!(config.validate().is_ok());

        config.whitelist.push(serde_json::from_str("\"abc\"").unwrap());
        assert!(config.validate().is_err());

        config.whitelist.clear();
        config.roles.insert("not-a-qq".to_string(), "管理员".to_string());
        assert!(config.validate().is_err());
    }
}
