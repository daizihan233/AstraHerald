import { defineConfig } from "vite";
import vue from "@vitejs/plugin-vue";
import process from "node:process";
import { fileURLToPath } from "node:url";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [vue()],

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || "127.0.0.1",
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },

  // 四个入口：配置页（窗口 main）、私聊提醒窗（窗口 notice）、侧通知（窗口 notice-side-*）
  // 和聊天记录（窗口 history）。frontendDist 是 ../dist，所以这些 html 必须落在 dist 根目录。
  build: {
    rollupOptions: {
      input: {
        main: fileURLToPath(new URL("./index.html", import.meta.url)),
        notice: fileURLToPath(new URL("./notice.html", import.meta.url)),
        side: fileURLToPath(new URL("./side.html", import.meta.url)),
        history: fileURLToPath(new URL("./history.html", import.meta.url)),
      },
    },
  },
}));
