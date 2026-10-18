<script setup>
import { NCard, NSelect } from "naive-ui";
import { SCENE_PERIODS, SCENE_STATES, sceneActionsFor } from "../config/schema.js";

/// { in_class: { fullscreen, maximized, unfocused, desktop }, after_class: { ... } }
const scene = defineModel({ type: Object, required: true });

// 每个时段的可选提醒方式：只有「上课」那一列多了「延后至下课」。
const optionsByPeriod = Object.fromEntries(
  SCENE_PERIODS.map((period) => [
    period.key,
    sceneActionsFor(period.key).map((action) => ({ label: action.label, value: action.key })),
  ]),
);

</script>

<template>
  <n-card size="small">
    <template #header>场景设置</template>

    <p class="hint">按「窗口状态 × 是否在上课」决定怎么提醒。</p>

    <div class="matrix__scroll">
      <table class="matrix">
        <caption class="matrix__caption">
          场景设置：四种窗口状态在上课时、下课时分别用哪种提醒方式
        </caption>
        <thead>
          <tr>
            <th scope="col">窗口状态</th>
            <th v-for="period in SCENE_PERIODS" :key="period.key" scope="col">
              {{ period.label }}
            </th>
          </tr>
        </thead>
        <tbody>
          <tr v-for="state in SCENE_STATES" :key="state.key">
            <th scope="row">{{ state.label }}</th>
            <td v-for="period in SCENE_PERIODS" :key="period.key">
              <n-select
                v-model:value="scene[period.key][state.key]"
                :options="optionsByPeriod[period.key]"
                :aria-label="`${period.label}、${state.label}时的提醒方式`"
                size="small"
                clearable
                placeholder="请选择"
              />
            </td>
          </tr>
        </tbody>
      </table>
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

.matrix__scroll {
  overflow-x: auto;
}

.matrix {
  width: 100%;
  border-collapse: collapse;
}

/* 表格要能说清自己是什么，但标题不该占版面。 */
.matrix__caption {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}

.matrix th,
.matrix td {
  border: 1px solid var(--hairline);
  padding: 6px 8px;
  text-align: left;
}

.matrix th {
  background: var(--hairline-fill);
  font-weight: 600;
  white-space: nowrap;
}
</style>
