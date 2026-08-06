import { createHash } from "node:crypto";
import { promises as fsPromises } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  app,
  BrowserWindow,
  dialog,
  ipcMain,
  Menu,
  nativeImage,
  shell,
  Tray,
} from "electron";
import type { MessageBoxOptions } from "electron";
import {
  AppSnapshot,
  ApplyRequest,
  DownloadRequest,
  IPC,
  SettingsPatch,
} from "../shared/contracts.js";
import { MulticaController } from "./multica-controller.js";
import { MediaLibrary } from "./media-library.js";
import { MediaServer } from "./media-server.js";
import { buildRendererPayload } from "./payload.js";
import { SettingsStore } from "./settings.js";

const here = path.dirname(fileURLToPath(import.meta.url));
const localAppData = process.env.LOCALAPPDATA || app.getPath("appData");
const dataDirectory = path.join(localAppData, "MulticaBackgroundStudio");
const applicationIconPath = path.join(here, "../../assets/icon.png");
app.setPath("userData", path.join(dataDirectory, "electron"));

let mainWindow: BrowserWindow | null = null;
let tray: Tray | null = null;
let quitting = false;
let slideshowTimer: NodeJS.Timeout | null = null;

const settingsStore = new SettingsStore(dataDirectory);
const library = new MediaLibrary(dataDirectory);
const mediaServer = new MediaServer(library);
const controller = new MulticaController(dataDirectory, () => {
  broadcastSnapshot();
  rebuildTray();
});

function snapshot(): AppSnapshot {
  const settings = settingsStore.value;
  // 带内容哈希做查询参数：随机 API 刷新后内容变了但 URL 不变，需要让预览图重新加载
  const items = library.items.map((item) => ({
    ...item,
    previewUrl: `${mediaServer.urlFor(item.id)}?v=${item.sha256.slice(0, 12)}`,
  }));
  return {
    settings,
    library: items,
    runtime: controller.status,
    dataDirectory,
  };
}

function broadcastSnapshot() {
  if (!mainWindow || mainWindow.isDestroyed()) return;
  mainWindow.webContents.send(IPC.snapshotChanged, snapshot());
}

const MAX_INLINE_MEDIA_BYTES = 64 * 1024 * 1024;

async function activePayload() {
  const settings = settingsStore.value;
  const media = settings.activeMediaId ? library.getById(settings.activeMediaId) : null;
  if (!media) throw new Error("请先从媒体库选择一张图片或一个视频。");
  // Multica 渲染页无法访问本机 HTTP 服务（回环 fetch 被沙箱拦截），
  // 因此媒体统一以 base64 内嵌进注入脚本
  if (media.byteSize > MAX_INLINE_MEDIA_BYTES) {
    throw new Error("背景媒体超过 64 MB 内嵌上限，请选择更小的文件。");
  }
  const bytes = await fsPromises.readFile(library.pathFor(media));
  const input = {
    mediaUrl: `data:${media.mimeType};base64,${bytes.toString("base64")}`,
    mediaKind: media.kind,
    display: settings.display,
  };
  const revision = createHash("sha256")
    .update(JSON.stringify({ sha256: media.sha256, display: settings.display, kind: media.kind }))
    .digest("hex");
  return {
    payload: buildRendererPayload({ ...input, revision }),
    revision,
  };
}

async function applyBackground(request: ApplyRequest = {}) {
  const { payload, revision } = await activePayload();
  try {
    await controller.apply(payload, revision, Boolean(request.restartExisting));
  } catch (error) {
    if ((error as Error).message.includes("需要重启一次") && !request.restartExisting) {
      const options: MessageBoxOptions = {
        type: "question",
        title: "重启 Multica",
        message: "应用背景需要重启一次 Multica",
        detail: "未发送的输入可能丢失。背景管理器只会关闭经过官方 Store 包身份校验的 Multica 进程。",
        buttons: ["重启并应用", "取消"],
        defaultId: 0,
        cancelId: 1,
        noLink: true,
      };
      const result = mainWindow
        ? await dialog.showMessageBox(mainWindow, options)
        : await dialog.showMessageBox(options);
      if (result.response === 0) await controller.apply(payload, revision, true);
    } else {
      throw error;
    }
  }
  scheduleSlideshow();
  broadcastSnapshot();
  rebuildTray();
  return snapshot();
}

async function applyLiveIfActive() {
  if (controller.status.phase !== "active") return;
  const { payload, revision } = await activePayload();
  await controller.apply(payload, revision, false);
}

