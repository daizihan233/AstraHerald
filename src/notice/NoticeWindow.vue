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
          <header v-if="group.sender || groups.length > 1" class="notice__bar">
            <span class="notice__bar-name">{{ group.sender }}</span>
            <!-- 只有一列时关掉它等于关窗，那交给右下角那颗；分栏了才需要单独收掉某一列。 -->
            <n-button
              v-if="groups.length > 1"
              size="tiny"
              quaternary
              aria-label="关闭这一列提醒"
              @click="closeColumn(group.sender)"
            >
              ✕
            </n-button>
          </header>

          <div class="notice__body">
            <article
              v-for="(message, index) in group.messages"
              :key="message.id"
              class="notice__message"
              :class="{ 'notice__message--divided': index > 0 }"
            >
              <p class="notice__text" :style="{ fontSize: fontSize + 'px' }">{{ message.text }}</p>

              <!-- 候选回复跟着它那条消息：浮动菜单指不清回哪条，一律排在消息下面。 -->
              <div v-if="message.replies.length" class="notice__replies">
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

        </section>
      </div>

      <!-- 字号是整窗共用的一份（分栏时也一样），所以整窗只留这一组按钮；✕ 收起整个提醒窗。 -->
      <n-float-button-group shape="circle" :right="20" :bottom="20">
        <!-- naive-ui 的 FloatButton 没有 disabled，只能自己表达边界 + 收键盘。 -->
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
          aria-label="关闭窗口"
          @click="closeWindow"
          @keydown.enter="closeWindow"
          @keydown.space.prevent="closeWindow"
        >
          ✕
        </n-float-button>
      </n-float-button-group>

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
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 16px 20px 14px 28px;
  border-bottom: 1px solid rgba(128, 128, 128, 0.28);
  font-size: 15px;
  font-weight: 500;
  line-height: 1.4;
  color: rgba(0, 0, 0, 0.55);
  overflow-wrap: anywhere;
}

/* 正文区：消息少时整块居中（和以前单条提醒的观感一致），多了自己滚。
   右边和底部给右下角那一组固定按钮（40px 的圆 + 20px 边距）让位，
   别让最后一行钻到按钮底下。 */
.notice__body {
  flex: 1 1 auto;
  min-height: 0;
  display: flex;
  flex-direction: column;
  justify-content: center;
  padding: 24px 88px 96px 28px;
  overflow-y: auto;
}

/* 顶栏里那一列是谁发的：长昵称会折行，右边的 ✕ 不跟着跑。 */
.notice__bar-name {
  min-width: 0;
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
