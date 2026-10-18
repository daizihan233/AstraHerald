<script setup>
import { NButton, NCard, NEmpty, NInput } from "naive-ui";

/// 每项：{ keyword: string, replies: string[] }
const rows = defineModel({ type: Array, required: true });

function addRow() {
  rows.value = [...rows.value, { keyword: "", replies: [""] }];
}

function removeRow(index) {
  rows.value = rows.value.filter((_, i) => i !== index);
}

function addReply(row) {
  row.replies = [...row.replies, ""];
}

function removeReply(row, index) {
  row.replies = row.replies.filter((_, i) => i !== index);
}
</script>

<template>
  <n-card size="small">
    <template #header>快速回复</template>
    <template #header-extra>
      <n-button size="small" @click="addRow">添加关键词</n-button>
    </template>

    <p class="hint">消息里出现某个关键词时，从它的候选回复里挑一条发出去。</p>

    <n-empty v-if="rows.length === 0" description="还没有关键词">
      <template #extra>
        <n-button size="small" @click="addRow">添加第一个关键词</n-button>
      </template>
    </n-empty>

    <ul v-else class="rules">
      <li v-for="(row, index) in rows" :key="index" class="rule">
        <div class="rule__head">
          <n-input v-model:value="row.keyword" placeholder="关键词，例如：收到请回复" />
          <n-button
            quaternary
            circle
            aria-label="删除这个关键词"
            @click="removeRow(index)"
          >
            ✕
          </n-button>
        </div>

        <div class="rule__replies">
          <div v-for="(_reply, replyIndex) in row.replies" :key="replyIndex" class="rule__reply">
            <n-input
              v-model:value="row.replies[replyIndex]"
              placeholder="候选回复，例如：收到"
            />
            <n-button
              quaternary
              circle
              :disabled="row.replies.length === 1"
              aria-label="删除这条候选回复"
              @click="removeReply(row, replyIndex)"
            >
              ✕
            </n-button>
          </div>
          <n-button size="small" dashed block @click="addReply(row)">添加候选回复</n-button>
        </div>
      </li>
    </ul>
  </n-card>
</template>

<style scoped>
.hint {
  margin: 0 0 12px;
  font-size: 12px;
  line-height: 1.6;
  opacity: 0.65;
}

.rules {
  margin: 0;
  padding: 0;
  list-style: none;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.rule {
  padding: 12px;
  border: 1px solid var(--hairline);
  border-radius: 6px;
}

.rule__head {
  display: flex;
  align-items: center;
  gap: 8px;
}

.rule__replies {
  margin-top: 8px;
  padding-left: 12px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.rule__reply {
  display: flex;
  align-items: center;
  gap: 8px;
}
</style>