function scheduleSlideshow() {
  if (slideshowTimer) clearTimeout(slideshowTimer);
  slideshowTimer = null;
  const settings = settingsStore.value;
  const available = settings.playlistIds.filter((id) => library.getById(id));
  if (!settings.slideshow.enabled || controller.status.phase !== "active" || available.length === 0) return;
  // 随机 API 条目单独一个也能轮播：每次触发重新请求得到新图
  const hasDynamic = available.some((id) => library.getById(id)?.origin === "api");
  if (available.length < 2 && !hasDynamic) return;
  slideshowTimer = setTimeout(async () => {
    try {
      const currentIndex = Math.max(0, available.indexOf(settings.activeMediaId ?? ""));
      let nextIndex = available.length > 1 ? (currentIndex + 1) % available.length : currentIndex;
      if (settings.slideshow.order === "random" && available.length > 1) {
        const choices = available.map((_, index) => index).filter((index) => index !== currentIndex);
        nextIndex = choices[Math.floor(Math.random() * choices.length)];
      }
      const nextId = available[nextIndex];
      if (library.getById(nextId)?.origin === "api") {
        // 网络抖动时保留当前缓存的图片继续显示，不打断轮播、不弹窗
        await library.refreshDynamic(nextId).catch(() => undefined);
      }
      await settingsStore.patch({ activeMediaId: nextId });
      await applyLiveIfActive();
      broadcastSnapshot();
    } catch (error) {
      dialog.showErrorBox("轮播切换失败", (error as Error).message);
    } finally {
      scheduleSlideshow();
    }
  }, settings.slideshow.intervalSeconds * 1000);
}

async function integrateImport(result: Awaited<ReturnType<MediaLibrary["importFiles"]>>) {
  if (result.added.length > 0) {
    const settings = settingsStore.value;
    const newIds = result.added.map((item) => item.id);
    await settingsStore.patch({
      activeMediaId: settings.activeMediaId ?? newIds[0],
      playlistIds: [...new Set([...settings.playlistIds, ...newIds])],
    });
    await applyLiveIfActive();
  }
  broadcastSnapshot();
  rebuildTray();
  return result;
}

function showMainWindow() {
  if (!mainWindow) return;
  mainWindow.show();
  if (mainWindow.isMinimized()) mainWindow.restore();
  mainWindow.focus();
}

function rebuildTray() {
  if (!tray) return;
  const status = controller.status;
  tray.setToolTip(`Multica Background Studio · ${status.message}`);
  tray.setContextMenu(Menu.buildFromTemplate([
    { label: `状态：${status.message}`, enabled: false },
    { type: "separator" },
    { label: "打开背景管理器", click: showMainWindow },
    { label: "应用或重新应用", click: () => void applyBackground().catch(showError) },
    { label: "暂停背景", enabled: status.phase === "active", click: () => void controller.pause().then(() => {
      broadcastSnapshot(); rebuildTray();
    }).catch(showError) },
    { label: "恢复官方外观", click: () => void controller.restore().then(() => {
      broadcastSnapshot(); rebuildTray();
    }).catch(showError) },
    { type: "separator" },
    { label: "退出并恢复 Multica", click: () => void quitAndRestore() },
  ]));
}

function showError(error: unknown) {
  dialog.showErrorBox("Multica Background Studio", (error as Error).message || String(error));
}

async function quitAndRestore() {
  if (quitting) return;
  quitting = true;
  try {
    if (["active", "paused", "error"].includes(controller.status.phase)) await controller.restore();
  } catch (error) {
    const options: MessageBoxOptions = {
      type: "warning",
      title: "恢复未完成",
      message: "退出前未能完整恢复 Multica",
      detail: (error as Error).message,
      buttons: ["返回管理器", "仍然退出"],
      defaultId: 0,
      cancelId: 0,
      noLink: true,
    };
    const result = mainWindow
      ? await dialog.showMessageBox(mainWindow, options)
      : await dialog.showMessageBox(options);
    if (result.response === 0) { quitting = false; showMainWindow(); return; }
  }
  await mediaServer.stop();
  app.quit();
}

