<script setup>
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { NButton, NCheckbox, NConfigProvider, NEmpty, dateZhCN, zhCN } from "naive-ui";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { historyList, historyReplay } from "../api.js";
import { useSystemTheme } from "../theme.js";

const { theme } = useSystemTheme();

// 后端只留最近 500 条，最新的在前（history.rs 的 list）。
const entries = ref([]);
// 选中的 id 集合。用不可变替换而不是原地改，省得和 Vue 的追踪玩猜谜。
const selected = ref(new Set());
const status = ref("");
const busy = ref(false);
let unlisten = null;

const allChecked = computed(
  () => entries.value.length > 0 && selected.value.size === entries.value.length,
);

async function refresh() {
  try {
    const rest = await historyList();
    entries.value = Array.isArray(rest) ? rest : [];
    // 记录可能因为上限被裁掉，别把已经不在列表里的 id 留着算进「全选」。
    const alive = new Set(entries.value.map((entry) => entry.id));
    selected.value = new Set([...selected.value].filter((id) => alive.has(id)));
  } catch (e) {
    status.value = String(e);
  }
}

function toggle(id, checked) {
  const next = new Set(selected.value);
  if (checked) next.add(id);
  else next.delete(id);
  selected.value = next;
  status.value = "";
}

function toggleAll() {
  if (allChecked.value) {
    selected.value = new Set();
    return;
  }
  selected.value = new Set(entries.value.map((entry) => entry.id));
}

// 重放：后端走的是和真收到消息完全相同的路径（同一套场景矩阵），
// 已经在提醒窗口里显示着的那条会被去重跳过，所以条数可能少于选中的数量。
async function replay() {
  const ids = [...selected.value].sort((a, b) => a - b);
  if (ids.length === 0 || busy.value) return;
  busy.value = true;
  try {
    const shown = await historyReplay(ids);
    const skipped = ids.length - shown;
    status.value =
      skipped > 0
        ? "已重新展示 " + shown + " 条（另外 " + skipped + " 条已经在提醒窗口里）"
        : "已重新展示 " + shown + " 条";
    selected.value = new Set();
  } catch (e) {
    status.value = String(e);
  } finally {
    busy.value = false;
  }
}

function formatTime(at) {
  const date = new Date(at * 1000);
  const pad = (value) => String(value).padStart(2, "0");
  return (
    pad(date.getMonth() + 1) +
    "-" +
    pad(date.getDate()) +
    " " +
    pad(date.getHours()) +
    ":" +
    pad(date.getMinutes())
  );
}

function closeWindow() {
  getCurrentWindow().hide();
}

onMounted(async () => {
  // 窗口是复用的（关闭只是隐藏）：每次重新叫到前台都补一次，别让用户看着旧列表。
  unlisten = await getCurrentWindow().onFocusChanged(({ payload }) => {
    if (payload) refresh();
  });
  await refresh();
});

onBeforeUnmount(() => {
  if (unlisten) unlisten();
});
</script>

<template>
  <n-config-provider :theme="theme" :locale="zhCN" :date-locale="dateZhCN">
    <div class="history">
      <header class="history__bar">
        <div class="history__meta">
          <h1 class="history__title">聊天记录</h1>
          <span class="history__count">共 {{ entries.length }} 条</span>
        </div>
        <div class="history__actions">
          <n-button size="small" quaternary :disabled="entries.length === 0" @click="toggleAll">
            {{ allChecked ? "取消全选" : "全选" }}
          </n-button>
          <n-button
            size="small"
            type="primary"
            :loading="busy"
            :disabled="selected.size === 0"
            @click="replay"
          >
            显示选中（{{ selected.size }}）
          </n-button>
          <n-button size="small" quaternary @click="closeWindow">关闭</n-button>
        </div>
      </header>

      <p v-if="status" class="history__status">{{ status }}</p>

      <div v-if="entries.length === 0" class="history__empty">
        <n-empty description="还没有聊天记录。收到的文字消息会自动记在这里。" />
      </div>

      <ul v-else class="history__list">
        <li v-for="entry in entries" :key="entry.id" class="history__row">
          <n-checkbox
            :checked="selected.has(entry.id)"
            :aria-label="`选中这条来自 ${entry.name} 的消息`"
            @update:checked="(checked) => toggle(entry.id, checked)"
          />
          <div class="history__body">
            <div class="history__head">
              <span class="history__sender">{{ entry.name }}</span>
              <time class="history__time">{{ formatTime(entry.at) }}</time>
            </div>
            <p class="history__text">{{ entry.text }}</p>
          </div>
        </li>
      </ul>
    </div>
  </n-config-provider>
</template>

<style scoped>
.history {
  box-sizing: border-box;
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--n-color, #fff);
  color: var(--n-text-color, #1f1f1f);
}

.history__bar {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  flex-wrap: wrap;
  padding: 14px 18px;
  border-bottom: 1px solid rgb(128 128 128 / 24%);
}

.history__meta {
  display: flex;
  align-items: baseline;
  gap: 10px;
}

.history__title {
  margin: 0;
  font-size: 17px;
  font-weight: 700;
}

.history__count {
  font-size: 12px;
  opacity: 0.6;
}

.history__actions {
  display: flex;
  align-items: center;
  gap: 8px;
}

.history__status {
  margin: 0;
  padding: 8px 18px;
  font-size: 12px;
  color: #18a058;
}

.history__empty {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 24px;
}

.history__list {
  flex: 1;
  margin: 0;
  padding: 0;
  list-style: none;
  overflow-y: auto;
}

.history__row {
  display: flex;
  align-items: flex-start;
  gap: 10px;
  padding: 12px 18px;
  border-bottom: 1px solid rgb(128 128 128 / 14%);
}

.history__body {
  flex: 1;
  min-width: 0;
}

.history__head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: 12px;
}

.history__sender {
  font-size: 13px;
  font-weight: 600;
}

.history__time {
  font-size: 12px;
  opacity: 0.55;
  white-space: nowrap;
}

.history__text {
  margin: 4px 0 0;
  font-size: 14px;
  line-height: 1.5;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
}
</style>
