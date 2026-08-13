---
name: multica-background-development
description: Maintains Multica Background Studio, including CDP injection, Electron WCO transparent titlebar, Multica page and panel transparency, media/slideshow behavior, Background Studio plugin packaging, debugging, and release verification. Use when changing this repository or investigating a Multica desktop UI surface that does not follow background settings.
---

# Multica Background Studio 开发

## 目标

维护 Windows 官方 Multica 桌面应用的可逆背景工具。通过本机 CDP 注入样式和媒体，
不修改 `app.asar`、应用签名、登录状态或页面数据。可独立运行，也可作为
[Background Studio](https://github.com/background-studio/background-studio) 壳的
`--plugin` 插件。

动手前按需读取：

- 页面入口、DOM 特征、透明度归属：[windows-and-selectors.md](windows-and-selectors.md)
- 架构、WCO、故障经验、安全边界：[architecture-and-pitfalls.md](architecture-and-pitfalls.md)

## 不可破坏的原则

1. 不直接修改 Multica 安装资源；暂停和恢复必须能完整移除注入。
2. 不凭截图猜选择器。通过 Multica CDP 检查实际 DOM、计算样式。
3. 一个视觉区域只保留一层有色背景。内部壳层透明，避免半透明叠加变暗。
4. 所有界面不透明度必须允许 `0`；不要添加最低 0.2 之类的兜底。
5. 不用宽泛选择器清空所有背景。先确认稳定入口、尺寸、层级和页面复用范围。
6. `backdrop-filter` 默认关闭。
7. 动态组件必须首帧生效。大媒体 early 脚本只做透明化，完整 payload 只 evaluate 一次。
8. 一次性 CDP / Inspector 探查脚本放在 `poc/`，验证完立即删除。
9. 修改 `BACKGROUND_CSS` 时必须进入修订哈希，确保现有会话热更新。
10. 更改共享设置时同步修改 contracts、默认值、规范化、UI、payload 和测试。
11. 壳层雾度用 `color-mix(... calc(opacity * 28%), transparent)`；侧栏选中/悬停、
    任务卡片等「实色块」用 `* 100%`，避免滑杆到高值变成实心黑罩或块太淡看不见。
12. **标题栏必须走 Electron WCO**，不要再做独立 Win32 覆盖窗去盖原生 caption。
13. WCO 启动失败要能回退到普通 CDP 启动，且不能把 Multica 主进程留在 debugger pause。

## 代码入口

- `src-tauri/src/lib.rs`：Tauri/Rust 主后端、命令、轮播和共享状态。
- `src-tauri/src/host.rs`：托盘、窗口生命周期、退出恢复和 Windows 自启动。
- `src-tauri/src/controller.rs`：发现官方 `Multica.exe`、校验进程、启动 Multica、
  保存和恢复 CDP 会话；需要透明标题栏时走 WCO 启动。
- `src-tauri/src/electron_wco.rs`：用 `--inspect-brk` 在 BrowserWindow 创建前打透明 WCO 补丁。
- `src-tauri/src/injector.rs`：Rust CDP target 同步、早期脚本、运行时更新、暂停和移除；
  含 `window_controls_overlay_visible` 探测。
- `src-tauri/src/media.rs`、`network.rs`、`preview.rs`、`settings.rs`：媒体、安全下载、预览和事务设置。
- `src-tauri/build.rs`、`src-tauri/src/payload.rs`：从 TypeScript 提取共享 payload 并生成 Rust 资源。
- `src/main/payload.ts`：Multica 页面背景层、CSS 变量、选择器、WCO 安全区和清理逻辑。
- `src/shared/contracts.ts`、`src-tauri/src/models.rs`：前端和 Rust 对应的数据契约。
- `src/renderer/App.tsx`：Studio 操作界面和设置控件。
- `src-tauri/src/plugin.rs`、`plugin_ipc.rs`：Background Studio 壳的 named pipe 协议。
- `src/main/*`：历史 Electron 实现；当前构建以 Tauri 为准。

## 安装发现

优先路径：

1. `%LOCALAPPDATA%\Programs\@multicadesktop\Multica.exe`
2. `%LOCALAPPDATA%\Programs\Multica\Multica.exe`
3. `%ProgramFiles%\Multica\Multica.exe`

首选调试口：`9227`。主进程 Inspector 首选：`9238`。
运行时状态：`%LOCALAPPDATA%\MulticaBackgroundStudio\runtime.json`
（`schemaVersion >= 2` 且 `wcoEnabled: true` 才视为可恢复的透明标题栏会话）。

插件安装目录（壳）：`%LOCALAPPDATA%\BackgroundStudio\plugins\multica\<version>\`。

## 标准开发流程

### 1. 建立基线

先看 Git 状态和当前版本，不覆盖用户已有改动。确认 Multica / Studio / 壳插件是否已在跑。

```powershell
git status --short --branch
npm run check
```

开发要求 Node.js 22+、Rust stable 和 MSVC C++ Build Tools。JavaScript 使用 npm，
Rust 使用 Cargo。Vite 开发地址默认 `http://127.0.0.1:5175/`。

### 2. 复现并确定视觉层

先判断问题属于哪一类：

- Windows 原生标题栏 / 最小化最大化关闭：归 **WCO**（`electron_wco.rs` + payload 安全区），
  不是 `surfaceOpacity` 能单独解决的。
- 左侧边栏壳：归 `sidebarOpacity`（`* 28%` 雾）。
- 侧栏选中项 / 悬停项：仍归 `sidebarOpacity`，但用 `* 100%` 打底。
- 顶栏、主画布、普通卡片壳：归 `surfaceOpacity`。
- 看板任务卡片：归 `cardOpacity`。
- 弹出菜单 / 对话框：归 `menuOpacity`。
- 整页压在背景上的底色：归 `overlayColor` + `overlayOpacity`。
- 背景媒体本身：归 `opacity`、路由强度。

不要用子层再画一层相同透明色。应让外层统一打底、内部壳透明。

### 3. 用 CDP 检查 Multica

运行时端口记录在 `%LOCALAPPDATA%/MulticaBackgroundStudio/runtime.json`。只连接：

- `127.0.0.1` 回环地址；
- browser ID 与状态文件一致的实例；
- page target 为 Multica Electron renderer（`file://.../out/renderer/index.html`）。

探查内容至少包括：

- 元素标签、id、role、`data-*`、完整 class；
- `getBoundingClientRect()`；
- `backgroundColor`、`backgroundImage`、`boxShadow`、`backdropFilter`；
- `::before`、`::after`；
- 元素祖先链；
- 标题栏问题额外查 `navigator.windowControlsOverlay` 是否 visible，以及顶栏
  `padding-right` / `--cbg-wco-safe-right`。

截图验证前后状态。不要把探查脚本或截图提交进仓库。

### 4. 选择实现方式

- 普通 DOM：在 `BACKGROUND_CSS` 增加精确规则。
- 侧栏入口：`[data-sidebar="menu-button"]` 的 `[data-active]` / `:hover` / `:focus-visible`
  必须与侧栏壳一起处理，否则会出现「壳透了、选中/悬停仍实黑」。
- 看板卡片：只打
  `[role="button"][aria-roledescription="sortable"] > a[href*="/issues/"] > [class~="bg-surface"]`，
  不要降低整列或文字透明度。
- 标题栏：只允许改 `electron_wco.rs` 的 BrowserWindow 代理 + payload WCO 安全区；
  **禁止**新建独立 overlay HWND 去盖 caption。
- 全页壳：清 `.bg-app-shell`、`[data-slot="sidebar-wrapper"]` 实底，由侧栏/画布各自打底。

### 5. 保证首帧和可恢复性

早期脚本必须在 `documentElement` 出现时即可运行。大媒体（超过约 400KB 脚本）
early 只注入透明化 CSS，完整媒体脚本只走一次 `Runtime.evaluate`。

新增任何注入对象时，同时补齐 `cleanup()`：

- 移除 style、layer；
- 断开 observer、timer、WCO `geometrychange` 监听；
- 撤销 Blob URL；
- 删除根 class 和 CSS 变量（含 `--cbg-wco-safe-right`、`--cbg-card-opacity`）。

### 6. 验证

```powershell
npm run check
```

随后使用真实 Multica 验证：

1. 标题栏背景与页面连续，无白/黑条；原生最小化/最大化/关闭可点。
2. 左侧栏壳、选中项、悬停项都跟随「左侧边栏」滑杆。
3. 任务看板卡片跟随「任务卡片」滑杆（0 / 中间 / 1）。
4. 顶栏与主画布跟随「顶栏与页面」。
5. 附件二级预览（点 markdown/图片附件弹出的中间大卡）能透出壁纸，小确认框仍跟「弹出菜单」。
6. 弹出菜单跟随「弹出菜单」。
7. 深色、浅色主题。
8. 导航、重载时无黑底闪烁。
9. 暂停、恢复官方外观后不残留样式；WCO 失败时能回退且 Multica 不卡住。

### 7. 版本、插件包和发布

补丁修复递增 patch；功能或设置结构变化再考虑 minor。同步修改：

- `package.json`
- `src-tauri/tauri.conf.json`
- `src-tauri/Cargo.toml`
- `src-tauri/Cargo.lock` 中本包版本

本地插件热替换（开发验证）必须走完整打包，**禁止**只 `cargo build --release`
后复制（会连不上 Vite，设置面板出现 `ERR_CONNECTION_REFUSED`）：

```powershell
npm run package:tauri
# 或下载 GitHub Release 的 MulticaBackgroundStudio-<version>-plugin.zip 后解压
# 先停掉占用中的插件进程，再装到：
# %LOCALAPPDATA%\BackgroundStudio\plugins\multica\<version>\
# 并更新 plugins.json 的 installedVersion
```

正式发版：版本三处一致后打 `vX.Y.Z` 标签并 push；`.github/workflows/release.yml` 会：

1. 校验 tag 与三处版本一致；
2. `npm run check` + Tauri 构建；
3. 发布 GitHub Release（NSIS 安装包）；
4. 额外上传 `MulticaBackgroundStudio-<version>-plugin.zip` 供壳安装。

壳 catalog **不钉死版本号**，它从 GitHub 最新带 `*-plugin.zip` 的 Release 拉取。

### 8. 提交和传输

提交前检查完整 diff、测试结果、版本和临时文件。只在用户明确要求时提交或推送。
提交信息说明为什么改，而不是只罗列文件。发版时用户说「发新版」即包含 bump、tag、push。

## 完成条件

- 目标页面视觉结果符合对应透明度控制；
- 标题栏连续透明且原生按钮可用；
- 没有透明度叠加或首帧闪烁；
- 真实 Multica 验证完成；
- `npm run check` 全部通过；
- 临时探查文件已删除；
- 恢复流程仍完整可逆；
- 若发版：Release 含 NSIS 与 `*-plugin.zip`。
