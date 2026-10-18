# AstraHerald

QQ 消息托管 + 私聊分级提醒 + 课表联动。Tauri v2 + Vue 3 + NaiveUI，前端纯 JavaScript。

## 它做什么

- 私聊消息按「课表状态 × 前台窗口状态」分级：强提醒窗 / 右侧侧通知 / 延后至下课（见下文场景矩阵）
- 快速回复候选、白名单 / 黑名单、好友身份（一键按备注名匹配课表课程名）
- 文件消息：ppt / pptx / doc / docx 自动下载到桌面；当前或下一节正是这位老师的课且没有全屏时自动打开；全屏只下载
- 课表状态来自 AstraSchedule/desktop 的本地只读接口 http://127.0.0.1:3011/status（连不上就用配置里的默认状态）
- QQ Hook 与 OneBot 11 网关由你自备的 OneBot 框架（SnowLuma / NapCat / Lagrange…）承担，
  在配置页「连接配置」里指一下它的启动文件就行——本应用不随包发布它，两边各自升级

## 安装和使用

- 安装包不需要管理员，默认装到 `D:\AstraHerald`；D 盘不可用就退 `C:\AstraHerald`，两个盘都不可写才落回
  `%LOCALAPPDATA%\AstraHerald`。升级 / 重装沿用上次装的位置
- 首次启动会弹出配置页；之后不再自动弹，需要时点托盘图标的**应用配置**
- 托盘菜单三项：**应用配置** / **连接配置**（打开 `http://127.0.0.1:5099`，即 SnowLuma 的 WebUI；别的
  框架的 WebUI 端口不一样，改了端口就自己开）/ **退出**
- 退出时只收掉本应用自己拉起的那个框架进程，你自己起的框架不受影响
- 启动时会查一次更新（读 OSS 上的版本信息），有新版会问你要不要下载并安装

无人值守安装：安装包认命令行参数，配合 `/S` 一条命令就能装完并预设好配置（`/D=` 必须是最后一个
参数且不能加引号；其余参数的值里带 `/` 或空格的——URL、带空格的路径——要用双引号包住）：

- `/AUTOSTART=0|1`：开机自启
- `/PERIOD=in_class|after_class`：连不上课表服务时的默认状态
- `/WHITELIST=1,2,3` `/BLACKLIST=4,5`：逗号分隔的 QQ 号
- `/ONEBOT=<启动文件>` `/ONEBOT_HOST=` `/ONEBOT_PORT=` `/ONEBOT_PATH=` `/ONEBOT_TOKEN=`：连接配置
- `/SIDE_DURATION=15`：侧通知显示时长（秒）
- `/UPDATER=0|1` `/UPDATE_SOURCE=<latest.json 地址>`：自动更新

这些值先摊成安装目录里的 `install-preset.ini`，首次启动并入 `config.json` 后自动删掉；
带了预设参数的那次启动不会再弹配置页。注意升级 / 重装沿用上次的安装位置，`/D=` 想换位置得先卸载。

## OneBot 框架（自备）

QQ Hook 与 OneBot 11 网关由你自备的框架承担，本应用不随包发布它：两边各自升级，也不用去改
别的项目的源码。配置页「连接配置」里指一下它的启动文件：

1. **启动文件**：框架的入口，SnowLuma / NapCat 的 launcher.bat、独立的 exe、项目的 index.mjs 都认。
   它所在的目录就算框架目录
2. **连接参数**：主机 / 端口 / 路径 / Access Token 留空就自动从框架自己的配置里读（token 也跟着走，
   不用手抄）；填了就以配置页为准。点「自动发现」立刻按框架配置回填一遍
