<script setup>
import { computed, onMounted, ref } from "vue";
import { NAlert, NButton, NSpin, NTag, useDialog, useMessage, useThemeVars } from "naive-ui";
import AccessListCard from "../components/AccessListCard.vue";
import ConnectionCard from "../components/ConnectionCard.vue";
import FilePolicyCard from "../components/FilePolicyCard.vue";
import QuickRepliesCard from "../components/QuickRepliesCard.vue";
import RolesCard from "../components/RolesCard.vue";
import RuntimeCard from "../components/RuntimeCard.vue";
import SceneMatrixCard from "../components/SceneMatrixCard.vue";
import SideNoticeCard from "../components/SideNoticeCard.vue";
import UpdateCard from "../components/UpdateCard.vue";
import { autoFillRoles, friendOptions, loadConfig, saveConfig } from "../api.js";
import {
  DEFAULT_AUTOSTART,
  DEFAULT_PERIOD,
  ONEBOT_WEBUI_DEFAULT,
  ROLE_OPTIONS,
  defaultFilePolicy,
  defaultOnebot,
  defaultScene,
  defaultSide,
  defaultUpdater,
  normalizeConfig,
} from "../config/schema.js";

const message = useMessage();
const dialog = useDialog();
const themeVars = useThemeVars();

const loading = ref(true);
const saving = ref(false);
const filling = ref(false);
const loadError = ref("");

// 可编辑的表单状态。磁盘上的 map 在界面上都是「数组 + 行」，方便增删和报错定位。
const quickReplies = ref([]);
const whitelist = ref([]);
const blacklist = ref([]);
const roles = ref([]);
// 好友身份下拉的候选：全量好友（value=QQ 号，label=备注 [用户名] （QQ号））。
const friendList = ref([]);
const scene = ref(defaultScene());
const defaultPeriod = ref(DEFAULT_PERIOD);
const autostart = ref(DEFAULT_AUTOSTART);
const filePolicy = ref(defaultFilePolicy());
/// 连接配置：启动文件 + 四项连接参数（留空 = 自动发现）。
const onebot = ref(defaultOnebot());
/// 侧通知：显示多久自己收起来。
const side = ref(defaultSide());
/// 更新：自动检查开关 + 更新源。
const updater = ref(defaultUpdater());

/// 最近一次读入/写出的表单快照，用来判断有没有未保存的改动。
/// 存表单而不是 toConfig() 的结果：空行、只填了一半的行也算改动，
/// 否则用户刚加了行、指示器还写「已保存」，一按保存才报错。
const savedSnapshot = ref("");

function formSnapshot() {
  return JSON.stringify({
    quickReplies: quickReplies.value,
    whitelist: whitelist.value,
    blacklist: blacklist.value,
    roles: roles.value,
    scene: scene.value,
    defaultPeriod: defaultPeriod.value,
    autostart: autostart.value,
    onebot: onebot.value,
    filePolicy: filePolicy.value,
    side: side.value,
    updater: updater.value,
  });
}

/// 表单 -> 后端 Config。trim 一遍、丢掉空行，并且始终用同一套键序，
/// 这样 JSON.stringify 才能拿来做改动比较（对象键序会影响字符串）。
function toConfig() {
  const quickRepliesObject = {};
  for (const row of quickReplies.value) {
    const keyword = row.keyword.trim();
    const replies = row.replies.map((reply) => reply.trim()).filter(Boolean);
    if (!keyword || replies.length === 0) continue;
    quickRepliesObject[keyword] = replies;
  }

  const rolesObject = {};
  for (const row of roles.value) {
    const qq = row.qq.trim();
    if (!qq || !row.role) continue;
    rolesObject[qq] = row.role;
  }

  return {
    quick_replies: quickRepliesObject,
    whitelist: whitelist.value.map((id) => id.trim()).filter(Boolean),
    blacklist: blacklist.value.map((id) => id.trim()).filter(Boolean),
    roles: rolesObject,
    scene: scene.value,
    default_period: defaultPeriod.value,
    autostart: autostart.value,
    onebot: {
      startup: onebot.value.startup.trim(),
      host: onebot.value.host.trim(),
      port: onebot.value.port.trim(),
      path: onebot.value.path.trim(),
      access_token: onebot.value.access_token.trim(),
      // 空串后端会兜底，但落盘就写清楚，免得配置文件里看着像没配。
      webui_url: onebot.value.webui_url.trim() || ONEBOT_WEBUI_DEFAULT,
    },
    files: filePolicy.value,
    // 时长和字号都给后端一个能用的整数：输入框清空时是 null，后端那边会
    // clamp 到 3..300 秒、12..40 像素。
    side: {
      duration_secs:
        Number.isFinite(Number(side.value.duration_secs)) && Number(side.value.duration_secs) > 0
          ? Math.round(Number(side.value.duration_secs))
          : 15,
      font_size_px:
        Number.isFinite(Number(side.value.font_size_px)) && Number(side.value.font_size_px) > 0
          ? Math.round(Number(side.value.font_size_px))
          : 17,
      nick_font_px:
        Number.isFinite(Number(side.value.nick_font_px)) && Number(side.value.nick_font_px) > 0
          ? Math.round(Number(side.value.nick_font_px))
          : 15,
    },
    updater: {
      enabled: Boolean(updater.value.enabled),
      source: updater.value.source.trim(),
    },
  };
}

