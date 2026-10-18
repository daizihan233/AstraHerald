<script setup>
import { NCard, NDynamicTags } from "naive-ui";

/// QQ 号字符串数组。
const ids = defineModel({ type: Array, required: true });

defineProps({
  title: { type: String, required: true },
  hint: { type: String, required: true },
});
</script>

<template>
  <n-card size="small">
    <template #header>{{ title }}</template>
    <n-dynamic-tags
      v-model:value="ids"
      :input-props="{ placeholder: '输入 QQ 号后按回车' }"
    />
    <p class="hint">{{ hint }}</p>
  </n-card>
</template>

<style scoped>
.hint {
  margin: 12px 0 0;
  font-size: 12px;
  line-height: 1.6;
  opacity: 0.65;
}
</style>
