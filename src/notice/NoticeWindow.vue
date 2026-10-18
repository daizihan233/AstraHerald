<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { NButton, NConfigProvider, NFloatButton, NFloatButtonGroup, dateZhCN, zhCN } from "naive-ui";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSystemTheme } from "../theme.js";

const MIN_FONT = 16;
const MAX_FONT = 96;
const FONT_STEP = 8;

const { isDark, theme } = useSystemTheme();
// 这一段时间里收到的全部消息，按到达顺序：{ id, text, sender, replies }。
// 后端 hold 着一整段会话（notice.rs 的 NoticeState），这里每次都跟它对齐，
// 不在前端自己判断「是不是新的一段」——窗口 hide/show 与事件投递有竞态。
const messages = ref([]);
// 发送失败的原因（后端报错），显示在底部。
const replyError = ref("");
// 正在发送的那条，形如 "12|收到"；发送期间不接受第二次点击。
const sending = ref("");
// 哪一列的候选菜单展开了（存 sender；一列只有一条消息时才有菜单）。
const openMenu = ref("");
const fontSize = ref(32);
let unlisten = null;

// 同一个人发的消息合成一列：同列垂直排不重复标题栏；不同人各自一列，各自有标题栏和按钮。
const groups = computed(() => {
  const bySender = new Map();
  for (const message of messages.value) {
    if (!bySender.has(message.sender)) {
      bySender.set(message.sender, { sender: message.sender, messages: [] });
    }
    bySender.get(message.sender).messages.push(message);
  }
  return [...bySender.values()];
});

async function refresh() {
  const rest = await invoke("notice_current");
  messages.value = Array.isArray(rest) ? rest : [];
  replyError.value = "";
  sending.value = "";
  openMenu.value = "";
}

function zoomBy(delta) {
  fontSize.value = Math.min(MAX_FONT, Math.max(MIN_FONT, fontSize.value + delta));
}

// 关闭 = 隐藏：窗口留在后台待命，下一条消息 show 一下就能出来（不用重建 WebView）。
// Rust 侧也拦了 CloseRequested（notice.rs 的 init），标题栏那个 X 走的是同一条路。
function closeWindow() {
  getCurrentWindow().hide();
}

// 关掉一整列（这个发送者的所有消息）。返回剩下的消息，空了后端会把窗口收起来。
async function closeColumn(sender) {
  const rest = await invoke("notice_close", { id: null, sender });
  messages.value = Array.isArray(rest) ? rest : [];
}

async function sendReply(message, reply) {
  const key = message.id + "|" + reply;
  if (sending.value) return;
  sending.value = key;
  replyError.value = "";
  openMenu.value = "";
  try {
    const rest = await invoke("notice_reply", { id: message.id, reply });
    // 发出去了才收窗；失败就留在窗口里让用户看见原因。
    if (!rest || rest.length === 0) {
      getCurrentWindow().hide();
      return;
    }
    messages.value = rest;
  } catch (e) {
    replyError.value = String(e);
  } finally {
    sending.value = "";
  }
}

onMounted(async () => {
  // 先挂监听、再补拉：窗口是启动时就预建好的，但页面挂载和第一条消息仍可能撞上
  // （事件发出时没人接），补拉能兜住那种情况。
  unlisten = await listen("notice:new", () => { refresh(); });
  await refresh();
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
});
</script>

