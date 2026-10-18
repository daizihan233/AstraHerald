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
// 后端每推一条 id 就 +1；同时当正文和进度条的 key，让入场动画和倒计时都重播一遍。
const seq = ref(0);
const text = ref("");
const sender = ref("");
const error = ref("");
// 入场动画只给「本窗口这一轮的第一条」：整张卡从窗口右沿外滑进来（Rust 那边窗口已经摆在
// 终点不动，滑入全靠 CSS）。复用窗口换卡不能整张卡重播这个动画 —— 起始帧是 opacity:0，
// 而窗口是透明的，换卡瞬间这块区域会整块变透明（就是之前那个「闪一下」）。
const entering = ref(false);
// 整张卡的 key：只在「本窗口新的一轮」+1，让入场动画重播（换卡时不重播）。
const cardKey = ref(0);
// 收起时先播退场动画（整张卡淡出 + 往屏幕外挪一点），播完后端才隐藏窗口。
const leaving = ref(false);
// 换卡时把上一条内容当残影留在卡片里淡出，新内容原地淡入：卡片底色一帧都不会透明。
const ghost = ref(null);
// 本窗口下一份内容是不是「新的一轮」（还没显示过、或者刚收起过）：是就直接走入场动画。
let fresh = true;
let ghostTimer = 0;
let exitTimer = 0;

/// 退场/残影动画的时长（毫秒）。EXIT_MS 必须和后端推迟隐藏窗口的时间一致（notice.rs 的 SIDE_EXIT_MS）。
const EXIT_MS = 200;
const GHOST_MS = 220;
// 显示参数随负载一起下来（config.rs 的 SideSettings）：自动收起时长、正文和昵称字号。
// 底部那条进度线按 durationMs 跑完，Rust 那边的 schedule_dismiss 用的是同一个值；
// 正文字号设成 CSS 变量 --side-font，卡片里的尺寸都按 em 跟着它走；昵称是另一个
// 变量 --side-nick-font，不跟着正文缩放（配置页两行分开填）。默认值只在负载缺字段时兜底。
const durationMs = ref(15000);
const fontPx = ref(17);
const nickFontPx = ref(15);

let shownSeq = 0;
let unlisten = null;
let unlistenExit = null;

// 卡片高度随内容变：量出来报给 Rust，window 尺寸和下面几条的位置都由它决定。
async function reportHeight() {
  await nextTick();
  const height = root.value?.offsetHeight ?? 0;
  if (height > 0) await invoke("side_resize", { height });
}

/// 真的把内容画上去（不管动画怎么编排）。
function render(payload) {
  shownSeq = payload.id;
  seq.value = payload.id;
  text.value = payload.text;
  sender.value = payload.sender ?? "";
  const px = Number(payload.font_px);
  if (Number.isFinite(px) && px > 0) fontPx.value = px;
  const nickPx = Number(payload.nick_font_px);
  if (Number.isFinite(nickPx) && nickPx > 0) nickFontPx.value = nickPx;
  const ms = Number(payload.duration_ms);
  if (Number.isFinite(ms) && ms > 0) durationMs.value = ms;
  error.value = "";
  reportHeight();
}

/// 画一条通知。负载里就带着要用的字号和时长，不必再去问一次配置：
/// 配置页预览用的可能就是输入框里还没保存的值。
///
/// 换卡不是「旧内容瞬间没了、新内容再淡入」，而是旧内容先淡出（.side__ghost 残影）
/// 同时新内容淡入：卡片底色始终在，整块区域一帧都不会露出透明窗口。
function apply(payload) {
  if (!payload || payload.id <= shownSeq) return;
  // 正在播退场时来了新内容（这条窗口被复用）：新的立刻赢，卡片当场变回不透明。
  leaving.value = false;
  if (fresh) {
    // 本窗口新的一轮（第一次显示，或者刚收起过）：整张卡从右沿滑进来。
    fresh = false;
    entering.value = true;
    cardKey.value += 1;
    ghost.value = null;
    render(payload);
    return;
  }
  if (shownSeq !== 0) {
    ghost.value = { text: text.value, sender: sender.value, error: error.value };
    clearTimeout(ghostTimer);
    ghostTimer = setTimeout(() => {
      ghost.value = null;
    }, GHOST_MS);
  }
  render(payload);
}

/// 后端要收起这一条（自动到点 / 点 ✕ / 被上限挤掉）：先播退场，后端随后隐藏窗口。
function playExit() {
  clearTimeout(ghostTimer);
  ghost.value = null;
  if (leaving.value) return;
  leaving.value = true;
  clearTimeout(exitTimer);
  exitTimer = setTimeout(() => {
    // 窗口这时候已经被隐藏了：下一份内容当作新的一轮，重新走入场动画。
    leaving.value = false;
    fresh = true;
  }, EXIT_MS);
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
    unlistenExit = await listen("side:exit", () => playExit(), {
      target: getCurrentWindow().label,
    });
    await apply(await invoke("side_current"));
  } catch (e) {
    error.value = "初始化失败：" + String(e);
  }
  await reportHeight();
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
  if (unlistenExit) unlistenExit();
  clearTimeout(ghostTimer);
  clearTimeout(exitTimer);
});
</script>

