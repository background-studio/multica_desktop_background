# 架构与故障经验

## CDP 注入链路

完整流程：

1. `controller.rs` 在候选路径查找官方 `Multica.exe`：
   - `%LOCALAPPDATA%\Programs\@multicadesktop\Multica.exe`
   - `%LOCALAPPDATA%\Programs\Multica\Multica.exe`
   - `%ProgramFiles%\Multica\Multica.exe`
2. 校验可执行路径，读取版本。
3. 若已有未启用 CDP 的 Multica，要求用户确认重启。
4. **优先**走 `electron_wco::launch_with_transparent_wco`（CDP + 主进程 Inspector）；
   WCO 失败则回退普通 `launch_multica`（仅 renderer CDP）。
5. 首选调试口 **9227**（占用则向后试），等待 `/json/version`。
6. 保存 `schemaVersion`、port、browser ID、executable、`wcoEnabled` 到 `runtime.json`。
7. `injector.rs` 只接受同一 browser ID、回环 WebSocket、Multica renderer page。
8. 为每个 target 开启 Runtime/Page、临时 bypass CSP。
9. `Page.addScriptToEvaluateOnNewDocument` 注册早期 payload（大媒体则只注册透明化）。
10. `Runtime.evaluate` 立即应用当前页面。
11. 定期同步新 target；导航和重载由早期脚本覆盖。

安全边界不要放松：

- 不连接任意调试端口。
- 不接受非回环 WebSocket。
- 不向非 Multica 主界面 target 注入。
- 不按进程名粗暴结束所有 `Multica.exe`；必须比较官方安装路径的完整路径。
- 主进程 Inspector 只用于启动瞬间补丁，完成后必须关掉；失败路径也要 `resume`，
  不能把 Multica 留在 `--inspect-brk` 暂停态。

## Electron WCO 启动补丁

目标：让网页内容铺进标题栏区域，同时保留 Windows 原生最小化/最大化/关闭。

核心文件：`src-tauri/src/electron_wco.rs`。

步骤概要：

1. 启动 Multica 时附加
   `--inspect-brk=127.0.0.1:<port>`（首选 9238）和 renderer CDP 参数。
2. 连接 Node Inspector；`--inspect-brk` 往往先停在匿名包装脚本（`url: ""`），
   需要 resume 到真正的 `app.asar/out/main/index.js`。
3. 读取入口源码，确认仍存在：
   - `titleBarStyle: "hiddenInset"`（或当前版本等价结构）
   - `new electron.BrowserWindow`
   结构对不上就**拒绝补丁**，不要硬猜。
4. 在 Electron import 结束、窗口创建前下断点。
5. 在该 call frame 上 `evaluate` 代理模块局部变量 `electron.BrowserWindow`：
   - `titleBarStyle = "hidden"`
   - `titleBarOverlay = { color: "rgba(0,0,0,0)", symbolColor: "#ffffff", height: 48 }`
6. resume，等到 renderer 上报 `windowControlsOverlay` visible。
7. 关闭 Inspector；写入 `runtime.json` 的 `wcoEnabled: true`、`schemaVersion: 2`。
8. payload 加 `multica-background-wco`，用 `navigator.windowControlsOverlay`
   维护 `--cbg-wco-safe-right`。

旧会话过滤：`schemaVersion` 过旧或 `wcoEnabled != true` 的 runtime 不能当透明标题栏会话复用，
应重新走 WCO 启动（或明确回退）。

### 为什么不用 Win32 覆盖窗

曾用独立 layered overlay 去盖原生 caption，实测问题：

- 覆盖层与客户区接缝出现白条/黑条；
- 频繁 `SetWindowPos` 导致标题栏抖动；
- 原生最小化/最大化/关闭被挡或点不到；
- 两窗 Z-order / hit-test 同步极脆。

结论：**废弃 overlay 方案**。相关 `titlebar_overlay` / `titlebar-overlay.html` 已删除，
不要加回来。

