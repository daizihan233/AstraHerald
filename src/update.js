// 自动更新：调用 src-tauri/src/update.rs 的两个命令，走 invoke 而不是 JS 插件——
// 更新源要能在配置页里改，而插件的 endpoints 是编译期固定的。
// 用 createDiscreteApi 而不是 useDialog()，因为 App.vue 自己就在 NDialogProvider 外面。
import { invoke } from "@tauri-apps/api/core";
import { relaunch } from "@tauri-apps/plugin-process";
import { createDiscreteApi } from "naive-ui";
import { loadConfig } from "./api.js";

const { dialog, message } = createDiscreteApi(["dialog", "message"]);

/// 启动时查一次。配置里关掉自动更新就什么都不做。
export async function checkUpdateOnStart() {
  // 开发时版本号一直是仓库里的 0.1.0，线上永远更大，不挡一下 dev 里会反复弹。
  if (!import.meta.env.PROD) return;
  try {
    const config = await loadConfig();
    if (!config?.updater?.enabled) return;
  } catch (error) {
    // 配置都读不到就别去麻烦更新服务了。
    console.warn("读配置失败，跳过自动更新检查：", error);
    return;
  }

  let update;
  try {
    update = await invoke("check_update");
  } catch (error) {
    // 没网、更新源挂了都走进这里：静默失败，不打扰用户。
    console.warn("检查更新失败：", error);
    return;
  }
  if (!update) return;
  askAndInstall(update);
}

/// 配置页「检查更新」按钮：用户主动点的，所以无论自动更新开没开、是不是 dev 都真去查，
/// 而且结果要说出来（没新版本也 toast 一下，不然点了没反应像坏了）。
export async function checkUpdateManually() {
  let update;
  try {
    update = await invoke("check_update");
  } catch (error) {
    message.error("检查更新失败：" + error);
    return;
  }
  if (!update) {
    message.success("已经是最新版本了");
    return;
  }
  askAndInstall(update);
}

function askAndInstall(update) {
  const notes = update.notes ? "\n\n" + update.notes : "";
  dialog.info({
    title: "有新版本",
    content:
      update.current + " → " + update.version + notes + "\n\n现在下载并安装吗？装完会自动重启。",
    positiveText: "下载并安装",
    negativeText: "以后再说",
    onPositiveClick: async () => {
      const loading = message.loading("正在下载更新…", { duration: 0 });
      try {
        await invoke("install_update");
        loading.destroy();
        await relaunch();
      } catch (error) {
        loading.destroy();
        message.error("更新失败：" + error);
      }
    },
  });
}
