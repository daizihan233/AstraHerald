# 开发与发布

这份是维护者用的（仓库配置、CI、签名、安装包模板）。使用者要看的是 [README.md](./README.md)。

## 开发环境

    bun install
    bun run tauri dev                                   # 前端 Vite + Tauri 一起起
    cargo test --lib --manifest-path src-tauri/Cargo.toml
    bun run build                                       # 只构建前端

提交前至少要过 `cargo test --lib` 与 `bun run build`。

## 安装包模板是 fork 过的

Tauri 不支持配置安装目录，所以 `src-tauri/nsis/installer.nsi` 是从 `@tauri-apps/cli` 里提取原版 NSIS 模板、
再打三处补丁的产物（`bundle.windows.nsis.template` 指过去）：

1. 目录选择页的默认值：`D:\AstraHerald` 可写就用它，否则 `C:\AstraHerald`，两个盘都不可写才回
   `$LOCALAPPDATA\AstraHerald`（`AstraHeraldPickInstallDir` / `AstraHeraldDirWritable`）；升级 / 重装沿用注册表里
   上次装的位置（Tauri 模板自带的 `RestorePreviousInstallLocation`，故意保留）
2. 卸载时清掉安装目录里的运行时文件（`config.json`、`config.json.bak`、`history.jsonl`、
   `kovi.conf.toml`、`kovi.plugin.toml`、`logs`、`.webview2`）——**以后 app 往工作目录里多写一个文件，
   记得加进这份清单**，否则最后那句 `RMDir` 会因为目录非空而收不掉
3. WebView2 缓存不再落在 `%LOCALAPPDATA%\<identifier>`，改到安装目录下的 `.webview2`：主窗口在
   `tauri.conf.json` 里 `create: false`，由 `lib.rs` 的 setup 用 `WebviewWindowBuilder` +
   `utils::webview_data_dir()` 建；两个提醒窗同理

升级 `@tauri-apps/cli` 之后重跑 `node scripts/sync-nsis-template.mjs`：脚本按锚点从 CLI 二进制里重新提取
原版再补齐补丁，锚点对不上就直接 throw，不会写坏文件。

## 发布（.github/workflows/build.yml）

推送到 main 就发一版（提交信息以 `docs` 开头会跳过）：

1. 算一次滚动版本号 `YYYYMM.D.N`（例 `202510.2.40` = 2025-10-02 的第 40 次构建，D 不带前导零），
   经 job output 给后面共用；随后写进 package.json / tauri.conf.json / Cargo.toml
2. windows-latest：`bun install` → `bun x tauri build --bundles nsis`（带签名环境变量）
3. 收集产物：`AstraHerald_<ver>_x64-setup.exe`（安装包）、`AstraHeraldInstaller.exe`（固定名字的别名，
   官网 / 注册中心的永久链接）、自动更新产物 + 它的 `.sig`、`latest.json`。自动更新的载荷以 bundler
   实际输出为准：当前 CLI 直接给安装包签名（`...setup.exe.sig`），有的版本会另出
   `...setup.exe.zip` + `.sig`；两者 updater 都吃，CI 谁在就用谁（`latest.json` 的 `url` 与
   `signature` 必须指向同一份产物）
4. 发 GitHub Release（标签 `v<版本>`）
5. 传 OSS：`AstraHerald/<版本>/` 与 `AstraHerald/latest/`，然后 prune 任务校验 latest/ 上的文件与本次
   构建等大，删掉不是本版本的残留对象

仓库 Secrets（名字与 AstraSchedule 一致）：

- `OSS_ACCESS_KEY` / `OSS_SECRET_KEY`：S3 兼容对象存储，endpoint `https://cn-nb1.rains3.com`，
  bucket `kuohu-objects`
- `TAURI_SIGNING_PRIVATE_KEY`：`.secrets/updater.key` 的**内容**（不是路径）
- `TAURI_SIGNING_PRIVATE_KEY_PASSWORD`：私钥没设密码也要建这个 secret（留空），否则 tauri 会交互式
  提问、CI 直接挂住

## 签名与自动更新

- 生成密钥：`bun x tauri signer generate -w .secrets/updater.key`。`.secrets/` 已 gitignore；
  **私钥丢了就没法再给老版本发更新**（客户端只认这一个公钥）
- 公钥写在 `src-tauri/tauri.conf.json` 的 `plugins.updater.pubkey`，端点同样是那个文件里的
  `plugins.updater.endpoints`（`https://static.getastra.cn/AstraHerald/latest/latest.json`）
- `bundle.createUpdaterArtifacts = true` 之后，本地 `tauri build` 也必须设
  `TAURI_SIGNING_PRIVATE_KEY`，否则构建直接失败（`tauri dev` 不受影响）
- 客户端：`src/update.js` 启动时 `check()`，有新版弹窗 → `downloadAndInstall()` →
  `relaunch()`；用 naive-ui 的 `createDiscreteApi` 是因为这段代码在 `NDialogProvider` 外面，
  拿不到 `useDialog()`
- 更新安装模式 NSIS `passive`（`plugins.updater.windows.installMode`）
- 换镜像 / 换域名要同时改客户端端点与 CI 里生成 `latest.json` 用的那个 URL
