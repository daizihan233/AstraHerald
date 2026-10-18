<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { NButton, NConfigProvider, dateZhCN, zhCN } from "naive-ui";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { useSystemTheme } from "../theme.js";

// 自动收起的时间。必须和 notice.rs 里的 SIDE_DURATION_MS 一致：
// 底部那条进度线就是按它跑完的，改了这边忘了那边，线跑完了卡片还在。
const DURATION_MS = 15000;

const { isDark, theme } = useSystemTheme();
// 整张卡片的外部容器：窗口高度按它的 offsetHeight 报给后端（含画阴影用的内边距）。
const root = ref(null);
// 后端每推一条 id 就 +1；它同时是进度线动画的 key，复用窗口时能把动画重跑一遍。
const seq = ref(0);
const text = ref("");
const sender = ref("");
const replies = ref([]);
const replyError = ref("");
const sending = ref("");

let shownSeq = 0;
let unlisten = null;

// 卡片高度随内容变：量出来报给 Rust，window 尺寸和下面几条的位置都由它决定。
async function reportHeight() {
  await nextTick();
  const height = root.value?.offsetHeight ?? 0;
  if (height > 0) await invoke("side_resize", { height });
}

function apply(payload) {
  if (!payload || payload.id <= shownSeq) return;
  shownSeq = payload.id;
  seq.value = payload.id;
  text.value = payload.text;
  sender.value = payload.sender ?? "";
  replies.value = Array.isArray(payload.replies) ? payload.replies : [];
  replyError.value = "";
  sending.value = "";
  reportHeight();
}

async function sendReply(reply) {
  if (sending.value) return;
  sending.value = reply;
  replyError.value = "";
  try {
    await invoke("side_reply", { reply });
    // 发出去了才收卡片；失败就留在这儿让人看见原因。
    await invoke("side_close");
  } catch (e) {
    replyError.value = String(e);
  } finally {
    sending.value = "";
  }
}

// 收起这条：后端从栈里摘掉它并把下面的补上来，窗口留着复用。
function closeSelf() {
  invoke("side_close");
}

onMounted(async () => {
  // 先挂监听、再补拉：窗口是建好之后才加载页面的，emit 可能早于监听注册。
  // 失败必须看得见：静默失败只剩一张空白卡片，连错在哪都不知道。
  try {
    unlisten = await listen("side:new", (event) => apply(event.payload));
    apply(await invoke("side_current"));
  } catch (e) {
    replyError.value = "初始化失败：" + String(e);
  }
  await reportHeight();
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
});
</script>

<template>
  <n-config-provider :theme="theme" :locale="zhCN" :date-locale="dateZhCN">
    <div ref="root" class="side">
      <article
        class="side__card"
        :class="{ 'side__card--dark': isDark }"
        role="status"
        aria-live="polite"
      >
        <header class="side__head">
          <span class="side__badge" aria-hidden="true"></span>
          <span class="side__sender">{{ sender || "新消息" }}</span>
          <button
            class="side__close"
            type="button"
            aria-label="关闭这条侧通知"
            @click="closeSelf"
          >✕</button>
        </header>

        <p class="side__text">{{ text }}</p>

        <p v-if="replyError" class="side__error">{{ replyError }}</p>

        <div v-if="replies.length" class="side__actions">
          <n-button
            v-for="reply in replies"
            :key="reply"
            size="tiny"
            secondary
            :loading="sending === reply"
            :aria-label="'回复：' + reply"
            @click="sendReply(reply)"
          >{{ reply }}</n-button>
        </div>

        <!-- 自动收起的倒计时。duration 来自上面的 DURATION_MS（和 Rust 的 SIDE_DURATION_MS 对齐），
             :key="seq" 让复用的窗口重新跑一遍这条动画。 -->
        <span
          :key="seq"
          class="side__progress"
          :style="{ animationDuration: DURATION_MS + 'ms' }"
          aria-hidden="true"
        ></span>
      </article>
    </div>
  </n-config-provider>
</template>

<style>
/* 窗口是透明的（notice.rs 的 ensure_side_window），圆角外的部分必须让出来。 */
html,
body {
  margin: 0;
  background: transparent;
  overflow: hidden;
}
</style>

<style scoped>
/* 这 10px 是给卡片阴影留的，量高度时也算在内，所以和窗口之间不会露出白边。 */
.side {
  padding: 10px;
  font-family: system-ui, -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
}

.side__card {
  position: relative;
  overflow: hidden;
  padding: 10px 14px 12px;
  border-radius: 10px;
  border: 1px solid rgba(128, 128, 128, 0.28);
  background: #ffffff;
  color: #1f1f1f;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.22);
}

.side__card--dark {
  background: #202020;
  color: #f5f5f5;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.55);
}

.side__head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.side__badge {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: #2080f0;
  flex: none;
}

.side__card--dark .side__badge {
  background: #70c0e8;
}

.side__sender {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 13px;
  font-weight: 600;
  opacity: 0.75;
}

.side__close {
  flex: none;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  border-radius: 50%;
  background: transparent;
  color: inherit;
  opacity: 0.6;
  font-size: 12px;
  line-height: 1;
  cursor: pointer;
}

.side__close:hover,
.side__close:focus-visible {
  opacity: 1;
  background: rgba(128, 128, 128, 0.2);
}

/* 文字长了在卡片里滚，不要把窗口撑到 SIDE_MAX_HEIGHT 之外被裁掉。 */
.side__text {
  margin: 6px 0 0;
  max-height: 260px;
  overflow-y: auto;
  font-size: 14px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
}

.side__error {
  margin: 8px 0 0;
  font-size: 12px;
  color: #d03050;
}

.side__actions {
  display: flex;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 10px;
}

.side__progress {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 2px;
  background: #2080f0;
  transform-origin: left center;
  animation-name: side-progress;
  animation-timing-function: linear;
  animation-fill-mode: forwards;
}

.side__card--dark .side__progress {
  background: #70c0e8;
}

@keyframes side-progress {
  from {
    transform: scaleX(1);
  }
  to {
    transform: scaleX(0);
  }
}
</style>
