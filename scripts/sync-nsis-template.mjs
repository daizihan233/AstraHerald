// 从 @tauri-apps/cli 里提取 Tauri 原版 NSIS 模板，打上 AstraHerald 的默认安装目录补丁，
// 输出 src-tauri/nsis/installer.nsi（tauri.conf.json 的 bundle.windows.nsis.template 指向它）。
// 升级 Tauri 后重跑：node scripts/sync-nsis-template.mjs
// ponytail: 整份 fork，Tauri 模板变了要重跑本脚本；锚点对不上会直接报错，不会写坏文件。
import fs from 'node:fs';
import path from 'node:path';

const BANNER = "; ---------------------------------------------------------------------------\n; AstraHerald 定制版 NSIS 模板 —— 不要手改，改动请写到 scripts/sync-nsis-template.mjs\n; 相对 Tauri 原版 installer.nsi 只有四处差异：\n;   1. currentUser 模式的默认安装目录：D:\\AstraHerald → C:\\AstraHerald → %LOCALAPPDATA%（只有两个盘都不可写时才用）\n;   2. 追加 AstraHeraldPickInstallDir / AstraHeraldDirWritable 两个函数\n;   3. 卸载勾了「删除应用数据」时，连安装目录里的运行时数据一起清掉（见 NEW_APPDATA）\n;   4. 安装时可用命令行参数预设设置（/AUTOSTART=1 /ONEBOT=... 见 PRESET_BLOCK），摊平成 $INSTDIR\\install-preset.ini\n; 升级 Tauri（@tauri-apps/cli）后重跑：node scripts/sync-nsis-template.mjs\n; ---------------------------------------------------------------------------\n";
const OLD_BRANCH = "    !else if \"${INSTALLMODE}\" == \"currentUser\"\n      StrCpy $INSTDIR \"$LOCALAPPDATA\\${PRODUCTNAME}\"\n    !endif";
const NEW_BRANCH = "    !else if \"${INSTALLMODE}\" == \"currentUser\"\n      ; AstraHerald: 默认 D:\\AstraHerald，D 盘不可用退 C:\\AstraHerald\n      Call AstraHeraldPickInstallDir\n    !endif";
const HELPERS = "; --- AstraHerald 定制：默认安装目录 D:\\<Product> → C:\\<Product> → %LOCALAPPDATA%\\<Product> ---\nFunction AstraHeraldPickInstallDir\n  StrCpy $INSTDIR \"D:\\${PRODUCTNAME}\"\n  Call AstraHeraldDirWritable\n  StrCmp $R0 \"1\" 0 +2\n    Return\n  StrCpy $INSTDIR \"C:\\${PRODUCTNAME}\"\n  Call AstraHeraldDirWritable\n  StrCmp $R0 \"1\" 0 +2\n    Return\n  StrCpy $INSTDIR \"$LOCALAPPDATA\\${PRODUCTNAME}\"\nFunctionEnd\n\n; 试建目录并写一个探针文件；可写 $R0=1，否则 $R0=0\nFunction AstraHeraldDirWritable\n  StrCpy $R0 0\n  CreateDirectory $INSTDIR\n  ClearErrors\n  FileOpen $R1 \"$INSTDIR\\.astra-herald-write-test\" w\n  IfErrors 0 +2\n    Return\n  FileClose $R1\n  Delete \"$INSTDIR\\.astra-herald-write-test\"\n  StrCpy $R0 \"1\"\nFunctionEnd\n";
const OLD_APPDATA = "    RmDir /r \"$APPDATA\\${BUNDLEID}\"\n    RmDir /r \"$LOCALAPPDATA\\${BUNDLEID}\"\n";
const NEW_APPDATA = OLD_APPDATA + "\n" + [
  "    ; AstraHerald: 运行时数据都落在安装目录里，勾了这个框就一并清掉，别让安装目录剩半截。",
  "    ; 以后 app 往工作目录里写新文件，记得加到这里，不然最后的 RMDir 收不掉目录。",
  "    Delete \"$INSTDIR\\install-preset.ini\"",
  "    Delete \"$INSTDIR\\config.json\"",
  "    Delete \"$INSTDIR\\config.json.bak\"",
  "    Delete \"$INSTDIR\\history.jsonl\"",
  "    Delete \"$INSTDIR\\kovi.conf.toml\"",
  "    Delete \"$INSTDIR\\kovi.plugin.toml\"",
  "    RmDir /r \"$INSTDIR\\logs\"",
  "    RmDir /r \"$INSTDIR\\.webview2\"",
  "    RMDir \"$INSTDIR\"",
  "",
].join("\n");
const FUNC_ANCHOR = "Function RestorePreviousInstallLocation";