<template>
  <n-config-provider :theme="theme" :locale="zhCN" :date-locale="dateZhCN">
    <div class="notice" :class="{ 'notice--dark': isDark }">
      <!-- 同一人的消息合成一列：一条就单列满屏，多条就并排分栏。 -->
      <div class="notice__groups">
        <section v-for="group in groups" :key="group.sender" class="notice__group">
          <header v-if="group.sender" class="notice__bar">{{ group.sender }}</header>

          <div class="notice__body">
            <article
              v-for="(message, index) in group.messages"
              :key="message.id"
              class="notice__message"
              :class="{ 'notice__message--divided': index > 0 }"
            >
              <p class="notice__text" :style="{ fontSize: fontSize + 'px' }">{{ message.text }}</p>

              <!-- 一列里有多条消息时，浮动菜单指不清回哪条，改成每条下面并排内联按钮。 -->
              <div
                v-if="group.messages.length > 1 && message.replies.length"
                class="notice__replies"
              >
                <n-button
                  v-for="reply in message.replies"
                  :key="reply"
                  size="small"
                  secondary
                  :loading="sending === message.id + '|' + reply"
                  :aria-label="'回复：' + reply"
                  @click="sendReply(message, reply)"
                >
                  {{ reply }}
                </n-button>
              </div>
            </article>
          </div>

          <!-- 每列自己一组按钮：A+/A− 是整窗的，✕ 只关这一列。
               position=absolute 是相对自己这一列定位，分栏时几组按钮才不会叠在一起。 -->
          <n-float-button-group shape="circle" position="absolute" :right="20" :bottom="20">
            <n-float-button
              v-if="group.messages.length === 1 && group.messages[0].replies.length"
              menu-trigger="click"
              :show-menu="openMenu === group.sender"
              tabindex="0"
              :aria-label="'选择候选回复'"
              :aria-expanded="openMenu === group.sender ? 'true' : 'false'"
              @update:show-menu="(v) => { openMenu = v ? group.sender : '' }"
              @keydown.enter="openMenu = openMenu === group.sender ? '' : group.sender"
              @keydown.space.prevent="openMenu = openMenu === group.sender ? '' : group.sender"
            >
              <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true" focusable="false">
                <path
                  fill="currentColor"
                  d="M8 1.5c-3.6 0-6.5 2.3-6.5 5.2 0 1.7.9 3.2 2.4 4.2l-.5 3.6 3.3-1.7c.4.1.9.1 1.3.1 3.6 0 6.5-2.3 6.5-5.2S11.6 1.5 8 1.5z"
                />
              </svg>
              <template #menu>
                <div class="notice__menu">
                  <n-button
                    v-for="reply in group.messages[0].replies"
                    :key="reply"
                    block
                    quaternary
                    :loading="sending === group.messages[0].id + '|' + reply"
                    :aria-label="'回复：' + reply"
                    @click="sendReply(group.messages[0], reply)"
                  >
                    <span class="notice__menu-text">{{ reply }}</span>
                  </n-button>
                </div>
              </template>
            </n-float-button>

            <!-- naive-ui 的 FloatButton 没有 disabled 这个 prop，只能自己表达边界 + 收键盘。 -->
            <n-float-button
              :class="{ 'is-limit': fontSize >= MAX_FONT }"
              :aria-disabled="fontSize >= MAX_FONT"
              tabindex="0"
              aria-label="放大字号"
              @click="zoomBy(FONT_STEP)"
              @keydown.enter="zoomBy(FONT_STEP)"
              @keydown.space.prevent="zoomBy(FONT_STEP)"
            >
              A+
            </n-float-button>
            <n-float-button
              :class="{ 'is-limit': fontSize <= MIN_FONT }"
              :aria-disabled="fontSize <= MIN_FONT"
              tabindex="0"
              aria-label="缩小字号"
              @click="zoomBy(-FONT_STEP)"
              @keydown.enter="zoomBy(-FONT_STEP)"
              @keydown.space.prevent="zoomBy(-FONT_STEP)"
            >
              A−
            </n-float-button>
            <n-float-button
              type="error"
              tabindex="0"
              :aria-label="groups.length > 1 ? '关闭这一列提醒' : '关闭窗口'"
              @click="groups.length > 1 ? closeColumn(group.sender) : closeWindow()"
              @keydown.enter="groups.length > 1 ? closeColumn(group.sender) : closeWindow()"
              @keydown.space.prevent="groups.length > 1 ? closeColumn(group.sender) : closeWindow()"
            >
              ✕
            </n-float-button>
          </n-float-button-group>
        </section>
      </div>

      <p v-if="replyError" class="notice__reply-error">{{ replyError }}</p>
    </div>
  </n-config-provider>
</template>

<style>
html,
body {
  margin: 0;
}

