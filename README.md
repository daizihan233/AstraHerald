# AstraHerald

QQ 消息托管 + 私聊分级提醒 + 课表联动。Tauri v2 + Vue 3 + NaiveUI，前端纯 JavaScript。

## 它做什么

- 私聊消息按「课表状态 × 前台窗口状态」分级：强提醒窗 / 右侧侧通知 / 延后至下课（见下文场景矩阵）
- 快速回复候选、白名单 / 黑名单、好友身份（一键按备注名匹配课表课程名）
- 文件消息：ppt / pptx / doc / docx 自动下载到桌面；当前或下一节正是这位老师的课且没有全屏时自动打开；全屏只下载
- 课表状态来自 AstraSchedule/desktop 的本地只读接口 http://127.0.0.1:3011/status（连不上就用配置里的默认状态）
- QQ Hook 与 OneBot 11 网关由你自备的 OneBot 框架（SnowLuma / NapCat / Lagrange…）承担，
  在配置页「连接配置」里指一下它的启动文件就行——本应用不随包发布它，两边各自升级

## OneBot 框架（自备）

QQ Hook 与 OneBot 11 网关由你自备的框架承担，本应用不随包发布它：两边各自升级，也不用去改
别的项目的源码。配置页「连接配置」里指一下它的启动文件：

1. **启动文件**：框架的入口，SnowLuma 的 index.mjs、NapCat 的 launcher.bat、独立的 exe 都认。
   它所在的目录就算框架目录
2. **连接参数**：主机 / 端口 / 路径 / Access Token 留空就自动从框架自己的配置里读（token 也跟着走，
   不用手抄）；填了就以配置页为准。点「自动发现」立刻按框架配置回填一遍