<template>
  <n-config-provider :theme="theme" :locale="zhCN" :date-locale="dateZhCN">
    <div
      ref="root"
      class="side"
      :style="{ '--side-font': fontPx + 'px', '--side-nick-font': nickFontPx + 'px' }"
    >
      <article
        :key="cardKey"
        class="side__card"
        :class="{
          'side__card--dark': isDark,
          'side__card--in': entering,
          'side__card--out': leaving,
        }"
        role="status"
        aria-live="polite"
      >
        <!-- 上一条内容的残影：换卡时原地淡出，不占高度，不挡点击。 -->
        <div v-if="ghost" class="side__ghost" aria-hidden="true">
          <header class="side__head">
            <span class="side__badge"></span>
            <span class="side__sender">{{ ghost.sender || "新消息" }}</span>
            <span class="side__close side__close--ghost">✕</span>
          </header>
          <p class="side__text">{{ ghost.text }}</p>
          <p v-if="ghost.error" class="side__error">{{ ghost.error }}</p>
        </div>

        <div :key="'body-' + seq" class="side__body">
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
        </div>

        <!-- 自动收起的倒计时，时长 = 配置页里的「侧通知显示时长」。 -->
        <span
          :key="'progress-' + seq"
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
  /* 正文字号来自配置页（config.rs 的 SideSettings::font_size_px）：
     卡片里其它尺寸都写成 em，改一次字号整张卡片等比缩放。 */
  font-size: var(--side-font, 17px);
  font-family: system-ui, -apple-system, "Segoe UI", "Microsoft YaHei", sans-serif;
}

.side__card {
  position: relative;
  overflow: hidden;
  padding: 0.7em 0.94em 0.82em;
  border-radius: 0.6em;
  border: 1px solid rgba(128, 128, 128, 0.28);
  background: #ffffff;
  color: #1f1f1f;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.22);
}

/* 入场动画只给本窗口的第一条：Rust 那边已经把窗口摆在终点不动了，卡片自己从窗口右沿外面滑进来
   （早先是逐帧挪原生窗口，低配机上会一顿一顿的）。只动 transform/opacity，走合成层。
   复用窗口换卡不能加这个：起始帧 opacity:0 + translateX(100%) 会让这块区域整块变透明。 */
.side__card--in {
  animation: side-card-in 260ms cubic-bezier(0.22, 1, 0.36, 1) both;
}

/* 收起：整张卡淡出 + 往屏幕外挪一点。时长不能超过后端推迟隐藏窗口的时间
   （notice.rs 的 SIDE_EXIT_MS），否则动画播到一半窗口就没了。 */
.side__card--out {
  animation: side-card-out 200ms ease both;
  pointer-events: none;
}

/* 换卡时上一条内容的残影：绝对定位在卡片里淡出，不占高度（高度按新内容算），
   新内容同时原地淡入。整块区域任何一帧都有底色盖着。 */
.side__ghost {
  position: absolute;
  inset: 0;
  padding: 0.7em 0.94em 0.82em;
  pointer-events: none;
  animation: side-ghost-out 220ms ease both;
}

/* 残影里的关闭按钮只是个占位，别让它滚起来或者被点。 */
.side__close--ghost {
  display: flex;
  align-items: center;
  justify-content: center;
}

.side__ghost .side__text {
  overflow: hidden;
}

/* 新内容入场：从右沿淡入（和残影同方向，像被推上来的一张）。 */
.side__body {
  animation: side-body-in 220ms cubic-bezier(0.22, 1, 0.36, 1) both;
}

.side__card--dark {
  background: #202020;
  color: #f5f5f5;
  box-shadow: 0 6px 18px rgba(0, 0, 0, 0.55);
}

.side__head {
  display: flex;
  align-items: center;
  gap: 0.47em;
}

.side__badge {
  width: 0.53em;
  height: 0.53em;
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
  /* 昵称字号是独立配置项（SideSettings::nick_font_px）：不跟着正文的 em 缩放。 */
  font-size: var(--side-nick-font, 15px);
  font-weight: 600;
  opacity: 0.8;
}

/* 关闭按钮常显：以前是 hover 才看得见，容易不知道能关。 */
.side__close {
  flex: none;
  width: 1.53em;
  height: 1.53em;
  padding: 0;
  border: 1px solid rgba(128, 128, 128, 0.35);
  border-radius: 50%;
  background: rgba(128, 128, 128, 0.12);
  color: inherit;
  opacity: 0.85;
  font-size: 0.82em;
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
  margin: 0.47em 0 0;
  /* 15em：字号多大都大约滚 9 行左右，别把窗口撑到 SIDE_MAX_HEIGHT 之外被裁掉。 */
  max-height: 15em;
  overflow-y: auto;
  font-size: 1em;
  line-height: 1.6;
  white-space: pre-wrap;
  word-break: break-word;
}

.side__error {
  margin: 0.47em 0 0;
  font-size: 0.76em;
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
    /* 位移量就是卡片自己的宽度：起点正好在窗口右沿外面，像是从屏幕边上抽出来。 */
    opacity: 0;
    transform: translateX(100%);
  }
  to {
    opacity: 1;
    transform: translateX(0);
  }
}

@keyframes side-card-out {
  to {
    opacity: 0;
    transform: translateX(0.6em);
  }
}

@keyframes side-ghost-out {
  to {
    opacity: 0;
    transform: translateX(0.6em);
  }
}

@keyframes side-body-in {
  from {
    opacity: 0;
    transform: translateX(0.6em);
  }
  to {
    opacity: 1;
    transform: none;
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