// 安装时命令行预设：变量 + 取参数宏 + 裁剪函数 + 读参数函数 + 落盘函数。
const PRESET_BLOCK = "; --- AstraHerald 定制：安装时命令行预设 -------------------------------------\n; 用法（可只传一部分）：\n;   AstraHerald_x.y.z_x64-setup.exe /S /AUTOSTART=1 /PERIOD=after_class\n;     /WHITELIST=123,456 /BLACKLIST= /ONEBOT=D:\\SnowLuma\\launcher.bat\n;     /ONEBOT_HOST= /ONEBOT_PORT= /ONEBOT_PATH= /ONEBOT_TOKEN=\n;     /SIDE_DURATION=20 /UPDATER=0 /UPDATE_SOURCE=https://example.com/latest.json\n; 值里带 / 或空格的（URL、带空格的路径）建议用双引号包住：/UPDATE_SOURCE=\"https://example.com/latest.json\"；\n; GetOptions 只在引号外把 / 当参数边界，所以没加引号一般也认，加了更保险。\n; 这里只负责摊平成 $INSTDIR\\install-preset.ini；首次启动时由 src-tauri/src/preset.rs\n; 并进 config.json 再把它删掉。所以装完不启动就等于没设过。\nVar AstraHeraldHasPreset\nVar AstraHeraldAutostart\nVar AstraHeraldPeriod\nVar AstraHeraldWhitelist\nVar AstraHeraldBlacklist\nVar AstraHeraldOnebotStartup\nVar AstraHeraldOnebotHost\nVar AstraHeraldOnebotPort\nVar AstraHeraldOnebotPath\nVar AstraHeraldOnebotToken\nVar AstraHeraldSideDuration\nVar AstraHeraldUpdater\nVar AstraHeraldUpdateSource\n\n; 取一个 /KEY=值：$R0 是 ${GetParameters} 的结果（下面用宏，别在这里改 $R0）。\n!macro AstraHeraldTakeOption KEY VAR\n  ${GetOptions} $R0 \"/${KEY}=\" $R1\n  ${IfNot} ${Errors}\n    ${If} $R1 != \"\"\n      StrCpy ${VAR} $R1\n      StrCpy $AstraHeraldHasPreset \"1\"\n    ${EndIf}\n  ${EndIf}\n!macroend\n\nFunction AstraHeraldReadPreset\n  StrCpy $AstraHeraldHasPreset \"0\"\n  ${GetParameters} $R0\n  !insertmacro AstraHeraldTakeOption AUTOSTART $AstraHeraldAutostart\n  !insertmacro AstraHeraldTakeOption PERIOD $AstraHeraldPeriod\n  !insertmacro AstraHeraldTakeOption WHITELIST $AstraHeraldWhitelist\n  !insertmacro AstraHeraldTakeOption BLACKLIST $AstraHeraldBlacklist\n  !insertmacro AstraHeraldTakeOption ONEBOT $AstraHeraldOnebotStartup\n  !insertmacro AstraHeraldTakeOption ONEBOT_HOST $AstraHeraldOnebotHost\n  !insertmacro AstraHeraldTakeOption ONEBOT_PORT $AstraHeraldOnebotPort\n  !insertmacro AstraHeraldTakeOption ONEBOT_PATH $AstraHeraldOnebotPath\n  !insertmacro AstraHeraldTakeOption ONEBOT_TOKEN $AstraHeraldOnebotToken\n  !insertmacro AstraHeraldTakeOption SIDE_DURATION $AstraHeraldSideDuration\n  !insertmacro AstraHeraldTakeOption UPDATER $AstraHeraldUpdater\n  !insertmacro AstraHeraldTakeOption UPDATE_SOURCE $AstraHeraldUpdateSource\nFunctionEnd\n\n; 摊平成 preset.rs 认的那份 key=value（空值不写，等于没传）。\nFunction AstraHeraldWritePreset\n  StrCmp $AstraHeraldHasPreset \"1\" 0 astraherald_preset_skip\n  FileOpen $R9 \"$INSTDIR\\install-preset.ini\" w\n  FileWrite $R9 \"[app]$\\r$\\n\"\n  ${If} $AstraHeraldAutostart != \"\"\n    FileWrite $R9 \"autostart=$AstraHeraldAutostart$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldPeriod != \"\"\n    FileWrite $R9 \"period=$AstraHeraldPeriod$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldWhitelist != \"\"\n    FileWrite $R9 \"whitelist=$AstraHeraldWhitelist$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldBlacklist != \"\"\n    FileWrite $R9 \"blacklist=$AstraHeraldBlacklist$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldOnebotStartup != \"\"\n    FileWrite $R9 \"onebot_startup=$AstraHeraldOnebotStartup$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldOnebotHost != \"\"\n    FileWrite $R9 \"onebot_host=$AstraHeraldOnebotHost$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldOnebotPort != \"\"\n    FileWrite $R9 \"onebot_port=$AstraHeraldOnebotPort$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldOnebotPath != \"\"\n    FileWrite $R9 \"onebot_path=$AstraHeraldOnebotPath$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldOnebotToken != \"\"\n    FileWrite $R9 \"onebot_token=$AstraHeraldOnebotToken$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldSideDuration != \"\"\n    FileWrite $R9 \"side_duration=$AstraHeraldSideDuration$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldUpdater != \"\"\n    FileWrite $R9 \"updater=$AstraHeraldUpdater$\\r$\\n\"\n  ${EndIf}\n  ${If} $AstraHeraldUpdateSource != \"\"\n    FileWrite $R9 \"update_source=$AstraHeraldUpdateSource$\\r$\\n\"\n  ${EndIf}\n  FileClose $R9\nastraherald_preset_skip:\nFunctionEnd";
// 插到 .onInit 的第一条语句之前：${GetParameters} 在这里才能拿到完整命令行。
const ONINIT_ANCHOR = "Function .onInit\n  ${GetOptions} $CMDLINE \"/P\" $PassiveMode";
const ONINIT_PATCHED = PRESET_BLOCK + "\nFunction .onInit\n  ; AstraHerald: 读安装命令行的预设参数（见上面的 PRESET_BLOCK）\n  Call AstraHeraldReadPreset\n  ${GetOptions} $CMDLINE \"/P\" $PassiveMode";
const INSTALL_ANCHOR = "Section Install\n  SetOutPath $INSTDIR\n";
const INSTALL_PATCHED = INSTALL_ANCHOR + "\n  ; AstraHerald: 把预设摊平成 $INSTDIR\\install-preset.ini（首次启动由 preset.rs 并进 config.json）\n  Call AstraHeraldWritePreset\n";

