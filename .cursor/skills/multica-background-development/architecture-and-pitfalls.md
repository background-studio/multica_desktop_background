# 架构与坑

## 启动与附着

1. 发现 `Multica.exe`（含 `@multicadesktop` 安装目录）。
2. 若已有未启用 CDP 的 Multica，要求用户确认重启。
3. 以 `--remote-debugging-address=127.0.0.1 --remote-debugging-port=9227` 启动或附着。
4. 校验 browser_id，只注入允许的 page target。

## 注入对象

- `#multica-background-style`：普通 DOM 样式。
- `#multica-background-layer`：固定背景媒体层。
- `#multica-background-media`：图片或视频。
- `#multica-background-tile`：平铺模式。
- `#multica-background-overlay`：整页底色。

## 安全边界

- 只连 `127.0.0.1`。
- browser_id 必须与 `runtime.json` 一致。
- 不修改 Multica 安装文件。
- 暂停/恢复必须完整 cleanup。