.notice {
  box-sizing: border-box;
  height: 100vh;
  overflow: hidden;
  background: #fff;
  color: rgba(0, 0, 0, 0.9);
}

/* 一列 = 一个发送者。多列时横向并排，中间一条竖分割线。 */
.notice__groups {
  display: flex;
  height: 100%;
}

.notice__group {
  position: relative;
  flex: 1 1 0;
  min-width: 0;
  display: flex;
  flex-direction: column;
}

.notice__group + .notice__group {
  border-left: 1px solid rgba(128, 128, 128, 0.28);
}

/* 顶栏：这一列是谁发的。同一列多条消息也只出现一次。 */
.notice__bar {
  flex: none;
  padding: 16px 28px 14px;
  border-bottom: 1px solid rgba(128, 128, 128, 0.28);
  font-size: 15px;
  font-weight: 500;
  line-height: 1.4;
  color: rgba(0, 0, 0, 0.55);
  overflow-wrap: anywhere;
}

/* 正文区：消息少时整块居中（和以前单条提醒的观感一致），多了自己滚。
   右边留 88px 给固定按钮列（40px 宽 + 20px 右距 + 一点间隙）；
   底部留 228px 给最多 4 个圆叠起来（4*40 + 3*16 + 20）。 */
.notice__body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 24px 88px 228px 28px;
  overflow-y: auto;
}

/* 同一列里的第二条起：画一条横分割线，免得两条消息粘在一起读不出边界。 */
.notice__message--divided {
  margin-top: 20px;
  padding-top: 20px;
  border-top: 1px solid rgba(128, 128, 128, 0.28);
}

.notice__message {
  flex: none;
}

.notice--dark {
  background: rgb(16, 16, 20);
  color: rgba(255, 255, 255, 0.9);
}

.notice--dark .notice__bar {
  color: rgba(255, 255, 255, 0.55);
}

.notice--dark .notice__group + .notice__group,
.notice--dark .notice__message--divided {
  border-color: rgba(255, 255, 255, 0.14);
}

/* 字号到头了：按钮还在，但不该装成还能按。 */
.is-limit {
  opacity: 0.45;
  cursor: default;
}

.notice__text {
  margin: 0;
  font-weight: 600;
  line-height: 1.4;
  /* 保留消息里的换行；长串（URL、连续数字）也要能断行。 */
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}

/* 一列多条消息时的内联候选回复，排在各自那条消息下面。 */
.notice__replies {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-top: 12px;
}

/* 候选回复菜单：跟着气泡按钮一起进 group，右缘对齐按钮右缘（默认会往右溢出窗口）。 */
.notice .n-float-button__menu {
  right: 0;
}

/* 选择器带 .notice 是为了盖过 naive-ui 那条「.n-float-button__menu > * { margin-bottom:16px }」，
   否则菜单底部会多出 16px，在 320px 高的窗口里正好把第一项顶出屏幕。 */
.notice .notice__menu {
  display: flex;
  flex-direction: column;
  /* 和 group 里四个圆的间距保持一致。 */
  gap: 16px;
  min-width: 120px;
  /* 触发按钮上方真正可用的高度：窗口高 - 按钮列 248 - 顶栏那一圈 40。 */
  max-height: max(140px, calc(100vh - 288px));
  margin-bottom: 0;
  overflow-y: auto;
  padding: 8px;
  border: 1px solid rgba(128, 128, 128, 0.28);
  border-radius: 8px;
  background: #fff;
  box-shadow: 0 6px 20px rgba(0, 0, 0, 0.18);
}

.notice--dark .notice__menu {
  border-color: rgba(255, 255, 255, 0.14);
  background: rgb(28, 28, 32);
}

.notice__menu-text {
  display: block;
  max-width: 200px;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 发送失败：贴在底部偏左，给右边那一列按钮让出位置（右留 96px）。 */
.notice__reply-error {
  position: fixed;
  left: 0;
  right: 96px;
  bottom: 24px;
  margin: 0;
  padding: 0 0 0 28px;
  font-size: 13px;
  color: rgb(208, 48, 80);
}

.notice--dark .notice__reply-error {
  color: rgb(255, 122, 148);
}
</style>
