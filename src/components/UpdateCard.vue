<script setup>
import { ref } from "vue";
import { NButton, NCard, NInput, NSwitch } from "naive-ui";
import { checkUpdateManually } from "../update.js";

/// { enabled, source }：自动检查更新的开关和更新源（config.rs 的 UpdaterSettings）。
const updater = defineModel({ type: Object, required: true });

const checking = ref(false);

async function check() {
  checking.value = true;
  try {
    // 结果（有更新/最新/失败）由 update.js 自己用 message + dialog 说，这里只管转圈。
    await checkUpdateManually();
  } finally {
    checking.value = false;
  }
}
</script>

<template>
  <n-card size="small">
    <template #header>更新</template>

    <div class="rows">
      <div class="row">
        <div class="row__label">
          <span>自动检查更新</span>
          <span class="hint">启动后自动查一次，有新版就静默装好并重启，不问你。</span>
        </div>
        <div class="row__control">
          <n-switch v-model:value="updater.enabled" />
        </div>
      </div>
      <div class="row">
        <div class="row__label">
          <span>更新源</span>
          <span class="hint">填 latest.json 的地址；留空用内置的那份。</span>
        </div>
        <div class="row__control row__control--wide">
          <n-input
            v-model:value="updater.source"
            size="small"
            clearable
            placeholder="留空 = 内置更新源"
            aria-label="更新源"
          />
        </div>
      </div>
      <div class="row">
        <div class="row__label">
          <span>立即检查</span>
          <span class="hint">不看上面的开关，手动查一次。</span>
        </div>
        <div class="row__control">
          <n-button size="small" :loading="checking" @click="check">检查更新</n-button>
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

.row__control--wide {
  flex: 1 1 320px;
  max-width: 420px;
}

.hint {
  font-size: 12px;
  line-height: 1.5;
  opacity: 0.65;
}
</style>
