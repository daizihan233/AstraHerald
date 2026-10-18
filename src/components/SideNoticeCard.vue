<script setup>
import { ref } from "vue";
import { NButton, NCard, NInputNumber } from "naive-ui";
import { previewSideNotice } from "../api.js";

/// { duration_secs, font_size_px, nick_font_px, max_count }：侧通知显示多久自己收起来、
/// 正文和昵称各多大、最多同时挂几条
/// （config.rs 的 SideSettings：3..300 秒、12..40 像素、1..10 条）。
const side = defineModel({ type: Object, required: true });

const previewing = ref(false);

/// 让后端弹一条侧通知样品，先把时长和字号效果看出来。
/// 直接把输入框里的值传过去：不用先保存就能看出改了字号是什么样子。
async function preview() {
  previewing.value = true;
  try {
    await previewSideNotice({
      font: side.value.font_size_px,
      nickFont: side.value.nick_font_px,
      secs: side.value.duration_secs,
    });
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
          <span>最多同时展示</span>
          <span class="hint">屏幕上同时挂几条。再多来的就把最早那条换成新的，不会一直往上堆。1~10 条。</span>
        </div>
        <div class="row__control">
          <n-input-number v-model:value="side.max_count" :min="1" :max="10" :step="1" size="small">
            <template #suffix>条</template>
          </n-input-number>
        </div>
      </div>
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
          <span>正文字号</span>
          <span class="hint">消息内容那几行。卡片宽度不变，字大了行就少、卡片更高。12~40 像素。</span>
        </div>
        <div class="row__control">
          <n-input-number v-model:value="side.font_size_px" :min="12" :max="40" :step="1" size="small">
            <template #suffix>像素</template>
          </n-input-number>
        </div>
      </div>
      <div class="row">
        <div class="row__label">
          <span>昵称字号</span>
          <span class="hint">上面那行昵称（发送者）。跟正文分开调，默认比正文小一号。</span>
        </div>
        <div class="row__control">
          <n-input-number v-model:value="side.nick_font_px" :min="12" :max="40" :step="1" size="small">
            <template #suffix>像素</template>
          </n-input-number>
        </div>
      </div>
      <div class="row">
        <div class="row__label">
          <span>看看效果</span>
          <span class="hint">弹一条样品侧通知，正文/昵称字号和时长就是上面这套（不用先保存）。</span>
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