function registerIpc() {
  ipcMain.handle(IPC.getSnapshot, () => snapshot());
  ipcMain.handle(IPC.chooseFiles, async () => {
    const result = await dialog.showOpenDialog(mainWindow!, {
      title: "选择背景图片或视频",
      properties: ["openFile", "multiSelections"],
      filters: [
        { name: "图片和视频", extensions: ["png", "jpg", "jpeg", "webp", "gif", "avif", "mp4", "webm", "ogv", "mov"] },
        { name: "所有文件", extensions: ["*"] },
      ],
    });
    return result.canceled ? { added: [], skipped: [] } : integrateImport(await library.importFiles(result.filePaths));
  });
  ipcMain.handle(IPC.chooseFolder, async () => {
    const result = await dialog.showOpenDialog(mainWindow!, {
      title: "导入背景文件夹",
      properties: ["openDirectory"],
    });
    if (result.canceled) return { added: [], skipped: [] };
    const files = await library.discoverFolder(result.filePaths[0]);
    return integrateImport(await library.importFiles(files));
  });
  ipcMain.handle(IPC.addRemote, async (_event, request: DownloadRequest) => {
    if (!request || typeof request.url !== "string" || request.url.length > 4096) throw new Error("网络地址无效。");
    return integrateImport(await library.importRemote(request.url, { dynamic: Boolean(request.dynamic) }));
  });
  ipcMain.handle(IPC.refreshMedia, async (_event, id: string) => {
    if (typeof id !== "string") throw new Error("媒体项目不存在。");
    await library.refreshDynamic(id);
    if (settingsStore.value.activeMediaId === id) await applyLiveIfActive();
    broadcastSnapshot();
    return snapshot();
  });
  ipcMain.handle(IPC.removeMedia, async (_event, id: string) => {
    const removed = library.getById(id);
    await library.remove(id);
    const settings = settingsStore.value;
    const playlistIds = settings.playlistIds.filter((candidate) => candidate !== id);
    const activeMediaId = settings.activeMediaId === id ? playlistIds[0] ?? library.items[0]?.id ?? null : settings.activeMediaId;
    await settingsStore.patch({ playlistIds, activeMediaId });
    if (removed && controller.status.phase === "active" && activeMediaId) await applyLiveIfActive();
    broadcastSnapshot();
    rebuildTray();
    return snapshot();
  });
  ipcMain.handle(IPC.setActive, async (_event, id: string) => {
    if (!library.getById(id)) throw new Error("媒体项目不存在。");
    const settings = settingsStore.value;
    await settingsStore.patch({
      activeMediaId: id,
      playlistIds: settings.playlistIds.includes(id) ? settings.playlistIds : [...settings.playlistIds, id],
    });
    await applyLiveIfActive();
    scheduleSlideshow();
    broadcastSnapshot();
    return snapshot();
  });
  ipcMain.handle(IPC.updateSettings, async (_event, patch: SettingsPatch) => {
    await settingsStore.patch(patch);
    const settings = settingsStore.value;
    app.setLoginItemSettings({
      openAtLogin: settings.behavior.autoStartWithWindows,
      args: settings.behavior.startMinimized ? ["--hidden"] : [],
    });
    await applyLiveIfActive();
    scheduleSlideshow();
    broadcastSnapshot();
    rebuildTray();
    return snapshot();
  });
  ipcMain.handle(IPC.apply, (_event, request?: ApplyRequest) => applyBackground(request));
  ipcMain.handle(IPC.pause, async () => {
    await controller.pause();
    scheduleSlideshow();
    broadcastSnapshot();
    rebuildTray();
    return snapshot();
  });
  ipcMain.handle(IPC.restore, async () => {
    await controller.restore();
    scheduleSlideshow();
    broadcastSnapshot();
    rebuildTray();
    return snapshot();
  });
  ipcMain.handle(IPC.openDataDirectory, async () => { await shell.openPath(dataDirectory); });
  ipcMain.handle(IPC.showWindow, () => showMainWindow());
}

async function createWindow() {
  mainWindow = new BrowserWindow({
    width: 1240,
    height: 800,
    minWidth: 980,
    minHeight: 640,
    show: false,
    backgroundColor: "#f4f6f5",
    title: "Multica Background Studio",
    icon: nativeImage.createFromPath(applicationIconPath),
    autoHideMenuBar: true,
    webPreferences: {
      preload: path.join(here, "preload.cjs"),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: true,
    },
  });
  const devUrl = process.env.VITE_DEV_SERVER_URL;
  if (devUrl) await mainWindow.loadURL(devUrl);
  else await mainWindow.loadFile(path.join(here, "../../dist/index.html"));
  mainWindow.on("close", (event) => {
    if (!quitting && settingsStore.value.behavior.closeToTray) {
      event.preventDefault();
      mainWindow?.hide();
    }
  });
  mainWindow.on("closed", () => { mainWindow = null; });
  if (!settingsStore.value.behavior.startMinimized && !process.argv.includes("--hidden")) mainWindow.show();
}

app.whenReady().then(async () => {
  await Promise.all([settingsStore.load(), library.load(), controller.initialize()]);
  await mediaServer.start();
  registerIpc();
  await createWindow();
  const icon = nativeImage.createFromPath(applicationIconPath).resize({ width: 16, height: 16 });
  tray = new Tray(icon);
  tray.on("double-click", showMainWindow);
  rebuildTray();
  scheduleSlideshow();
  app.on("activate", showMainWindow);
}).catch((error) => {
  showError(error);
  app.quit();
});

app.on("window-all-closed", () => {
  if (process.platform !== "darwin" && !settingsStore.value.behavior.closeToTray) void quitAndRestore();
});

app.on("before-quit", (event) => {
  if (!quitting) {
    event.preventDefault();
    void quitAndRestore();
  }
});
