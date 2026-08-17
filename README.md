# Multica Background Studio

[![org](https://img.shields.io/badge/org-background--studio-0ea5e9)](https://github.com/background-studio)
[![release](https://img.shields.io/github/v/release/background-studio/multica_desktop_background)](https://github.com/background-studio/multica_desktop_background/releases)

属于 [Background Studio](https://github.com/background-studio) 组织。这是给
[Background Studio 壳](https://github.com/background-studio/background-studio)
用的纯 Rust 无界面 worker：Named Pipe 协议 2，没有独立安装器，也没有常驻 UI。

协议见 [docs/plugin-protocol.md](./docs/plugin-protocol.md) 和根级 [plugin.json](./plugin.json)。

一个面向 Windows 官方 Multica 桌面应用的可逆背景 worker。它通过本机回环
Chromium DevTools Protocol 动态加载背景，不修改 `app.asar`、应用签名、
登录状态或页面数据。启动 Multica 时仍走透明 WCO 安全补丁；失败会恢复官方进程。

> 非 Multica 官方产品。Multica 及相关商标归其权利人所有。

## 功能

- 由壳 `configure` 下发回环媒体 URL 和 Multica 显示参数
- 图片覆盖、适应、拉伸和平铺
- 透明度、模糊、缩放、焦点位置、遮罩颜色与强度
- 侧栏、内容区、菜单、首页/任务页强度
- 自动接管新启动的官方 Multica，热更新已注入会话
- 一键暂停或完整恢复官方外观；`shutdown` 保留 Multica

支持的图片格式：PNG、JPEG、WebP、GIF、AVIF。

支持的视频容器：MP4、WebM、Ogg Video、QuickTime MOV。

## 开发

要求 Rust stable、Visual Studio Build Tools 的“使用 C++ 的桌面开发”工作负载，
以及已安装的官方 Multica 桌面应用
（默认路径 `%LOCALAPPDATA%\\Programs\\@multicadesktop\\Multica.exe`）。

```powershell
cargo fmt --manifest-path src-tauri/Cargo.toml
cargo test --manifest-path src-tauri/Cargo.toml
cargo build --release --manifest-path src-tauri/Cargo.toml
```

`cargo build --release` 产出 `src-tauri/target/release/multica-background-studio.exe`；发布 zip 会把它命名为 `Multica Background Studio.exe`。壳启动时仍可带 `--plugin`，worker 会忽略多余参数并直接进入 Named Pipe 服务。

## 发布

推送与 `src-tauri/Cargo.toml` 版本一致的 `v*` 标签会触发 GitHub Actions：
`cargo build --release`，然后上传
`MulticaBackgroundStudio-<version>-plugin.zip`（exe + `plugin.json` + 图标）。
不生成 NSIS。

维护 Multica 页面样式、CDP 注入或 WCO 启动前，请先阅读项目 Skill：
[`multica-background-development`](./.cursor/skills/multica-background-development/SKILL.md)。
