# Background Studio 插件协议

本插件是纯 Rust 无界面 worker，只由 Background Studio 壳启动。

- `pluginProtocol`: `2`
- `pluginId`: `multica`
- 可执行文件：`Multica Background Studio.exe`
- Pipe：`\\.\pipe\background-studio-multica`
- 清单：根级 `plugin.json`
- Release 产物：`MulticaBackgroundStudio-<version>-plugin.zip`

## 传输

Named Pipe，每行一个 JSON。

请求：

```json
{"id":"...","cmd":"...","params":{}}
```

成功：`{"id":"...","ok":true,"result":{...}}`

失败：`{"id":"...","ok":false,"error":"..."}`

## 命令

| 命令 | 作用 |
|------|------|
| `hello` | 返回 `pluginProtocol:2`、`pluginId`、`version`、`capabilities` |
| `configure` | 校验并下载回环媒体，保存最近一次有效配置；若当前已 active 则热更新 |
| `status` | `phase` / `message` / `activeTargets` / `paused` / `configured` / `revision` |
| `apply` | 使用最近一次有效 configure，手动接管并重新武装 watcher |
| `pause` | 暂停注入并暂停本进程内 watcher |
| `restore` | 恢复官方外观并暂停 watcher |
| `shutdown` | 结束 worker，不改当前 Multica；返回 `shutdown:true`、`keptTarget:true` |

`configure.params`：

```json
{
  "schemaVersion": 1,
  "revision": "...",
  "media": {
    "url": "http://127.0.0.1:<port>/...",
    "kind": "image",
    "mimeType": "image/png",
    "sha256": "<64 hex>",
    "byteSize": 123
  },
  "display": {}
}
```

`hello.capabilities.maxMediaBytes` 与 Manifest 都声明 64 MiB。媒体 URL 仅允许 `http://127.0.0.1` 或 `http://localhost`，拒绝 userinfo、fragment、非回环、端口 0 和超长 JSON。下载使用 `no_proxy`，并校验 `Content-Type`、`Content-Length`、`byteSize`、`sha256`、`mimeType`/`kind`。

未配置时 watcher 只等待并报告「尚未配置背景」，不会杀进程。

## 自动接管

启用后不会自动打开官方 Multica。

- 已有有效调试会话：配置后可重连并注入。
- 配置后用户再普通启动 Multica：按完整 `Multica.exe` 路径确认，关闭该实例，走透明 WCO 启动路径，再自动应用当前配置。
- 配置前已经在跑的普通进程：不自动关闭；`apply` 才会重启接管。
- `pause` / `restore` 会暂停本插件进程内的 watcher；手动 `apply` 重新武装。
- `shutdown` 或壳停用插件只结束 worker，不改当前 Multica。
