<script setup>
import { nextTick, onBeforeUnmount, onMounted, ref } from "vue";
import { NConfigProvider, dateZhCN, zhCN } from "naive-ui";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { useSystemTheme } from "../theme.js";

const { isDark, theme } = useSystemTheme();
// 整张卡片的外部容器：窗口高度按它的 offsetHeight 报给后端（含画阴影用的内边距）。
const root = ref(null);
// 后端每推一条 id 就 +1；同时当 article 的 key，让复用窗口时入场动画重播一遍。
const seq = ref(0);
const text = ref("");
const sender = ref("");
const error = ref("");
// 自动收起时长由配置页决定（config.rs 的 SideSettings，3..300 秒）；拿不到就用 15 秒。
// 底部那条进度线就是按它跑完的，Rust 那边的 schedule_dismiss 用的是同一个值。
const durationMs = ref(15000);

let shownSeq = 0;
let unlisten = null;

// 卡片高度随内容变：量出来报给 Rust，window 尺寸和下面几条的位置都由它决定。
async function reportHeight() {
  await nextTick();
  const height = root.value?.offsetHeight ?? 0;
  if (height > 0) await invoke("side_resize", { height });
}

async function refreshDuration() {
  try {
    const config = await invoke("get_config");
    const secs = Number(config?.side?.duration_secs);
    if (Number.isFinite(secs) && secs > 0) durationMs.value = Math.min(Math.max(Math.round(secs), 3), 300) * 1000;
  } catch {
    // 读不到配置就用默认的 15 秒，不要因为这件事把通知卡住。
  }
}

async function apply(payload) {
  if (!payload || payload.id <= shownSeq) return;
  shownSeq = payload.id;
  await refreshDuration();
  // 等配置这段时间里可能又来了更新的消息：让更晚的那条去画，不覆盖它。
  if (payload.id !== shownSeq) return;
  seq.value = payload.id;
  text.value = payload.text;
  sender.value = payload.sender ?? "";
  error.value = "";
  reportHeight();
}

// 收起这条：后端从栈里摘掉它并把下面的补上来，窗口留着复用。
function closeSelf() {
  invoke("side_close");
}

onMounted(async () => {
  // 先挂监听、再补拉：窗口是建好之后才加载页面的，emit 可能早于监听注册。
  // 失败必须看得见：静默失败只剩一张空白卡片，连错在哪都不知道。
  try {
    // 必须带 target：不带的话 JS 侧注册的是 {kind:"Any"} 广播监听，三条侧通知窗口
    // 会收到发给别的窗口的 payload，后到的新消息把所有卡片都改成同一条文字。
    // Rust 侧用 emit_to(label) 定向发，这里就按本窗口的 label 收。
    unlisten = await listen("side:new", (event) => apply(event.payload), {
      target: getCurrentWindow().label,
    });
    await refreshDuration();
    await apply(await invoke("side_current"));
  } catch (e) {
    error.value = "初始化失败：" + String(e);
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
        :key="seq"
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
            title="关闭"
            @click="closeSelf"
          >✕</button>
        </header>

        <p class="side__text">{{ text }}</p>

        <p v-if="error" class="side__error">{{ error }}</p>

        <!-- 自动收起的倒计时，时长 = 配置页里的「侧通知显示时长」。 -->
        <span
          class="side__progress"
          :style="{ animationDuration: durationMs + 'ms' }"
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
  padding: 12px 16px 14px;
  border-radius: 10px;
  border: 1px solid rgba(128, 128, 128, 0.28);
  background: #ffffff;
  color: #1f1f1f;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.22);
  /* 原生窗口的滑入动不了（窗口整体平移），卡片自己再进来一次，看着才不生硬。 */
  animation: side-card-in 200ms cubic-bezier(0.22, 1, 0.36, 1) both;
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
  width: 9px;
  height: 9px;
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
  font-size: 15px;
  font-weight: 600;
  opacity: 0.8;
}

/* 关闭按钮常显：以前是 hover 才看得见，容易不知道能关。 */
.side__close {
  flex: none;
  width: 26px;
  height: 26px;
  padding: 0;
  border: 1px solid rgba(128, 128, 128, 0.35);
  border-radius: 50%;
  background: rgba(128, 128, 128, 0.12);
  color: inherit;
  opacity: 0.85;
  font-size: 14px;
  line-height: 1;
  cursor: pointer;
}

.side__close:hover,
.side__close:focus-visible {
  opacity: 1;
  background: rgba(128, 128, 128, 0.28);
}

/* 文字长了在卡片里滚，不要把窗口撑到 SIDE_MAX_HEIGHT 之外被裁掉。 */
.side__text {
  margin: 8px 0 0;
  max-height: 260px;
  overflow-y: auto;
  font-size: 17px;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}

.side__error {
  margin: 8px 0 0;
  font-size: 13px;
  color: #d03050;
}

.side__progress {
  position: absolute;
  left: 0;
  right: 0;
  bottom: 0;
  height: 3px;
  background: #2080f0;
  transform-origin: left center;
  animation-name: side-progress;
  animation-timing-function: linear;
  animation-fill-mode: forwards;
}

.side__card--dark .side__progress {
  background: #70c0e8;
}

@keyframes side-card-in {
  from {
    opacity: 0;
    transform: translateX(14px);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
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