const pkgRoot = path.join(process.cwd(), 'node_modules', '@tauri-apps');
const pkgDir = fs.readdirSync(pkgRoot).find((name) => name.startsWith('cli-'));
if (!pkgDir) throw new Error('找不到 @tauri-apps/cli-* 包：' + pkgRoot);
const binName = fs.readdirSync(path.join(pkgRoot, pkgDir)).find((name) => name.startsWith('cli.'));
if (!binName) throw new Error('找不到 ' + pkgDir + ' 里的 cli.* 二进制');
const binPath = path.join(pkgRoot, pkgDir, binName);

const buf = fs.readFileSync(binPath);
const anchor = buf.indexOf(Buffer.from('!define PLACEHOLDER_INSTALL_DIR', 'latin1'));
if (anchor < 0) throw new Error('二进制里找不到模板锚点 PLACEHOLDER_INSTALL_DIR：' + binPath);
const printable = (byte) => byte === 9 || byte === 10 || byte === 13 || (byte >= 32 && byte <= 126);
let start = anchor;
let end = anchor;
while (start > 0 && printable(buf[start - 1])) start--;
while (end < buf.length - 1 && printable(buf[end])) end++;
const raw = buf.slice(start, end).toString('latin1').replace(/\r\n/g, '\n');

const count = (hay, needle) => hay.split(needle).length - 1;
if (count(raw, OLD_BRANCH) !== 1) {
  throw new Error('上游模板的 currentUser 分支变了（命中 ' + count(raw, OLD_BRANCH) + ' 次），人工核对后再改本脚本');
}
if (count(raw, FUNC_ANCHOR) !== 1) throw new Error('找不到函数锚点 ' + FUNC_ANCHOR);
if (count(raw, OLD_APPDATA) !== 1) {
  throw new Error('上游模板的卸载「删除应用数据」分支变了（命中 ' + count(raw, OLD_APPDATA) + ' 次），人工核对后再改本脚本');
}
if (count(raw, ONINIT_ANCHOR) !== 1) {
  throw new Error('上游模板的 .onInit 开头变了（命中 ' + count(raw, ONINIT_ANCHOR) + ' 次），人工核对后再改本脚本');
}
if (count(raw, INSTALL_ANCHOR) !== 1) {
  throw new Error('上游模板的 Section Install 开头变了（命中 ' + count(raw, INSTALL_ANCHOR) + ' 次），人工核对后再改本脚本');
}
if (raw.indexOf('!define PRODUCTNAME') > raw.indexOf(FUNC_ANCHOR)) throw new Error('PRODUCTNAME 的定义在插入点之后');

const patched =
  BANNER +
  raw
    .replace(OLD_BRANCH, NEW_BRANCH)
    .replace(FUNC_ANCHOR, HELPERS + FUNC_ANCHOR)
    .replace(OLD_APPDATA, NEW_APPDATA)
    // 用函数形式的 replacement：PRESET_BLOCK 里有 $ 打头的东西，字符串形式会被当成特殊替换模式。
    .replace(ONINIT_ANCHOR, () => ONINIT_PATCHED)
    .replace(INSTALL_ANCHOR, () => INSTALL_PATCHED);
const outPath = path.join(process.cwd(), 'src-tauri', 'nsis', 'installer.nsi');
fs.mkdirSync(path.dirname(outPath), { recursive: true });
fs.writeFileSync(outPath, patched, 'utf8');
console.log('已生成 ' + outPath + '（' + patched.length + ' 字符 / ' + patched.split('\n').length + ' 行，来自 ' + binPath + '）');
