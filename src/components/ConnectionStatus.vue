<script setup>
import { NTag, NTooltip } from "naive-ui";
import { computed, onBeforeUnmount, onMounted, ref } from "vue";
import { connectionStatus } from "../api.js";

/// 轮询周期：课表那边每 2s 轮询一次，这里 3s 取一次就够跟得上，不会明显滞后。
const POLL_MS = 3000;

/// 后端 connection_status 的返回：{ onebot: {ok, detail}, schedule: {ok, detail}, period }。
/// 拿不到（IPC 出错）就整块显示「未知」——不猜。
const status = ref(null);
let timer = null;

async function refresh() {
  try {
    status.value = await connectionStatus();
  } catch {
    status.value = null;
  }
}

onMounted(() => {
  refresh();
  timer = setInterval(refresh, POLL_MS);
});

onBeforeUnmount(() => {
  if (timer) clearInterval(timer);
});

/// 灯的文案就是状态本身（已连接 / 未连接 / 未知），不靠颜色也能读懂；
/// 为什么连不上放在 tooltip 里（端口、配置来自哪个目录）。
const links = computed(() => {
  const onebot = status.value?.onebot;
  const schedule = status.value?.schedule;
  const unknown = "还没取到状态";
  return [
    {
      key: "onebot",
      label: onebot ? (onebot.ok ? "OneBot 已连接" : "OneBot 未连接") : "OneBot 未知",
      type: onebot ? (onebot.ok ? "success" : "error") : "default",
      detail: onebot?.detail ?? unknown,
    },
    {
      key: "schedule",
      label: schedule
        ? ["课表", schedule.ok ? "已连接" : "未连接", status.value.period].join(" · ")
        : "课表 未知",
      type: schedule ? (schedule.ok ? "success" : "error") : "default",
      detail: schedule?.detail ?? unknown,
    },
  ];
});
</script>

<template>
  <div class="conn">
    <n-tooltip v-for="link in links" :key="link.key" trigger="hover">
      <template #trigger>
        <n-tag :type="link.type" size="small" round :bordered="false">
          {{ link.label }}
        </n-tag>
      </template>
      {{ link.detail }}
    </n-tooltip>
  </div>
</template>

<style scoped>
.conn {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-wrap: wrap;
}
</style>
