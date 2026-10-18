<script setup>
import { ref } from "vue";
import { NButton, NCard, NInputNumber } from "naive-ui";
import { previewSideNotice } from "../api.js";

/// { duration_secs }：侧通知显示多久自己收起来（config.rs 的 SideSettings，3..300 秒）。
const side = defineModel({ type: Object, required: true });

const previewing = ref(false);

/// 让后端弹一条侧通知样品，先把时长和字号效果看出来。
async function preview() {
  previewing.value = true;
  try {
    await previewSideNotice();
  } catch (error) {
    // 预览失败基本都是后端连不上，别静默吞掉。
    window.alert(String(error));
  } finally {
    previewing.value = false;
  }
}
</script>

<template>
  <n-card size="small">
    <template #header>侧通知</template>

    <div class="rows">
      <div class="row">
        <div class="row__label">
          <span>显示时长</span>
          <span class="hint">过了这么久自己收起来，底部那条进度线就是它。</span>
        </div>
        <div class="row__control">
          <n-input-number v-model:value="side.duration_secs" :min="3" :max="300" :step="1" size="small">
            <template #suffix>秒</template>
          </n-input-number>
        </div>
      </div>
      <div class="row">
        <div class="row__label">
          <span>看看效果</span>
          <span class="hint">弹一条样品侧通知，样式和时长就是上面这套。</span>
        </div>
        <div class="row__control">
          <n-button size="small" :loading="previewing" @click="preview">预览侧通知</n-button>
        </div>
      </div>
    </div>
  </n-card>
</template>

<style scoped>
.rows {
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.row {
  display: flex;
  align-items: center;
  gap: 16px;
}

.row__label {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 0;
}

.row__control {
  margin-left: auto;
  flex: none;
}

.hint {
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.65;
}
</style>