## Payload 生命周期

`payload.ts` 生成自包含 IIFE，状态保存在：

```text
window.__MULTICA_BACKGROUND_STUDIO__
```

主要对象：

- `#multica-background-style`：普通 DOM 样式。
- `#multica-background-layer`：固定背景媒体层。
- `#multica-background-media`：图片或视频。
- `#multica-background-tile`：平铺模式。
- `#multica-background-overlay`：整页底色。

`build.rs` 从 `src/main/payload.ts` 提取 `BACKGROUND_CSS`、`REVIEW_SHADOW_CSS`
和 runtime 模板；改 payload 后需触发 Cargo 重编。

重应用前先调用旧状态的 `cleanup()`，避免 observer/timer 重复、Blob URL 泄漏、
style/layer 重复，以及旧 revision 阻止新 CSS 生效。

修订号同时混入媒体 sha256、display 设置、媒体 kind、`BACKGROUND_CSS`、
以及占位的 `REVIEW_SHADOW_CSS`。

Rust 侧断言注入脚本里的选择器时，记得 CSS 经 `serde_json` 进脚本后引号会变成
`\"`；应用 raw string 匹配，例如
`r#"[data-sidebar=\"menu-button\"]:hover"#`。

## 为什么媒体使用 base64 + Blob URL

Multica 渲染页不能稳定读取 Studio 回环 HTTP 媒体 URL。Rust 后端读取受管媒体、
限制内嵌大小（64 MB）、生成 data URL，payload 内解码为 Blob，再交给
`<img>` / `<video>`；cleanup 时 revoke。媒体加载失败时必须整体 cleanup。

大媒体脚本不要同时挂 early + evaluate 双份完整 payload，否则 WebSocket 会堵很久，
看起来像「应用卡住 / 退出无响应」。

## 媒体库与动态源

数据目录：

```text
%LOCALAPPDATA%/MulticaBackgroundStudio
```

主要文件：`settings.json`、`library.json`、`runtime.json`、`media/`、`temporary/`。

- `origin: "api"`：保存随机 API 地址，轮播/刷新时重新拉取。
- `origin: "folder"`：只保存目录路径，应用时再挑选文件，不复制入库。
- 不覆盖当前媒体文件；新内容用 `<id>-<hash-prefix>.<ext>`，再删旧文件，避免 Windows 文件锁。

## 网络安全

远程媒体只允许无账号信息的 HTTP/HTTPS。每次请求和重定向都要校验 URL、
DNS 解析结果，拒绝 loopback / 私网等；限制体积、边长、重定向次数和类型。

## 设置扩展流程

新增显示设置时同时修改：

1. `src/shared/contracts.ts`（`DisplaySettings` + `DEFAULT_SETTINGS`）
2. `src-tauri/src/models.rs`（结构、默认值、patch、normalize）
3. `src/main/settings.ts`（若仍有 JS 规范化路径）
4. `src/renderer/App.tsx`（控件、标签、预览变量）
5. `src/renderer/styles.css`（Studio 预览若需要）
6. `src/main/payload.ts`（`ROOT_PROPERTIES`、`setProp`、CSS、cleanup）
7. `injector.rs` 的 `EARLY_TRANSPARENCY_SCRIPT`（若首帧会闪实底）
8. 相关测试（Rust payload 断言 + 设置 normalize）

透明度设置统一 clamp 到 0..1。不要让 UI 最小值和 normalize 最小值不一致。

雾度经验：

- 大面积壳（侧栏、画布）：`opacity * 28%`
- 局部实色块（选中/悬停、卡片、菜单）：`opacity * 100%`

## Background Studio 插件

- 协议：`pluginProtocol: 1`
- 启动：`Multica Background Studio.exe --plugin`
- Pipe：`\\.\pipe\background-studio-multica`
- `pluginId`：`multica`
- Release 资产：`MulticaBackgroundStudio-<version>-plugin.zip`
  （zip 内是改名后的 exe，不是整个 NSIS 目录）

