<script setup>
import {
  NConfigProvider,
  NDialogProvider,
  NLayout,
  NLayoutHeader,
  NMessageProvider,
  dateZhCN,
  zhCN,
} from "naive-ui";
import ConfigPage from "./pages/ConfigPage.vue";
import ConnectionStatus from "./components/ConnectionStatus.vue";
import { useSystemTheme } from "./theme.js";

// 跟随系统深浅色；提醒窗口共用同一份逻辑（src/theme.js）。
const { theme } = useSystemTheme();
</script>

<template>
  <n-config-provider :theme="theme" :locale="zhCN" :date-locale="dateZhCN">
    <n-message-provider>
      <n-dialog-provider>
        <n-layout class="app">
          <n-layout-header bordered class="app__bar">
            <div>
              <h1 class="app__title">AstraHerald</h1>
              <p class="app__subtitle">QQ 消息托管 · 配置</p>
            </div>
            <!-- 与 OneBot 框架 / 课表的连接状态：在配置页任何位置都看得到 -->
            <connection-status />
          </n-layout-header>
          <main class="app__main">
            <config-page />
          </main>
        </n-layout>
      </n-dialog-provider>
    </n-message-provider>
  </n-config-provider>
</template>

<style>
:root {
  /* 结构线：浅色/深色下都够用的一层中性分隔色，不引入自己的调色板。 */
  --hairline: rgba(128, 128, 128, 0.28);
  --hairline-fill: rgba(128, 128, 128, 0.08);
}

html,
body {
  margin: 0;
}

.app {
  min-height: 100vh;
}

.app__bar {
  position: sticky;
  top: 0;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 14px 24px;
}

.app__title {
  margin: 0;
  font-size: 16px;
  font-weight: 600;
}

.app__subtitle {
  margin: 2px 0 0;
  font-size: 12px;
  opacity: 0.65;
}

.app__main {
  max-width: 880px;
  margin: 0 auto;
  padding: 16px 24px 0;
}
</style>
