<script setup>
import { computed } from "vue";
import { NButton, NCard, NEmpty, NSelect } from "naive-ui";

/// 每项：{ qq: string, role: string }
const rows = defineModel({ type: Array, required: true });

const props = defineProps({
  /// 身份可选值由外部数据源提供，暂时是空数组。
  options: { type: Array, default: () => [] },
  /// 好友候选项 [{ value: QQ号, label: 备注 [用户名] （QQ号） }]，由「连接」后的 bot 提供。
  friends: { type: Array, default: () => [] },
  /// 「一键配置」请求进行中，用来转圈。
  filling: { type: Boolean, default: false },
});

const emit = defineEmits(["auto-fill", "clear"]);

/// 下拉选项：好友列表 + 已经配过但不在列表里的号（换了号、或者 bot 还没连上）。
/// 后一种得留着，否则界面会把人的 QQ 号显示成一行数字、还看不出哪儿不对。
const friendOptions = computed(() => {
  const known = new Set(props.friends.map((item) => item.value));
  const extra = rows.value
    .map((row) => row.qq.trim())
    .filter((qq) => qq && !known.has(qq))
    .map((qq) => ({ value: qq, label: `${qq}（不在好友列表里）` }));
  return [...props.friends, ...extra];
});

function addRow() {
  rows.value = [...rows.value, { qq: "", role: "" }];
}

function removeRow(index) {
  rows.value = rows.value.filter((_, i) => i !== index);
}
</script>

<template>
  <n-card size="small">
    <template #header>好友身份</template>
    <template #header-extra>
      <span class="actions">
        <n-button size="small" :loading="filling" @click="emit('auto-fill')">一键配置</n-button>
        <n-button size="small" :disabled="filling || rows.length === 0" @click="emit('clear')">
          清除全部
        </n-button>
        <n-button size="small" @click="addRow">添加好友</n-button>
      </span>
    </template>

    <p class="hint">
      身份按好友来配：左边从好友列表里挑人（下拉显示「备注 [用户名] （QQ号）」，存进配置的只有 QQ 号），
      右边选身份。「一键配置」会拉取好友列表，用备注名匹配课表里的课程名，匹配不上的落「课程」；
      填完是一份未保存的改动，确认没问题再按保存。
    </p>

    <n-empty v-if="rows.length === 0" description="还没有给好友设置身份">
      <template #extra>
        <n-button size="small" @click="addRow">添加第一个好友</n-button>
      </template>
    </n-empty>

    <div v-else class="rows">
      <div v-for="(row, index) in rows" :key="index" class="row">
        <n-select
          v-model:value="row.qq"
          :options="friendOptions"
          :aria-label="`${row.qq || '好友'}的 QQ 号`"
          filterable
          clearable
          placeholder="从好友列表里选"
        />
        <n-select
          v-model:value="row.role"
          :options="options"
          :aria-label="`${row.qq || '好友'}的身份`"
          filterable
          tag
          clearable
          placeholder="选择或输入身份"
        />
        <n-button quaternary circle aria-label="删除这个好友" @click="removeRow(index)">
          ✕
        </n-button>
      </div>
    </div>
  </n-card>
</template>

<style scoped>
.hint {
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.6;
  opacity: 0.65;
}

.actions {
  display: flex;
  gap: 8px;
}

.rows {
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.row {
  display: grid;
  grid-template-columns: 1fr 1fr auto;
  gap: 8px;
  align-items: center;
}
</style>