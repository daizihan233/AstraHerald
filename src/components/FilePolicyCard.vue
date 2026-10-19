<script setup>
import { NCard, NDynamicTags } from "naive-ui";

/// 文件消息策略：{ open: string[], ignore: string[] }
const policy = defineModel({ type: Object, required: true });

/// n-dynamic-tags 只吐它自己那一栏，这里换成新对象让父组件的响应跟上。
function setList(key, value) {
  policy.value = { ...policy.value, [key]: value };
}
</script>

<template>
  <n-card size="small">
    <template #header>文件消息</template>

    <p class="hint">
      按后缀名决定收到文件怎么办。「自动打开」里的后缀先下载到桌面再打开；「完全不管」里的后缀直接忽略；
      两份名单都没命中的兜底只下载到桌面。后缀不用带点，写 docx 就行。
    </p>
    <p class="hint">
      自动打开还有个前提：没有软件占着全屏或最大化，并且当前或下一节课正好是发件好友对应的
      课程——上述任一不满足时只下载到桌面。
    </p>

    <div class="lists">
      <div class="list">
        <span class="list__label">自动打开</span>
        <n-dynamic-tags
          :value="policy.open"
          aria-label="收到这些后缀的文件时自动打开"
          @update:value="(value) => setList('open', value)"
        />
      </div>
      <div class="list">
        <span class="list__label">完全不管</span>
        <n-dynamic-tags
          :value="policy.ignore"
          aria-label="收到这些后缀的文件时完全不管"
          @update:value="(value) => setList('ignore', value)"
        />
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

.lists {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.list {
  display: grid;
  grid-template-columns: 88px 1fr;
  gap: 8px;
  align-items: start;
}

.list__label {
  padding-top: 4px;
  font-size: 13px;
  opacity: 0.8;
}
</style>
