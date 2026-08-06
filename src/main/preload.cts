import { contextBridge, ipcRenderer } from "electron";
import type {
  ApplyRequest,
  AppSnapshot,
  BackgroundBridge,
  DownloadRequest,
  SettingsPatch,
} from "../shared/contracts.js";

const IPC = {
  getSnapshot: "background:get-snapshot",
  chooseFiles: "background:choose-files",
  chooseFolder: "background:choose-folder",
  addRemote: "background:add-remote",
  refreshMedia: "background:refresh-media",
  removeMedia: "background:remove-media",
  setActive: "background:set-active",
  updateSettings: "background:update-settings",
  apply: "background:apply",
  pause: "background:pause",
  restore: "background:restore",
  openDataDirectory: "background:open-data-directory",
  showWindow: "background:show-window",
  snapshotChanged: "background:snapshot-changed",
} as const;

const bridge: BackgroundBridge = {
  getSnapshot: () => ipcRenderer.invoke(IPC.getSnapshot),
  chooseMediaFiles: () => ipcRenderer.invoke(IPC.chooseFiles),
  chooseMediaFolder: () => ipcRenderer.invoke(IPC.chooseFolder),
  addRemoteMedia: (request: DownloadRequest) => ipcRenderer.invoke(IPC.addRemote, request),
  refreshMedia: (id: string) => ipcRenderer.invoke(IPC.refreshMedia, id),
  removeMedia: (id: string) => ipcRenderer.invoke(IPC.removeMedia, id),
  setActiveMedia: (id: string) => ipcRenderer.invoke(IPC.setActive, id),
  updateSettings: (patch: SettingsPatch) => ipcRenderer.invoke(IPC.updateSettings, patch),
  apply: (request?: ApplyRequest) => ipcRenderer.invoke(IPC.apply, request),
  pause: () => ipcRenderer.invoke(IPC.pause),
  restore: () => ipcRenderer.invoke(IPC.restore),
  openDataDirectory: () => ipcRenderer.invoke(IPC.openDataDirectory),
  showWindow: () => ipcRenderer.invoke(IPC.showWindow),
  onSnapshot: (listener: (snapshot: AppSnapshot) => void) => {
    const handler = (_event: Electron.IpcRendererEvent, snapshot: AppSnapshot) => listener(snapshot);
    ipcRenderer.on(IPC.snapshotChanged, handler);
    return () => ipcRenderer.off(IPC.snapshotChanged, handler);
  },
};

contextBridge.exposeInMainWorld("backgroundStudio", bridge);

