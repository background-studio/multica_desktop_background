# Multica Background Studio

[![org](https://img.shields.io/badge/org-background--studio-0ea5e9)](https://github.com/background-studio)
[![release](https://img.shields.io/github/v/release/background-studio/multica_desktop_background)](https://github.com/background-studio/multica_desktop_background/releases)

属于 [Background Studio](https://github.com/background-studio) 组织。可独立安装，也可作为
[Background Studio 壳](https://github.com/background-studio/background-studio) 的插件
（`--plugin` + `*-plugin.zip`，见 [docs/plugin-protocol.md](./docs/plugin-protocol.md)）。

一个面向 Windows 官方 Multica 桌面应用的独立背景管理器。它通过本机回环
Chromium DevTools Protocol 动态加载背景，不修改 `app.asar`、应用签名、
登录状态或页面数据。

管理器采用 Tauri 2、Rust 和系统 WebView2。

> 非 Multica 官方产品。Multica 及相关商标归其权利人所有。

## 功能

- 导入本地图片、视频或整个文件夹
- 下载 HTTP/HTTPS 网络图片和视频并纳入受管媒体库
- 图片覆盖、适应、拉伸和平铺
- 透明度、模糊、缩放、焦点位置、遮罩颜色与强度
- 侧栏、内容区、菜单不透明度控制
- 顺序或随机轮播，自定义切换间隔与播放列表
- 实时预览、热更新、系统托盘、Windows 自启动
- 一键暂停或完整恢复官方外观

支持的图片格式：PNG、JPEG、WebP、GIF、AVIF。

支持的视频容器：MP4、WebM、Ogg Video、QuickTime MOV。

## 开发

要求 Node.js 22 或更高版本、Rust stable、Visual Studio Build Tools 的
“使用 C++ 的桌面开发”工作负载，以及已安装的官方 Multica 桌面应用
（默认路径 `%LOCALAPPDATA%\Programs\@multicadesktop\Multica.exe`）。
Windows 10/11 还需 WebView2 Runtime。

```powershell
npm install
npm run check
npm run dev
```

只预览界面：

```powershell
npm run dev:web
```

构建 Windows 安装包：

```powershell
npm run package:win
```

NSIS 产物位于 `src-tauri/target/release/bundle/nsis/`。

## 发布

推送与应用版本一致的 `v*` 标签会触发 GitHub Actions，在 Windows runner 上执行
完整检查、构建 NSIS 安装包，并创建正式 GitHub Release：

```powershell
git tag v0.2.0
git push origin v0.2.0
```

工作流会核对 `package.json`、`src-tauri/Cargo.toml`、`tauri.conf.json` 和标签版本，
任一不一致都会停止发布。Release 同时上传 NSIS 与
`MulticaBackgroundStudio-<version>-plugin.zip`。

维护 Multica 页面样式、CDP 注入或媒体流程前，请先阅读项目 Skill：
[`multica-background-development`](./.cursor/skills/multica-background-development/SKILL.md)。

## 安全边界

- 仅连接 `127.0.0.1` 回环调试口
- 仅向 `https://multica.ai/` / `https://www.multica.ai/` 页面注入
- 不修改 Multica 安装资源；暂停和恢复必须能完整移除注入
- 应用背景时如 Notion 未开启调试口，需要重启一次 Multica

## 版本

当前版本：`0.2.0`。
