# Multica 窗口与选择器

选择器经 CDP 复核（Electron renderer `file://.../out/renderer/index.html`）。

## 目标 page

- 主界面：`file://.../Programs/@multicadesktop/.../out/renderer/index.html`
  （或日后 `Programs/Multica/...`）
- 可选：`https://multica.ai` / `https://*.multica.ai`（若出现独立 webview）

## 注入根标记

- `html.multica-background-active`
- 路由：`multica-background-home` / `multica-background-task`
- 媒体层：`#multica-background-layer` / `#multica-background-media` / `#multica-background-overlay`
- 主题：`html.light` / `html.dark`（探测后设 `multica-background-dark`）

## 透明度归属

| 控制 | 稳定入口 |
|------|----------|
| `sidebarOpacity` | `[data-sidebar="sidebar"]`、`[data-slot="sidebar-inner"]` |
| `surfaceOpacity` | `.bg-page-canvas`、`header`、`[data-slot="card"]`、`[data-slot="chat-input-surface"]` |
| `menuOpacity` | `[role="dialog"|"menu"|"listbox"]`、`.bg-surface-raised` |
| `overlayColor` + `overlayOpacity` | `#multica-background-overlay` |
| 媒体 | `#multica-background-layer` |

外层打底：`.bg-app-shell`、`[data-slot="sidebar-wrapper"]` 透明，避免叠暗。

## CDP 启动

```
--remote-debugging-address=127.0.0.1
--remote-debugging-port=9227
--remote-allow-origins=*
```