/// 后端 Config -> 表单。
function toForm(config) {
  quickReplies.value = Object.entries(config.quick_replies).map(([keyword, replies]) => ({
    keyword,
    replies: [...replies],
  }));
  whitelist.value = [...config.whitelist];
  blacklist.value = [...config.blacklist];
  roles.value = Object.entries(config.roles).map(([qq, role]) => ({ qq, role }));
  scene.value = config.scene;
  defaultPeriod.value = config.default_period;
  autostart.value = config.autostart;
  onebot.value = { ...defaultOnebot(), ...config.onebot };
  filePolicy.value = { open: [...config.files.open], ignore: [...config.files.ignore] };
  side.value = { ...defaultSide(), ...config.side };
  updater.value = { ...defaultUpdater(), ...config.updater };
}

const dirty = computed(() => formSnapshot() !== savedSnapshot.value);

/// 和后端 Config::validate 同一套规则，先在前端拦一次，省得跑一趟 IPC 才知道填错了。
/// 必须是纯数字 QQ 号：kovi 侧非数字 ID 永远匹配不上事件里的整数 ID。
const QQ_RE = /^\d+$/;

function validate() {
  const keywords = [];
  for (const [index, row] of quickReplies.value.entries()) {
    const keyword = row.keyword.trim();
    if (!keyword) return `第 ${index + 1} 条快速回复没有填关键词`;
    if (keywords.includes(keyword)) return `关键词「${keyword}」重复了`;
    keywords.push(keyword);
    if (row.replies.every((reply) => !reply.trim())) {
      return `关键词「${keyword}」至少要有一条候选回复`;
    }
  }

  for (const [label, ids] of [
    ["白名单", whitelist.value],
    ["黑名单", blacklist.value],
  ]) {
    const seen = [];
    for (const id of ids.map((value) => value.trim()).filter(Boolean)) {
      if (!QQ_RE.test(id)) return `${label}里不是纯数字 QQ 号：${id}`;
      if (seen.includes(id)) return `${label}里 QQ 号重复：${id}`;
      seen.push(id);
    }
  }

  // 端口留空 = 自动发现；填了就得是合法端口，否则后端起不来（backend 会退回框架配置的端口，
  // 但那不是用户想要的结果，不如现在就说清楚）。
  const port = onebot.value.port.trim();
  if (port && (!/^\d+$/.test(port) || Number(port) < 1 || Number(port) > 65535)) {
    return `OneBot 端口要填 1..65535 的数字（留空就从框架配置里读）：${port}`;
  }

  // 「连接配置」要打开的地址：托盘菜单点开就是个网页窗口，填坏了那个窗口打不开。
  const webui = onebot.value.webui_url.trim() || ONEBOT_WEBUI_DEFAULT;
  if (!/^https?:\/\//.test(webui)) {
    return `连接服务网页端地址要以 http:// 或 https:// 开头：${webui}`;
  }

  const seenQq = [];
  for (const [index, row] of roles.value.entries()) {
    const qq = row.qq.trim();
    if (!qq) return `第 ${index + 1} 个好友没有填 QQ 号`;
    if (!QQ_RE.test(qq)) return `好友身份里不是纯数字 QQ 号：${qq}`;
    if (seenQq.includes(qq)) return `好友身份里 QQ 号重复：${qq}`;
    seenQq.push(qq);
    if (!row.role) return `QQ ${qq} 还没有选身份`;
  }

  return "";
}

/// 取好友列表给身份下拉用：bot 还没连上或没好友时保持空数组，不打扰用户。
async function loadFriends() {
  try {
    friendList.value = await friendOptions();
  } catch {
    friendList.value = [];
  }
}

/// 「一键配置」：让后端拉好友列表 + 课表科目，按备注名逐个匹配身份。
/// silent 给初始化用：bot 还没连上时不该一进页面就弹错误。
/// 返回是否成功，调用方自己决定要不要提示。
async function fillRoles(silent = false) {
  filling.value = true;
  try {
    const matched = await autoFillRoles();
    roles.value = Object.entries(matched).map(([qq, role]) => ({ qq, role }));
    // 能一键配置就说明 bot 通了：顺手把下拉的候选也补上。
    if (friendList.value.length === 0) await loadFriends();
    // 匹配不上的会落「课程」，所以说「填好」而不是「匹配」，免得看着像全对上了。
    if (!silent) message.success(`已填好 ${roles.value.length} 位好友的身份`);
    return true;
  } catch (error) {
    if (!silent) message.error(String(error));
    return false;
  } finally {
    filling.value = false;
  }
}

