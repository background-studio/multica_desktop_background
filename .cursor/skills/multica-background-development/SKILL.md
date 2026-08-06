---
name: multica-background-development
description: Maintains Multica Background Studio, including CDP injection into Multica desktop, page transparency, media/slideshow behavior, debugging, packaging, and release verification. Use when changing this repository or investigating a Multica desktop UI surface that does not follow background settings.
---

# Multica Background Studio 开发

## 目标

维护 Windows 官方 Multica 桌面应用的可逆背景工具。通过本机 CDP 注入样式和媒体，
不修改 `app.asar`、应用签名、登录状态或页面数据。

动手前按需读取：

- 页面入口、DOM 特征、透明度归属：[windows-and-selectors.md](windows-and-selectors.md)
- 架构、故障经验、安全边界：[architecture-and-pitfalls.md](architecture-and-pitfalls.md)

## 不可破坏的原则

1. 不直接修改 Multica 安装资源；暂停和恢复必须能完整移除注入。
2. 不凭截图猜选择器。通过 Multica CDP 检查实际 DOM、计算样式。
3. 一个视觉区域只保留一层有色背景。内部壳层透明，避免半透明叠加变暗。
4. 所有界面不透明度必须允许 `0`；不要添加最低 0.2 之类的兜底。
5. 不用宽泛选择器清空所有背景。先确认稳定入口、尺寸、层级和页面复用范围。
6. `backdrop-filter` 默认关闭。
7. 动态组件必须首帧生效。
8. 一次性 CDP 探查脚本放在 `poc/`，验证完立即删除。
9. 修改 `BACKGROUND_CSS` 时必须进入修订哈希，确保现有会话热更新。
10. 更改共享设置时同步修改 contracts、默认值、规范化、UI、payload 和测试。

## 代码入口

- `src-tauri/src/lib.rs`：Tauri/Rust 主后端、命令、轮播和共享状态。
- `src-tauri/src/host.rs`：托盘、窗口生命周期、退出恢复和 Windows 自启动。
- `src-tauri/src/controller.rs`：发现官方 `Multica.exe`、校验进程、启动 Multica、保存和恢复 CDP 会话。
- `src-tauri/src/injector.rs`：Rust CDP target 同步、早期脚本、运行时更新、暂停和移除。
- `src/main/payload.ts`：Multica 页面背景层、CSS 变量、选择器和清理逻辑。
- `src/shared/contracts.ts`、`src-tauri/src/models.rs`：前端和 Rust 对应的数据契约。
- `src/renderer/App.tsx`：Studio 操作界面和设置控件。

## 安装发现

优先路径：

1. `%LOCALAPPDATA%\Programs\@multicadesktop\Multica.exe`
2. `%LOCALAPPDATA%\Programs\Multica\Multica.exe`
3. `%ProgramFiles%\Multica\Multica.exe`

首选调试口：`9227`。运行时状态：`%LOCALAPPDATA%\MulticaBackgroundStudio\runtime.json`。

## 完成条件

- 目标页面视觉结果符合对应透明度控制；
- 没有透明度叠加或首帧闪烁；
- 真实 Multica 验证完成；
- `npm run check` 全部通过；
- 临时探查文件已删除；
- 恢复流程仍完整可逆。
