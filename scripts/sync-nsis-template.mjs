// 从 @tauri-apps/cli 里提取 Tauri 原版 NSIS 模板，打上 AstraHerald 的默认安装目录补丁，
// 输出 src-tauri/nsis/installer.nsi（tauri.conf.json 的 bundle.windows.nsis.template 指向它）。
// 升级 Tauri 后重跑：node scripts/sync-nsis-template.mjs
// ponytail: 整份 fork，Tauri 模板变了要重跑本脚本；锚点对不上会直接报错，不会写坏文件。
import fs from 'node:fs';
import path from 'node:path';

const BANNER = "; ---------------------------------------------------------------------------\n; AstraHerald 定制版 NSIS 模板 —— 不要手改，改动请写到 scripts/sync-nsis-template.mjs\n; 相对 Tauri 原版 installer.nsi 只有三处差异：\n;   1. currentUser 模式的默认安装目录：D:\\AstraHerald → C:\\AstraHerald → %LOCALAPPDATA%（只有两个盘都不可写时才用）\n;   2. 追加 AstraHeraldPickInstallDir / AstraHeraldDirWritable 两个函数\n;   3. 卸载勾了「删除应用数据」时，连安装目录里的运行时数据一起清掉（见 NEW_APPDATA）\n; 升级 Tauri（@tauri-apps/cli）后重跑：node scripts/sync-nsis-template.mjs\n; ---------------------------------------------------------------------------\n";
const OLD_BRANCH = "    !else if \"${INSTALLMODE}\" == \"currentUser\"\n      StrCpy $INSTDIR \"$LOCALAPPDATA\\${PRODUCTNAME}\"\n    !endif";
const NEW_BRANCH = "    !else if \"${INSTALLMODE}\" == \"currentUser\"\n      ; AstraHerald: 默认 D:\\AstraHerald，D 盘不可用退 C:\\AstraHerald\n      Call AstraHeraldPickInstallDir\n    !endif";
const HELPERS = "; --- AstraHerald 定制：默认安装目录 D:\\<Product> → C:\\<Product> → %LOCALAPPDATA%\\<Product> ---\nFunction AstraHeraldPickInstallDir\n  StrCpy $INSTDIR \"D:\\${PRODUCTNAME}\"\n  Call AstraHeraldDirWritable\n  StrCmp $R0 \"1\" 0 +2\n    Return\n  StrCpy $INSTDIR \"C:\\${PRODUCTNAME}\"\n  Call AstraHeraldDirWritable\n  StrCmp $R0 \"1\" 0 +2\n    Return\n  StrCpy $INSTDIR \"$LOCALAPPDATA\\${PRODUCTNAME}\"\nFunctionEnd\n\n; 试建目录并写一个探针文件；可写 $R0=1，否则 $R0=0\nFunction AstraHeraldDirWritable\n  StrCpy $R0 0\n  CreateDirectory $INSTDIR\n  ClearErrors\n  FileOpen $R1 \"$INSTDIR\\.astra-herald-write-test\" w\n  IfErrors 0 +2\n    Return\n  FileClose $R1\n  Delete \"$INSTDIR\\.astra-herald-write-test\"\n  StrCpy $R0 \"1\"\nFunctionEnd\n";
const OLD_APPDATA = "    RmDir /r \"$APPDATA\\${BUNDLEID}\"\n    RmDir /r \"$LOCALAPPDATA\\${BUNDLEID}\"\n";
const NEW_APPDATA = OLD_APPDATA + "\n" + [
  "    ; AstraHerald: 运行时数据都落在安装目录里，勾了这个框就一并清掉，别让安装目录剩半截。",
  "    ; 以后 app 往工作目录里写新文件，记得加到这里，不然最后的 RMDir 收不掉目录。",
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
if (raw.indexOf('!define PRODUCTNAME') > raw.indexOf(FUNC_ANCHOR)) throw new Error('PRODUCTNAME 的定义在插入点之后');

const patched =
  BANNER +
  raw
    .replace(OLD_BRANCH, NEW_BRANCH)
    .replace(FUNC_ANCHOR, HELPERS + FUNC_ANCHOR)
    .replace(OLD_APPDATA, NEW_APPDATA);
const outPath = path.join(process.cwd(), 'src-tauri', 'nsis', 'installer.nsi');
fs.mkdirSync(path.dirname(outPath), { recursive: true });
fs.writeFileSync(outPath, patched, 'utf8');
console.log('已生成 ' + outPath + '（' + patched.length + ' 字符 / ' + patched.split('\n').length + ' 行，来自 ' + binPath + '）');
