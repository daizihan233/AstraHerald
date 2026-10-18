/**
 * 配置文件的结构：默认值、场景矩阵的行列定义，以及把读到的配置补成可编辑的形状。
 *
 * 这里的 JSON 形状必须和 src-tauri/src/config.rs 的 Config / SceneSettings / SceneRow /
 * SceneAction 对齐——后端保存时会用 serde 再解析一遍，对不上会直接报错。
 */

/// 场景矩阵的一列：是否在上课。对应 SceneSettings 的字段名。
export const SCENE_PERIODS = [
  { key: "in_class", label: "上课" },
  { key: "after_class", label: "下课" },
];

/// 场景矩阵的一行：窗口状态。对应 SceneRow 的字段名。
export const SCENE_STATES = [
  { key: "fullscreen", label: "全屏" },
  { key: "maximized", label: "最大化" },
  { key: "unfocused", label: "非桌面焦点" },
  { key: "desktop", label: "桌面" },
];

/// 格子里可选的提醒方式。对应 SceneAction 的 snake_case 取值。
export const SCENE_ACTIONS = [
  { key: "strong", label: "强提醒" },
  { key: "side", label: "侧通知" },
  { key: "postpone", label: "延后至下课" },
  // 只记进聊天记录，不弹任何提醒：全屏看片、投屏开会的时候用这一格。
  { key: "ignore", label: "忽略" },
];

/// 「默认状态」配置项的选项。就是场景矩阵的两列，复用同一份键名定义。
export const DEFAULT_PERIOD_OPTIONS = SCENE_PERIODS.map(({ key, label }) => ({
  label,
  value: key,
}));

/// 连不上课表服务时假定的状态，必须和 config.rs 的 Default for ScenePeriod 一致。
export const DEFAULT_PERIOD = "in_class";

/// 开机自启默认开，必须和 config.rs 的 Default for Config 一致：
/// 这是个要一直挂着的消息托管程序，开机不拉起来等于没装。
export const DEFAULT_AUTOSTART = true;

/// 托盘「连接配置」默认打开的地址（框架自己的网页端），必须和 config.rs 的
/// DEFAULT_WEBUI_URL 一致。后端把这个字段空串也当默认值，所以这里填不填都对。
export const ONEBOT_WEBUI_DEFAULT = "http://127.0.0.1:5099";

/// 连接配置默认值，必须和 config.rs 的 Default for OnebotSettings 一致（连接四项全是空串）。
/// startup 空 = 不托管任何框架；四项空 = 每次启动都从框架自己的配置里读；
/// webui_url 只有「打开哪个网页端」这一个用途，空串后端会兜底成默认值。
export const ONEBOT_DEFAULT = {
  startup: "",
  host: "",
  port: "",
  path: "",
  access_token: "",
  webui_url: ONEBOT_WEBUI_DEFAULT,
};

/// 默认连接配置的副本——调用方会就地改它，不能把常量本身漏出去。
export function defaultOnebot() {
  return { ...ONEBOT_DEFAULT };
}

/// 侧通知默认设置，必须和 config.rs 的 Default for SideSettings 一致：
/// 显示 15 秒、正文 17 像素、昵称 15 像素、最多同时挂 3 条。后端会 clamp 到
/// 3..300 秒、12..40 像素、1..10 条，界面上几个数字输入框用同一区间。
/// 昵称和正文分开调，昵称默认比正文小一号。
export const SIDE_DEFAULT = { duration_secs: 15, font_size_px: 17, nick_font_px: 15, max_count: 3 };

/// 默认侧通知设置的副本——调用方会就地改它，不能把常量本身漏出去。
export function defaultSide() {
  return { ...SIDE_DEFAULT };
}

/// 自动更新默认值，必须和 config.rs 的 Default for UpdaterSettings 一致：
/// 默认开、更新源留空（= 用 tauri.conf.json 里编译进去的那个地址）。
export const UPDATER_DEFAULT = { enabled: true, source: "" };

/// 默认自动更新设置的副本——调用方会就地改它，不能把常量本身漏出去。
export function defaultUpdater() {
  return { ...UPDATER_DEFAULT };
}

/// 文件消息的后缀名策略默认值，必须和 config.rs 的 Default for FilePolicy 一致：
/// 文档默认自动打开（上课时那位任课老师发来的才真打开，见 files.rs），
/// 截图默认完全不管，其余后缀兜底只下载到桌面。
export const FILE_POLICY_DEFAULT = {
  open: ["pptx", "ppt", "docx", "doc"],
  ignore: ["jpg", "png", "jpeg", "webp"],
};

/// 默认策略的副本——调用方会就地改它，不能把常量本身漏出去。
export function defaultFilePolicy() {
  return {
    open: [...FILE_POLICY_DEFAULT.open],
    ignore: [...FILE_POLICY_DEFAULT.ignore],
  };
}

/// 好友身份的可选值由外部数据源提供，这里先留空：选择框开了 tag，
/// 空列表时会退化成自由输入，接上数据源后自动变成下拉建议。
export const ROLE_OPTIONS = [];

/// 「延后至下课」只在「上课」这一列有意义（见 config.rs 的 SceneAction::Postpone）。
export function sceneActionsFor(period) {
  return period === "in_class"
    ? SCENE_ACTIONS
    : SCENE_ACTIONS.filter((action) => action.key !== "postpone");
}

