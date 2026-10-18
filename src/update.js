// 启动时查一次自动更新：更新源是 OSS 上的 latest.json（见 tauri.conf.json 的 plugins.updater）。
// 用 createDiscreteApi 而不是 useDialog()，因为 App.vue 自己就在 NDialogProvider 外面。
import { check } from "@tauri-apps/plugin-updater";
import { relaunch } from "@tauri-apps/plugin-process";
import { createDiscreteApi } from "naive-ui";

const { dialog, message } = createDiscreteApi(["dialog", "message"]);

export async function checkUpdate() {
  // 开发时版本号一直是仓库里的 0.1.0，线上永远更大，不挡一下 dev 里会反复弹。
  if (!import.meta.env.PROD) return;

  let update;
  try {
    update = await check();
  } catch (error) {
    // 没网、更新源挂了都走进这里：静默失败，不打扰用户。
    console.warn("检查更新失败：", error);
    return;
  }
  if (!update) return;

  dialog.info({
    title: "有新版本",
    content: update.currentVersion + " → " + update.version + "，现在下载并安装吗？装完会自动重启。",
    positiveText: "下载并安装",
    negativeText: "以后再说",
    onPositiveClick: async () => {
      const loading = message.loading("正在下载更新…", { duration: 0 });
      try {
        await update.downloadAndInstall();
        loading.destroy();
        await relaunch();
      } catch (error) {
        loading.destroy();
        message.error("更新失败：" + error);
      }
    },
  });
}
