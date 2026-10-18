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

/// 连接配置默认值，必须和 config.rs 的 Default for OnebotSettings 一致（全是空串）。
/// startup 空 = 不托管任何框架；其余四项空 = 每次启动都从框架自己的配置里读。
export const ONEBOT_DEFAULT = {
  startup: "",
  host: "",
  port: "",
  path: "",
  access_token: "",
};

/// 默认连接配置的副本——调用方会就地改它，不能把常量本身漏出去。
export function defaultOnebot() {
  return { ...ONEBOT_DEFAULT };
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
/// 上课时只有全屏延后至下课、其余侧通知；下课时全屏侧通知、其余强提醒。
export const DEFAULT_SCENE = {
  in_class: {
    fullscreen: "postpone",
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
    },
    files: {
      open: normalizeExtList(raw?.files?.open, FILE_POLICY_DEFAULT.open),
      ignore: normalizeExtList(raw?.files?.ignore, FILE_POLICY_DEFAULT.ignore),
    },
  };
}