3. 找配置的目录顺序：启动文件所在目录 → 上一级目录（有的框架启动器在 bin/ 子目录里）。
   在目录下的 config/*.json 里先挑文件名以 onebot 开头的，否则按文件名排序；
   在 JSON 里按语义找 `wsServers` / `websocketServers` 数组（不写死路径，各家嵌套层数不一样），
   键名认 `accessToken` 和 `token` 两种。单个文件超过 256 KB、或超过 40 个文件就放弃
4. 读不到连接参数就回落到 kovi.conf.toml，再不行用 127.0.0.1:3001
5. 配置旁边若有 config/runtime.json，只把 hookAutoLoad 改成 true——SnowLuma 自动生成的是 false，
   不改就 Hook 不上 QQ、一条消息都收不到；别的字段一个不动
6. 端口已经有人在听就直接用现成的那份；没人听才拉起子进程，退出时收掉（只收自己拉起的那个）

读出来的 WS path 会去掉尾部斜杠（SnowLuma 写的是 "/"，原样交给 kovi 会拼成 ws://host:port//api → HTTP 400）。

启动 .mjs/.js/.cjs 时优先用同目录自带的 node.exe，没有才找 PATH 里的 node；.bat/.cmd 走 cmd /C。
框架自己的 EULA 确认、扫码登录还是它自己的流程（本应用带了 SnowLuma 那两个无人值守环境变量，
别的框架会忽略），本应用只在后台把它拉起来。

安装包固定按当前用户安装（bundle.windows.nsis.installMode = currentUser），
这样安装目录可写——AstraHerald 要在自己目录里生成 config.json、logs/、.webview2/。

## 场景矩阵

行是前台窗口状态（全屏 / 最大化 / 非桌面焦点 / 桌面），列是课表状态（上课 / 下课），
格子是提醒方式（强提醒 / 侧通知 / 延后至下课，延后只在上课时可选）。默认值：

- 上课：全屏 = 延后至下课，其余 = 侧通知
- 下课：全屏 = 侧通知，其余 = 强提醒

连不上课表服务时按「启动与运行」里的**默认状态**走，默认是**上课**——
宁可多提醒（上课那几格是侧通知 / 全屏延后），也不要当成下课把老师上课发的消息吞掉。

## 开发

    bun install
    bun run tauri dev                                  # 前端 Vite + Tauri
    cargo test --lib --manifest-path src-tauri/Cargo.toml
    bun run build                                      # 只构建前端

### 数据放在哪

默认在可执行文件同目录（配置 `config.json`、聊天记录 `history.jsonl`、`logs/`、`kovi.conf.toml`；
源码开发时是 `src-tauri/target/debug/`）。进程一启动就把工作目录钉到数据目录，所以从快捷方式、
开始菜单还是开机自启进来，读写的都是同一份——开机自启走 `HKCU\...\Run`，那里是不带工作目录的，
不钉死就会把配置写到 `C:\Windows\System32` 去。

如果 exe 目录不可写（装在 Program Files、只读共享、受控文件夹访问拦了），会自动退到
`%APPDATA%\AstraHerald`，日志里会打出退到了哪。**注意**：你指的那个 OneBot 框架有自己的目录，
它写不写得进由它自己的安装位置决定，跟这里的回退没关系。

安装包（per-user 模式，不需要管理员）的默认目录是 `D:\AstraHerald`，D 盘不可用就退 `C:\AstraHerald`，
两个盘都不可写才落回 `%LOCALAPPDATA%\AstraHerald`；升级/重装沿用上次装的位置。这是我们 fork 了
Tauri 的 NSIS 模板做的（`src-tauri/nsis/installer.nsi`，由 `scripts/sync-nsis-template.mjs` 从
`@tauri-apps/cli` 提取原版再打补丁；升级 Tauri 后重跑一次那个脚本）。

另外两件跟「干净」有关的事：

- WebView2 的内核缓存不放在 `%LOCALAPPDATA%\cn.getastra.herald` 了，改到安装目录下的 `.webview2`（跑起来后约 170 MB）。这是浏览器内核自己的缓存，删掉会自动重建，不影响配置；主窗口和两个提醒窗共用它，所以窗都改成运行时自己建（`tauri.conf.json` 里主窗口 `create: false`）。
- 卸载的确认页上有个「Delete the application data」勾选框，**默认不勾**。勾上以后会把安装目录里的运行时数据一并删掉：`config.json`、`config.json.bak`、`history.jsonl`、`kovi.conf.toml`、`kovi.plugin.toml`、`logs`、`.webview2`，
  最后连安装目录一起收掉；不勾就只删安装包带的东西（程序、快捷方式、开机自启、注册表项）。
  你自己那份 OneBot 框架的目录不在安装目录里，**不会被碰**。

配置解析失败不会静默丢数据：坏文件先备份成 `config.json.bak`，再用默认配置重写。

## 发布

推送到 main 就发一版（.github/workflows/build.yml，参考同组织 AstraSchedule/desktop 的那份）：

1. 滚动版本号 YYYYMM.D.N（例 202510.2.40 = 2025-10-02 的第 40 次构建），只算一次、给后面任务共用
2. windows-latest 上 bun install → bun x tauri build --bundles nsis
3. 发 GitHub Release（标签 v<版本>，附安装包和固定别名 AstraHeraldInstaller.exe）
4. 传 OSS 的 AstraHerald/<版本>/ 与 AstraHerald/latest/，再校验 latest/ 上的文件和本次构建等大、清掉 latest/ 里的旧安装包

需要的仓库配置（和 AstraSchedule 同名）：

- secrets `OSS_ACCESS_KEY` / `OSS_SECRET_KEY`：S3 兼容对象存储，endpoint https://cn-nb1.rains3.com，bucket yanmo-objects
自动更新（tauri-plugin-updater + latest.json + 签名）还没接：接的时候在 build 步骤加 TAURI_SIGNING_PRIVATE_KEY、
打开 bundle.createUpdaterArtifacts，把 latest.json 与 .sig 一起传 OSS，客户端再去读 latest/。