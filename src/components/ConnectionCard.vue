<script setup>
import { NButton, NCard, NInput, useMessage } from "naive-ui";
import { open } from "@tauri-apps/plugin-dialog";
import { ref } from "vue";
import { discoverOnebot } from "../api.js";

/// 后端 Config.onebot：{ startup, host, port, path, access_token }。
/// 只有 startup 是必须的；剩下四项留空 = 从框架自己的配置里读（user 手填的优先）。
const onebot = defineModel({ type: Object, required: true });

const message = useMessage();
const discovering = ref(false);
/// 自动发现的结果/失败原因，直接显示在卡片里（不弹气泡，用户能一直看着）。
const found = ref("");
const error = ref("");

async function browse() {
  const picked = await open({
    title: "选择 OneBot 框架的启动文件",
    multiple: false,
    filters: [
      { name: "启动文件", extensions: ["exe", "bat", "cmd", "mjs", "js", "cjs"] },
      { name: "全部文件", extensions: ["*"] },
    ],
  });
  if (typeof picked !== "string") return;
  onebot.value.startup = picked;
  // 选完直接顺手发现一次：八成的情况就是想要这个。
  await discover();
}

async function discover() {
  found.value = "";
  error.value = "";
  discovering.value = true;
  try {
    const info = await discoverOnebot(onebot.value.startup);
    onebot.value.host = info.host;
    onebot.value.port = String(info.port);
    onebot.value.path = info.path;
    onebot.value.access_token = info.access_token;
    found.value =
      "已从 " + info.dir + " 读到 ws://" + info.host + ":" + info.port + (info.path || "/");
    message.success("已按框架配置填好连接参数");
  } catch (e) {
    error.value = String(e);
  } finally {
    discovering.value = false;
  }
}

function clear() {
  onebot.value.startup = "";
  onebot.value.host = "";
  onebot.value.port = "";
  onebot.value.path = "";
  onebot.value.access_token = "";
  found.value = "";
  error.value = "";
}
</script>

<template>
  <n-card size="small">
    <template #header>连接配置</template>

    <div class="rows">
      <div class="row">
        <span class="row__label">OneBot 框架启动文件</span>
        <div class="row__control">
          <n-input
            v-model:value="onebot.startup"
            placeholder="D:\SnowLuma\launcher.bat"
            aria-label="OneBot 框架启动文件"
          />
          <n-button @click="browse">浏览…</n-button>
          <n-button
            type="primary"
            secondary
            :loading="discovering"
            :disabled="!onebot.startup"
            @click="discover"
          >
            自动发现
          </n-button>
          <n-button quaternary @click="clear">清除</n-button>
        </div>
        <span class="hint">
          指到框架的启动文件就行：SnowLuma / NapCat 的 launcher.bat、独立的 exe 都认。
          AstraHerald 从它所在的位置去找框架自己的 OneBot 配置（一般放在 config/ 下），
          需要时再把它当子进程拉起来；端口已经在听就直接用现成的那份。
        </span>
        <span v-if="found" class="found">{{ found }}</span>
        <span v-if="error" class="error">{{ error }}</span>
      </div>

      <div class="row">
        <span class="row__label">连接参数</span>
        <div class="grid">
          <label class="field">
            <span>主机</span>
            <n-input v-model:value="onebot.host" placeholder="自动发现" aria-label="OneBot 主机" />
          </label>
          <label class="field">
            <span>端口</span>
            <n-input v-model:value="onebot.port" placeholder="自动发现" aria-label="OneBot 端口" />
          </label>
          <label class="field">
            <span>路径</span>
            <n-input v-model:value="onebot.path" placeholder="自动发现" aria-label="OneBot 路径" />
          </label>
          <label class="field">
            <span>Access Token</span>
            <n-input
              v-model:value="onebot.access_token"
              type="password"
              show-password-on="click"
              placeholder="自动发现"
              aria-label="OneBot Access Token"
            />
          </label>
        </div>
        <span class="hint">
          留空的项每次启动时从框架配置里读（token 也跟着走，不用手抄）；填了就以这里为准。
        </span>
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
  flex-direction: column;
  gap: 8px;
}

.row__label {
  font-size: 14px;
}

.row__control {
  display: flex;
  align-items: center;
  gap: 8px;
}

.row__control :deep(.n-input) {
  flex: 1;
}

.grid {
  display: grid;
  grid-template-columns: repeat(auto-fit, minmax(180px, 1fr));
  gap: 12px;
}

.field {
  display: flex;
  flex-direction: column;
  gap: 4px;
  font-size: 13px;
}

.hint {
  font-size: 12px;
  line-height: 1.6;
  opacity: 0.65;
}

.found {
  font-size: 12px;
  line-height: 1.6;
  color: var(--n-color-target, #18a058);
}

.error {
  font-size: 12px;
  line-height: 1.6;
  color: #d03050;
}
</style>