壳从 GitHub Release 发现最新 `*-plugin.zip`，不需要改壳仓 catalog 版本号。
本地验证**不要**只跑 `cargo build --release` 再复制 exe：那样容易仍指向 Vite
`devUrl`（`http://127.0.0.1:5175`），壳里打开设置面板会出现
「127.0.0.1 拒绝连接 / ERR_CONNECTION_REFUSED」。

本地插件必须用完整前端内嵌产物：

- `npm run package:tauri`（或 CI 的 `*-plugin.zip`）
- 再解压/复制到 `%LOCALAPPDATA%\BackgroundStudio\plugins\multica\<version>\`
- 同步改 `%LOCALAPPDATA%\BackgroundStudio\plugins.json` 的 `installedVersion`
- 替换前先停掉占用中的插件进程

正式环境优先在壳里对 Multica 点「检查更新 / 安装」。

## 已解决的典型故障

### 标题栏白条 / 黑条 / 抖动 / 按钮消失

原因：独立 Win32 overlay 与 Multica 窗口不同步。

处理：改为 Electron WCO；删除 overlay 窗口代码；payload 只负责安全区 padding。

### WCO 补丁报 `process is not defined` / 停在错误脚本

原因：在 Inspector 最早 pause（匿名包装）或错误作用域里改 `process`/`require`。

处理：等到主入口 `index.js`；在 import 结束后、窗口创建前，代理**模块局部**
`electron` 变量上的 `BrowserWindow`，不要假设能直接改全局 `process.electron`。

### WCO 后 Multica 卡住不动

原因：失败路径没有 `Debugger.resume` / `Runtime.runIfWaitingForDebugger`，
进程一直停在 `--inspect-brk`。

处理：成功和失败都要 resume；成功后再确保 Inspector 端口关闭。

### 侧栏壳透了，但「任务」选中或「小队」悬停仍实黑

原因：只透明了 `[data-sidebar="sidebar"]`，没打
`[data-sidebar="menu-button"]` 的 active/hover/focus-visible。

处理：三项状态都跟随 `--cbg-sidebar-opacity`（`* 100%`），early 脚本同步包含。

### 看板卡片不透明

原因：列 tint 或外层 `data-slot="card"` 不是真正卡片底；真实底色在
sortable issue 链接内的 `bg-surface`。

处理：用精确 sortable 选择器接 `cardOpacity`，不要拿整列开刀。

### 用量 / 运行时卡片不透明

原因：这些页用 shadcn `.bg-card`（`oklch` 实底），不是看板那套 sortable/`bg-surface`。

处理：`.bg-card` 跟随 `--cbg-card-opacity`（`* 100%`），early 透明化同步包含。

### 新建智能体整页发黑、背景“整个没有”

原因：`.bg-page-canvas` 已透明，但路由在里面又铺了
`.bg-page-canvas .bg-background` 实底（`oklch(0.18 …)`），把背景盖死。

处理：内层 `.bg-background` 强制透明，只保留外层 canvas 的 surface 雾。

### 智能体列表中间出现黑色阴影横杠

原因：sticky 表头 `group/header` 用 `::after` 画了一条
`linear-gradient(oklch(0.18 …) → transparent)` 的滚动淡出；页面透明后这条渐变
看起来就像横切的黑杠（Codex sticky 搜索栏同类问题）。

处理：清 `.bg-page-canvas .sticky::before/::after` 的背景与渐变，不只改表头本身。

### 创建智能体底栏横切壁纸（头发被切断）

原因：配置页底部 `.pe-chat-launcher.sticky.bottom-0` 带约 95% 实色底，
透明化后仍像一条黑杠压在壁纸上，把头发等细节齐腰切断。

处理：让 `.pe-chat-launcher` 跟随 `--cbg-surface-opacity`（`* 28%`），
不要保留原生近实底。

### 创建智能体「访问权限」选中灰块

原因：权限 radiogroup 里选中项带实色 `.bg-muted`，悬停同款，
透明页面上会变成一整块黑/灰底。

处理：`[role="radiogroup"] [role="radio"][class~="bg-muted"]` / `:hover` 跟随
`--cbg-surface-opacity`（`* 100%`）；选中态仍靠 radio 圆点。

### 附件二级预览整块实黑、看不到壁纸

原因：`AttachmentPreviewModal` 把 `role="dialog"` 打在铺满窗口的遮罩上
（`fixed inset-0 bg-black/80`）。菜单规则按 `* 100%` 给所有 `[role="dialog"]`
涂近实底（默认菜单透明度 0.9），中间 `max-w-6xl` 卡片再铺一层 `bg-background`
实色，壁纸被盖死。小确认框才是 `fixed top-1/2` 的那张卡。

处理：`inset-0` 遮罩强制透明；`max-w-6xl` 卡片跟随 `--cbg-surface-opacity`
（`* 28%`）；内层 `.bg-background` / `bg-muted/30` / `iframe` 清空。

### 创建智能体「指令」框右下角小黑块

原因：描述/指令 `textarea` 开了 `resize: vertical`，Chromium 原生
`::-webkit-resizer` 手柄在 Electron 透明窗上会画成一颗实心小黑方块。
只把 resizer 背景设成 `transparent` 不够——那一块会直接透出窗体黑底
（CDP 用 lime 染 `::-webkit-resizer` 可复现：黑块变绿）。

处理：`textarea { resize: none }` 卸掉原生手柄；再清
`::-webkit-resizer` / `::-webkit-scrollbar-corner`。代价是不能再拖角改高。

### 应用后「没有可用的 Multica CDP 运行时」

原因：启动插件时 Multica 还没带着调试端口在跑，或 runtime.json 失效。

处理：在 Studio / 壳里再点一次应用；需要重启时走确认框，让 WCO+CDP 一起起来。

### 页面冻结或样式打转

原因：每次无条件改写 style `textContent` 触发 MutationObserver 环。

处理：revision 不同才写；CSS 变量值不同才 `setProperty`；DOM 更新按 rAF 合并。

### 品牌图标人物层太透/太实

图标源在 `assets/icon-source/`。改人物层 alpha 后用 `npx tauri icon` 重生全套
平台图标，不要只换一张 png。

## 调试策略

### 推荐

- 先 `npm run dev` 调试 Tauri Studio（Vite `5175`）。
- 用一次性脚本连 Multica renderer CDP（9227）和必要时主进程 Inspector（9238）。
- 把 DOM、计算样式、`windowControlsOverlay`、截图作为证据。
- 对动态页面测试导航后、悬停后、重载后状态。

### 避免

- 不用类名关键词大范围 `background: transparent`。
- 不根据截图颜色猜元素。
- 不恢复 Win32 标题栏覆盖窗。
- 不用 `setInterval` 作为正常首帧方案。
- 不用 opacity 作用于整块卡片/菜单文字。
- 不长期打开 `--inspect-brk` 会话。

## 测试和发布

`npm run check` 包含：

- renderer TypeScript（`tsc --noEmit`）；
- `cargo test`（含 payload 嵌入断言、WCO patch 断言、设置 normalize）。

历史 Vitest 文件可能仍在仓库，但当前 npm scripts 以 Tauri/Cargo 为准；
缺 vitest 依赖时不要硬跑 `npx vitest`。

发布前：

1. 删除 `poc/` 一次性文件。
2. 跑 `npm run check`。
3. 在真实 Multica 完成标题栏 + 侧栏 + 卡片矩阵验证。
4. 同步 bump `package.json` / `tauri.conf.json` / `Cargo.toml` / `Cargo.lock`。
5. 提交后打 `vX.Y.Z` 并 push，触发 Release workflow。
6. 确认 Release 同时有 NSIS 安装包和 `*-plugin.zip`。
