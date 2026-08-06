import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import {
  ApplyRequest,
  AppSnapshot,
  BackgroundBridge,
  DEFAULT_SETTINGS,
  DownloadRequest,
  ImportResult,
  MediaItem,
  SettingsPatch,
} from "../shared/contracts";

const listeners = new Set<(snapshot: AppSnapshot) => void>();
const demoItem: MediaItem = {
  id: "00000000-0000-0000-0000-000000000001",
  name: "预览背景.jpg",
  kind: "image",
  origin: "local",
  fileName: "preview.jpg",
  mimeType: "image/jpeg",
  byteSize: 2_048_000,
  sha256: "demo",
  createdAt: new Date().toISOString(),
  previewUrl: "/@fs/D:/WORK/multica_desktop_background/reference/Notion-Dream-Skin/windows/assets/dream-reference.jpg",
};

let mockSnapshot: AppSnapshot = {
  settings: {
    ...structuredClone(DEFAULT_SETTINGS),
    activeMediaId: demoItem.id,
    playlistIds: [demoItem.id],
  },
  library: [demoItem],
  runtime: { phase: "idle", message: "尚未连接 Multica", activeTargets: 0 },
  dataDirectory: "D:\\MulticaBackgroundStudio",
};

const emit = () => listeners.forEach((listener) => listener(structuredClone(mockSnapshot)));
const emptyImport = async (): Promise<ImportResult> => ({ added: [], skipped: [] });

const mockBridge: BackgroundBridge = {
  getSnapshot: async () => structuredClone(mockSnapshot),
  chooseMediaFiles: emptyImport,
  chooseMediaFolder: emptyImport,
  addRemoteMedia: emptyImport,
  refreshMedia: async () => structuredClone(mockSnapshot),
  removeMedia: async (id) => {
    mockSnapshot.library = mockSnapshot.library.filter((item) => item.id !== id);
    if (mockSnapshot.settings.activeMediaId === id) mockSnapshot.settings.activeMediaId = null;
    emit();
    return structuredClone(mockSnapshot);
  },
  setActiveMedia: async (id) => {
    mockSnapshot.settings.activeMediaId = id;
    emit();
    return structuredClone(mockSnapshot);
  },
  updateSettings: async (patch: SettingsPatch) => {
    mockSnapshot.settings = {
      ...mockSnapshot.settings,
      ...patch,
      display: { ...mockSnapshot.settings.display, ...patch.display },
      slideshow: { ...mockSnapshot.settings.slideshow, ...patch.slideshow },
      behavior: { ...mockSnapshot.settings.behavior, ...patch.behavior },
    };
    emit();
    return structuredClone(mockSnapshot);
  },
  apply: async () => {
    mockSnapshot.runtime = { phase: "active", message: "背景已应用", activeTargets: 1 };
    emit();
    return structuredClone(mockSnapshot);
  },
  pause: async () => {
    mockSnapshot.runtime = { phase: "paused", message: "背景已暂停", activeTargets: 1 };
    emit();
    return structuredClone(mockSnapshot);
  },
  restore: async () => {
    mockSnapshot.runtime = { phase: "idle", message: "已恢复官方外观", activeTargets: 0 };
    emit();
    return structuredClone(mockSnapshot);
  },
  openDataDirectory: async () => undefined,
  showWindow: async () => undefined,
  onSnapshot: (listener) => {
    listeners.add(listener);
    return () => listeners.delete(listener);
  },
};

const isTauri = "__TAURI_INTERNALS__" in window;

const tauriBridge: BackgroundBridge = {
  getSnapshot: () => invoke<AppSnapshot>("get_snapshot"),
  chooseMediaFiles: () => invoke<ImportResult>("choose_media_files"),
  chooseMediaFolder: () => invoke<ImportResult>("choose_media_folder"),
  addRemoteMedia: (request: DownloadRequest) => invoke<ImportResult>("add_remote_media", { request }),
  refreshMedia: (id: string) => invoke<AppSnapshot>("refresh_media", { id }),
  removeMedia: (id: string) => invoke<AppSnapshot>("remove_media", { id }),
  setActiveMedia: (id: string) => invoke<AppSnapshot>("set_active_media", { id }),
  updateSettings: (patch: SettingsPatch) => invoke<AppSnapshot>("update_settings", { patch }),
  apply: (request?: ApplyRequest) => invoke<AppSnapshot>("apply_background", { request: request ?? null }),
  pause: () => invoke<AppSnapshot>("pause_background"),
  restore: () => invoke<AppSnapshot>("restore_background"),
  openDataDirectory: () => invoke<void>("open_data_directory"),
  showWindow: () => invoke<void>("show_window"),
  onSnapshot: (listener) => {
    let disposed = false;
    const pending = listen<AppSnapshot>("background:snapshot-changed", (event) => {
      if (!disposed) listener(event.payload);
    });
    return () => {
      disposed = true;
      void pending.then((unlisten) => unlisten());
    };
  },
};

if (!window.backgroundStudio && !isTauri && !import.meta.env.DEV) {
  throw new Error("安全预加载桥接未能启动，请重新安装 Multica Background Studio。");
}

export const bridge = window.backgroundStudio ?? (isTauri ? tauriBridge : mockBridge);