/// 「清除全部」：清空的是界面上的行，磁盘上那份要等保存才动，
/// 所以还能靠「重新加载」找回，这里只做一次确认。
function clearRoles() {
  if (roles.value.length === 0) return;
  dialog.warning({
    title: "清除全部好友身份？",
    content: "会清空下面所有行。没保存之前可以点「重新加载」找回磁盘上的那份。",
    positiveText: "清除全部",
    negativeText: "取消",
    onPositiveClick: () => {
      roles.value = [];
    },
  });
}

/// 兜底身份：后端 roles.rs 的 FALLBACK_ROLE，撞不上任何课程名时落的值。
const FALLBACK_ROLE = "课程";

/// 整份身份都是兜底值。多半是上次初始化时课表服务（AstraSchedule）还没起来。
function allFallbackRoles() {
  return (
    roles.value.length > 0 && roles.value.every((row) => row.role === FALLBACK_ROLE)
  );
}

async function reload() {
  loading.value = true;
  loadError.value = "";
  try {
    toForm(normalizeConfig(await loadConfig()));
    savedSnapshot.value = formSnapshot();
    await loadFriends();
    // 还没配过好友身份、或上次只填到兜底值「课程」时顺手重填一遍（后端会再拉几次课表）；
    // 填完就是「有未保存的修改」，由用户决定要不要保存。
    if (roles.value.length === 0 || allFallbackRoles()) await fillRoles(true);
  } catch (error) {
    loadError.value = String(error);
  } finally {
    loading.value = false;
  }
}

async function save() {
  const invalid = validate();
  if (invalid) {
    message.error(invalid);
    return;
  }

  saving.value = true;
  try {
    await saveConfig(toConfig());
    // 保存不改表单，快照直接对齐当前表单；被 trim 掉/丢掉的空行不算新改动。
    savedSnapshot.value = formSnapshot();
    message.success("已保存");
  } catch (error) {
    message.error(String(error));
  } finally {
    saving.value = false;
  }
}

function confirmReload() {
  if (!dirty.value) {
    reload();
    return;
  }
  dialog.warning({
    title: "放弃未保存的修改？",
    content: "重新加载会用磁盘上的配置覆盖当前改动。",
    positiveText: "重新加载",
    negativeText: "取消",
    onPositiveClick: () => reload(),
  });
}

onMounted(reload);

const actionsStyle = computed(() => ({
  background: themeVars.value.bodyColor,
  borderColor: themeVars.value.borderColor,
}));
</script>

<template>
  <div class="page">
    <n-alert v-if="loadError" type="error" title="读取配置失败">
      <p class="page__error">{{ loadError }}</p>
      <n-button size="small" @click="reload">重试</n-button>
    </n-alert>

    <n-spin :show="loading">
      <div class="page__cards">
        <quick-replies-card v-model="quickReplies" />
        <access-list-card
          v-model="whitelist"
          title="白名单"
          hint="白名单非空时只响应名单里的 QQ，名单外的一律不理。必须是纯数字 QQ 号。"
        />
        <access-list-card
          v-model="blacklist"
          title="黑名单"
          hint="只在白名单为空时生效：名单里的 QQ 会被忽略。"
        />
        <scene-matrix-card v-model="scene" />
        <connection-card v-model="onebot" />
        <runtime-card v-model:period="defaultPeriod" v-model:autostart="autostart" />
        <file-policy-card v-model="filePolicy" />
        <side-notice-card v-model="side" />
        <update-card v-model="updater" />
        <!-- 好友身份放在最后：一屏几十行，早先摆中间会把后面的卡片全顶下去 -->
        <roles-card
          v-model="roles"
          :options="ROLE_OPTIONS"
          :friends="friendList"
          :filling="filling"
          @auto-fill="fillRoles()"
          @clear="clearRoles"
        />
      </div>
    </n-spin>

    <div class="page__actions" :style="actionsStyle">
      <span class="page__state">
        <n-tag v-if="dirty" size="small" type="warning">有未保存的修改</n-tag>
        <n-tag v-else size="small" :bordered="false">已保存</n-tag>
      </span>
      <n-button :disabled="loading || saving" @click="confirmReload">重新加载</n-button>
      <n-button type="primary" :loading="saving" :disabled="loading" @click="save">保存</n-button>
    </div>
  </div>
</template>

<style scoped>
.page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.page__error {
  margin: 4px 0 8px;
  font-size: 12px;
  word-break: break-all;
}

.page__cards {
  display: flex;
  flex-direction: column;
  gap: 16px;
  /* 给右下角那颗浮动按钮让位，不然最后一张卡片会被它压住 */
  padding-bottom: 80px;
}

/* 浮动在右下角的操作条：页面再长也不用滚到底才能保存。
   背景必须不透明（:style 里取的是 themeVars.bodyColor），否则卡片会从底下透出来。 */
.page__actions {
  position: fixed;
  right: 24px;
  bottom: 24px;
  z-index: 10;
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 8px 8px 14px;
  border: 1px solid;
  border-radius: 999px;
  box-shadow: 0 8px 24px rgb(0 0 0 / 12%);
}

.page__state {
  margin-right: 4px;
}
</style>
