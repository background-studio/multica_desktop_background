# Background Studio 插件协议

见组织规范：与 Codex / Notion 共用 `pluginProtocol: 1`。

本插件：

- 启动：`Multica Background Studio.exe --plugin`
- Pipe：`\\.\pipe\background-studio-multica`
- `pluginId`：`multica`
- Release 产物：`MulticaBackgroundStudio-<version>-plugin.zip`

## 插件模式自动接管

`--plugin` 只拉起后台托管 worker，不会自动打开官方 Multica。

- 已有有效调试会话：直接重连并注入上次背景。
- 启用后用户再普通启动 Multica：按完整 `Multica.exe` 路径确认，关闭该实例，走透明 WCO 启动路径带上 remote-debugging，再自动应用上次背景。
- 启用前已经在跑的普通进程：不自动关闭，状态为「Multica 已在运行，点立即接管可重启」；壳或 Studio 的 `apply` 才会重启接管。
- 目标退出后清理失效 engine/runtime，重新等待下一次启动。
- 带调试参数但会话 45 秒内未就绪：报错并等待进程退出后再武装，不会静默强杀。
- `pause` / `restore` 会暂停本插件进程内的 watcher；手动 `apply` 重新武装。
- 停用由壳结束插件进程，不改当前 Multica。

命令仍是 `status|open-ui|apply|pause|restore|quit-keep-target`。`status.message` 会反映等待启动、手动接管、正在接管、已自动应用、暂停托管或错误。

完整协议说明见壳仓
[background-studio/docs/plugin-protocol.md](https://github.com/background-studio/background-studio/blob/main/docs/plugin-protocol.md)。