/// 产品定的默认场景，必须和 config.rs 的 Default for SceneSettings 一致：
/// 上课时全屏忽略（只记聊天记录，不弹）、其余侧通知；下课时全屏侧通知、其余强提醒。
export const DEFAULT_SCENE = {
  in_class: {
    fullscreen: "ignore",
    maximized: "side",
    unfocused: "side",
    desktop: "side",
  },
  after_class: {
    fullscreen: "side",
    maximized: "strong",
    unfocused: "strong",
    desktop: "strong",
  },
};

/// 默认场景的副本——调用方会就地改它，不能把常量本身漏出去。
export function defaultScene() {
  return {
    in_class: { ...DEFAULT_SCENE.in_class },
    after_class: { ...DEFAULT_SCENE.after_class },
  };
}

/**
 * 把读到的配置补成完整、可编辑的形状。
 *
 * 缺字段补默认值；「下课 + 延后至下课」这种非法组合退回未选择
 * （后端不该产生，但读到旧文件/手改过的文件时要能兜住，不然选择框会显示一个不存在的选项）。
 */
/// 后缀名列表归一化：去空格、去开头的点、转小写、去重、丢掉空项。
/// 字段整个缺失（老配置、或后端还没补出这个字段）→ 用默认值；
/// 显式给了数组（哪怕是空数组，表示「一个都不打开」）→ 尊重它。
function normalizeExtList(value, fallback) {
  if (!Array.isArray(value)) return [...fallback];
  const seen = [];
  for (const item of value) {
    const ext = String(item ?? "").trim().replace(/^\.+/, "").toLowerCase();
    if (!ext || seen.includes(ext)) continue;
    seen.push(ext);
  }
  return seen;
}

export function normalizeConfig(raw) {
  const scene = defaultScene();
  for (const period of SCENE_PERIODS) {
    const allowed = new Set(sceneActionsFor(period.key).map((action) => action.key));
    for (const state of SCENE_STATES) {
      const value = raw?.scene?.[period.key]?.[state.key];
      // 整个字段缺失（老配置、或后端还没补出 scene）→ 保留默认值；
      // 显式为 null（用户在界面上清空了）或非法取值 → 退回未选择。
      if (value === undefined) continue;
      scene[period.key][state.key] = allowed.has(value) ? value : null;
    }
  }
  const periodKeys = new Set(SCENE_PERIODS.map((period) => period.key));
  const defaultPeriod = periodKeys.has(raw?.default_period)
    ? raw.default_period
    : DEFAULT_PERIOD;

  return {
    quick_replies: raw?.quick_replies ?? {},
    whitelist: raw?.whitelist ?? [],
    blacklist: raw?.blacklist ?? [],
    roles: raw?.roles ?? {},
    scene,
    default_period: defaultPeriod,
    // 旧配置（或后端还没补出这个字段）按默认「开」；显式 false 要保住。
    autostart: raw?.autostart === undefined ? DEFAULT_AUTOSTART : Boolean(raw.autostart),
    // 四项连接参数一律当字符串处理：后端 port 也是 String（空 = 自动发现），
    // 老配置里没有这个字段时按空串（= 自动发现）补齐。
    onebot: {
      startup: String(raw?.onebot?.startup ?? ""),
      host: String(raw?.onebot?.host ?? ""),
      port: raw?.onebot?.port === undefined || raw?.onebot?.port === null ? "" : String(raw.onebot.port),
      path: String(raw?.onebot?.path ?? ""),
      access_token: String(raw?.onebot?.access_token ?? ""),
      // 托盘「连接配置」打开的地址；留空 = 默认（后端也一样兜底）。空串归一成默认值，
      // 免得输入框一片空白让人以为没配置。
      webui_url: String(raw?.onebot?.webui_url ?? "") || ONEBOT_WEBUI_DEFAULT,
    },
    files: {
      open: normalizeExtList(raw?.files?.open, FILE_POLICY_DEFAULT.open),
      ignore: normalizeExtList(raw?.files?.ignore, FILE_POLICY_DEFAULT.ignore),
    },
    side: {
      // 读到垃圾（手改坏了）就用默认值：输入框不接受 NaN。
      duration_secs: Number.isFinite(Number(raw?.side?.duration_secs))
        ? Number(raw.side.duration_secs)
        : SIDE_DEFAULT.duration_secs,
      // 字号同理：读不到（老配置）就 17，读到也先夹到 12..40（后端还会再夹一次）。
      font_size_px: Number.isFinite(Number(raw?.side?.font_size_px))
        ? Math.min(Math.max(Math.round(Number(raw.side.font_size_px)), 12), 40)
        : SIDE_DEFAULT.font_size_px,
      // 昵称字号同理：老配置里没有这一项就 15（以前是正文的 0.88 倍）。
      nick_font_px: Number.isFinite(Number(raw?.side?.nick_font_px))
        ? Math.min(Math.max(Math.round(Number(raw.side.nick_font_px)), 12), 40)
        : SIDE_DEFAULT.nick_font_px,
      // 最多同时展示几条：老配置里没有这一项就 3，读到也先夹到 1..10（后端还会再夹一次）。
      max_count: Number.isFinite(Number(raw?.side?.max_count))
        ? Math.min(Math.max(Math.round(Number(raw.side.max_count)), 1), 10)
        : SIDE_DEFAULT.max_count,
    },
    updater: {
      enabled:
        raw?.updater?.enabled === undefined
          ? UPDATER_DEFAULT.enabled
          : Boolean(raw.updater.enabled),
      source: String(raw?.updater?.source ?? ""),
    },
  };
}
