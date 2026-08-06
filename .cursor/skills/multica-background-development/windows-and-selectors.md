# Multica 窗口与选择器

选择器经 CDP 复核（Electron renderer `file://.../out/renderer/index.html`）。
Multica 更新后 DOM 可能变；改样式前必须再用 CDP 核对，不要只靠截图猜。

## 目标 page

- 主界面：`file://.../Programs/@multicadesktop/.../out/renderer/index.html`
  （或 `Programs/Multica/...`）
- 可选：`https://multica.ai` / `https://*.multica.ai`（若出现独立 webview，需单独确认是否注入）

只向 Multica 主应用 page 注入；辅助/空白 target 不要乱注入。

## 注入根标记

- `html.multica-background-active`
- WCO 会话：`html.multica-background-wco`
- 路由：`multica-background-home` / `multica-background-task`
- 媒体层：`#multica-background-layer` / `#multica-background-media` /
  `#multica-background-tile` / `#multica-background-overlay`
- 主题：`html.light` / `html.dark`（探测后设 `multica-background-dark`）
- 状态：`window.__MULTICA_BACKGROUND_STUDIO__`

## 窗口标题栏（WCO）

- 用户入口：窗口最上方、原生最小化/最大化/关闭按钮所在区域。
- **不能**用独立 Tauri/Win32 覆盖窗去盖原生 caption（会白/黑条、抖动、挡按钮）。
- 正确做法：启动时把 Multica `BrowserWindow` 改成
  `titleBarStyle: "hidden"` + 透明 `titleBarOverlay`（见 `electron_wco.rs`）。
- 渲染页标记：`html.multica-background-wco`
- 顶栏为原生按钮预留右侧：
  `header.relative.shrink-0.h-12` + `--cbg-wco-safe-right`
- 安全区来源：`navigator.windowControlsOverlay` 的 `getTitlebarAreaRect()` /
  `geometrychange`；无 WCO API 时不要瞎写死很大的 padding。
- 验证：`window_controls_overlay_visible`（injector）为 true；按钮可点；
  标题栏与下方画布背景连续。

## 全局壳

### 外层打底（必须透明）

- `.bg-app-shell`
- `[data-slot="sidebar-wrapper"]`
- `html` / `body` / `#root`

这些层透明，避免和侧栏/画布叠成双层雾。

### 左侧边栏壳

- 稳定入口：`[data-sidebar="sidebar"]`、`[data-slot="sidebar-inner"]`
- 透明度：`--cbg-sidebar-opacity`，雾度 `* 28%`
- 必须清：`backdrop-filter`、`box-shadow`

### 侧栏选中 / 悬停 / 键盘聚焦

- 用户入口：左侧「收件箱 / 聊天 / 任务 / 小队 / 项目」等菜单项。
- 稳定入口：`[data-sidebar="menu-button"]`
- 状态：
  - 选中：`[data-active]`
  - 悬停：`:hover`
  - 键盘：`:focus-visible`
- 透明度：仍用 `--cbg-sidebar-opacity`，但打底用 `* 100%`
- 只改背景，不要淡化文字和图标
- early 透明化脚本也要包含这三项，否则首帧会先闪实黑块

### 顶栏与主画布

- 稳定入口：`.bg-page-canvas`、`header`、`[data-slot="card"]`、
  `[data-slot="chat-input-surface"]`、创建页底栏 `.pe-chat-launcher`
- 透明度：`--cbg-surface-opacity`，雾度 `* 28%`
- WCO 时顶栏额外吃 `--cbg-wco-safe-right`
- **内层实底**：Create Agent 等路由会在 canvas 内再铺
  `.bg-page-canvas .bg-background`（`oklch(...)` 实色）。外层 canvas 已打雾时，
  内层必须强制透明，否则整页发黑、背景“整个没有”。
- **sticky 黑杠**：智能体列表等 `group/header sticky` 表头带
  `::after` 向下渐变（`linear-gradient(oklch(...) → transparent)`，高约 12px）。
  透明化后会变成横向黑影；必须清
  `.bg-page-canvas .sticky::before/::after` 的 `background-image`。
