<script setup>
import { NCard, NSelect, NSwitch } from "naive-ui";
import { DEFAULT_PERIOD_OPTIONS } from "../config/schema.js";

/// 默认状态：连不上课表服务时假定现在在上课还是下课。
const period = defineModel("period", { type: String, required: true });
/// 开机自启。
const autostart = defineModel("autostart", { type: Boolean, required: true });
</script>

<template>
  <n-card size="small">
    <template #header>启动与运行</template>

    <div class="rows">
      <div class="row">
        <div class="row__label">
          <span>默认状态</span>
          <span class="hint">
            连不上课表服务时按这个状态决定提醒方式。连着的时候以课表的实时状态为准。
          </span>
        </div>
        <n-select
          v-model:value="period"
          class="row__control"
          :options="DEFAULT_PERIOD_OPTIONS"
          aria-label="默认状态"
        />
      </div>

      <div class="row">
        <div class="row__label">
          <span>开机自启</span>
          <span class="hint">
            开机后直接待在托盘里，不再弹配置页。开发构建不会写入注册表，打包安装后才生效。
          </span>
        </div>
        <n-switch v-model:value="autostart" aria-label="开机自启" />
      </div>
    </div>
  </n-card>
</template>

<style scoped>
.rows {
  display: flex;
  flex-direction: column;
  gap: 16px;
}

.row {
  display: flex;
  align-items: center;
  gap: 16px;
}

.row__label {
  display: flex;
  flex-direction: column;
  gap: 4px;
  margin-right: auto;
}

.row__control {
  width: 160px;
}

.hint {
  font-size: 12px;
  line-height: 1.6;
  opacity: 0.65;
}
</style>