3. 找配置的目录顺序：启动文件所在目录 → 上一级目录（有的框架启动器在 bin/ 子目录里）。
   在目录下的 config/*.json 里先挑文件名以 onebot 开头的，否则按文件名排序；
   在 JSON 里按语义找 `wsServers` / `websocketServers` 数组（不写死路径，各家嵌套层数不一样），
   键名认 `accessToken` 和 `token` 两种。单个文件超过 256 KB、或超过 40 个文件就放弃
4. 读不到连接参数就回落到 kovi.conf.toml，再不行用 127.0.0.1:3001
5. 框架自己的配置本应用一个字都不写。Hook 自动加载（SnowLuma 的 hookAutoLoad）原版是 false，
   不开就 Hook 不上 QQ、一条消息都收不到——我们只在日志里提醒一句，要无人值守就在启动框架前设
   `SNOWLUMA_HOOK_AUTOLOAD=1`（见本仓库外的 deploy.bat），或者去框架自己的 WebUI 里打开
6. 端口已经有人在听就直接用现成的那份；没人听才拉起子进程，退出时收掉（只收自己拉起的那个）

读出来的 WS path 会去掉尾部斜杠（SnowLuma 写的是 "/"，原样交给 kovi 会拼成 ws://host:port//api → HTTP 400）。

启动 .mjs/.js/.cjs 时优先用同目录自带的 node.exe，没有才找 PATH 里的 node；.bat/.cmd 走 cmd /C。
框架自己的 EULA 确认、扫码登录还是它自己的流程：本应用不代你同意任何协议、也不替框架改配置，
拉起子进程时一个环境变量都不设。无人值守要用的那几个变量（SnowLuma 的 `SNOWLUMA_ACCEPT_EULA` /
`SNOWLUMA_ACCEPT_PRIVACY` / `SNOWLUMA_HOOK_AUTOLOAD`）由启动框架的人在启动前自己设，
deploy.bat 就是这么做的——跑那个脚本的是你，所以同意条款的也是你。

## 场景矩阵

行是前台窗口状态（全屏 / 最大化 / 非桌面焦点 / 桌面），列是课表状态（上课 / 下课），
格子是提醒方式（强提醒 / 侧通知 / 延后至下课，延后只在上课时可选）。默认值：

- 上课：全屏 = 延后至下课，其余 = 侧通知
- 下课：全屏 = 侧通知，其余 = 强提醒

连不上课表服务时按「启动与运行」里的**默认状态**走，默认是**上课**——
宁可多提醒（上课那几格是侧通知 / 全屏延后），也不要当成下课把老师上课发的消息吞掉。

全屏认的是「窗口盖满整块屏幕」，所以独占全屏和无边框全屏都算。

## 数据放在哪

都在安装目录里：配置 `config.json`、聊天记录 `history.jsonl`、`logs/`、`kovi.conf.toml`、
WebView2 内核缓存 `.webview2/`（跑起来后约 170 MB）。进程一启动就把工作目录钉到数据目录，
所以从快捷方式、开始菜单还是开机自启进来，读写的都是同一份——开机自启走 `HKCU\...\Run`，那里不带
工作目录，不钉死就会把配置写到 `C:\Windows\System32` 去。

如果安装目录不可写（装在 Program Files、只读共享、受控文件夹访问拦了），会自动退到
`%APPDATA%\AstraHerald`，日志里会打出退到了哪。**注意**：你指的那个 OneBot 框架有自己的目录，
它写不写得进由它自己的安装位置决定，跟这里的回退没关系。

卸载时确认页上有个「Delete the application data」勾选框，**默认不勾**。勾上会把安装目录里的运行时
数据一并删掉（`config.json`、`config.json.bak`、`history.jsonl`、`kovi.conf.toml`、`kovi.plugin.toml`、
`logs`、`.webview2`），最后连安装目录一起收掉；不勾就只删安装包带的东西（程序、快捷方式、开机自启、
注册表项）。你自己那份 OneBot 框架的目录不在安装目录里，**不会被碰**。

配置解析失败不会静默丢数据：坏文件先备份成 `config.json.bak`，再用默认配置重写。

## 从源码构建

    bun install
    bun run tauri dev                                  # 前端 Vite + Tauri
    cargo test --lib --manifest-path src-tauri/Cargo.toml
    bun run build                                      # 只构建前端

打安装包用 `bun run tauri build --bundles nsis`；需要自动更新签名（`TAURI_SIGNING_PRIVATE_KEY`）
才能过，开发时不用打安装包。发布流程、仓库 Secrets、签名私钥这些维护者的事见
[CONTRIBUTING.md](./CONTRIBUTING.md)。