- **textarea 小黑块**：创建智能体「指令 / 描述」等 `resize: vertical` 输入框右下角
  原生 `::-webkit-resizer`。Electron 透明窗上不能只清背景（会透窗体黑底）；
  用 `textarea { resize: none }` 卸手柄，并清 resizer / scrollbar-corner。
- **创建页底栏横切**：`.pe-chat-launcher.sticky.bottom-0` 原生约 95% 实底，
  会把壁纸（头发等）齐腰切断；跟随 `--cbg-surface-opacity` `* 28%`。

### 看板 / 用量 / 运行时卡片

- 看板任务卡片：

```css
[role="button"][aria-roledescription="sortable"]
  > a[href*="/issues/"]
  > [class~="bg-surface"]
```

- 用量、运行时、排行榜等：shadcn `.bg-card`（`rounded-lg border bg-card`）
- 透明度：`--cbg-card-opacity`，打底 `* 100%`
- 只动卡片底色；文字、状态色、拖拽命中保持完整不透明
- 列背景（黄/绿/蓝 tint）不要误伤成卡片规则

### 弹出层

- `[role="dialog"]`、`[role="menu"]`、`[role="listbox"]`、`.bg-surface-raised`
- 透明度：`--cbg-menu-opacity`，打底 `* 100%`

## 透明度归属

| 控制 | CSS 变量 | 稳定入口 | 混合系数 |
|------|----------|----------|----------|
| 左侧边栏壳 | `--cbg-sidebar-opacity` | `[data-sidebar="sidebar"]`、`[data-slot="sidebar-inner"]` | `* 28%` |
| 侧栏选中/悬停 | `--cbg-sidebar-opacity` | `[data-sidebar="menu-button"][data-active]` / `:hover` / `:focus-visible` | `* 100%` |
| 顶栏与页面 | `--cbg-surface-opacity` | `.bg-page-canvas`、`header`、`[data-slot="card"]`、`[data-slot="chat-input-surface"]`、`.pe-chat-launcher` | `* 28%` |
| 任务卡片 | `--cbg-card-opacity` | sortable issue 卡片内 `[class~="bg-surface"]` | `* 100%` |
| 弹出菜单 | `--cbg-menu-opacity` | dialog/menu/listbox、`.bg-surface-raised` | `* 100%` |
| 底色遮罩 | overlay 设置 | `#multica-background-overlay` | 直接 opacity |
| 媒体 | `--cbg-opacity` × 路由强度 | `#multica-background-layer` | — |

所有滑杆范围均为 0..1，必须允许到 0。

## 新页面定位步骤

1. 进入目标页面并截图。
2. 从异常区域中心用 `document.elementsFromPoint()` 获取元素栈。
3. 找到第一个非透明背景、渐变、shadow 或 backdrop-filter。
4. 沿祖先链确认它是壳、卡片还是菜单项状态层。
5. 检查 `::before`、`::after`。
6. 先在 CDP 临时设置样式并截图对比。
7. 再把精确规则写入 `payload.ts` 的 `BACKGROUND_CSS`，同步 early 透明化（如需要）、
   测试断言，并删除探查文件。

## 视觉回归重点

- 标题栏与客户区背景连续，无断裂色带；原生三键可点。
- 侧栏壳透了之后，选中项和悬停项不能再冒实黑块。
- 看板卡片透明度独立于列 tint 和页面雾。
- 透明度为 0 时仍保留文字、图标、拖拽和按钮命中。
- 浅色主题不能继续使用深色 surface，深色主题不能回落到浅灰色。

## CDP 启动参数

```
--remote-debugging-address=127.0.0.1
--remote-debugging-port=9227
--remote-allow-origins=*
```

需要透明标题栏时额外：

```
--inspect-brk=127.0.0.1:<inspector-port>
```

Inspector 仅启动瞬间使用，补丁完成后必须关闭，不能长期挂着 debugger。
