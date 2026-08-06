export type MediaKind = "image" | "video";
// api: 随机图片接口，每次请求返回不同内容，轮播/刷新时重新拉取
// folder: 只记录本地目录路径，轮播/应用时再按需挑选其中一张，不复制入库
export type MediaOrigin = "local" | "remote" | "api" | "folder";
export type FitMode = "cover" | "contain" | "fill" | "tile";
export type SlideshowOrder = "sequential" | "random";

export interface MediaItem {
  id: string;
  name: string;
  kind: MediaKind;
  origin: MediaOrigin;
  fileName: string;
  mimeType: string;
  byteSize: number;
  sha256: string;
  sourceUrl?: string;
  /** 文件夹源内当前扫描到的媒体数量（不复制文件） */
  fileCount?: number;
  createdAt: string;
  previewUrl?: string;
}

export interface DisplaySettings {
  fit: FitMode;
  positionX: number;
  positionY: number;
  opacity: number;
  blur: number;
  scale: number;
  overlayColor: string;
  overlayOpacity: number;
  /** 预留字段（Multica 首版 UI 不展示） */
  blockFillOpacity: number;
  homeIntensity: number;
  taskIntensity: number;
  sidebarOpacity: number;
  surfaceOpacity: number;
  cardOpacity: number;
  composerOpacity: number;
  menuOpacity: number;
  terminalOpacity: number;
  enabledOnHome: boolean;
  enabledOnTasks: boolean;
  videoMuted: boolean;
  videoPlaybackRate: number;
}

export interface SlideshowSettings {
  enabled: boolean;
  intervalSeconds: number;
  order: SlideshowOrder;
}

export interface BehaviorSettings {
  closeToTray: boolean;
  startMinimized: boolean;
  autoStartWithWindows: boolean;
  launchMulticaOnApply: boolean;
}

export interface AppSettings {
  schemaVersion: 1;
  activeMediaId: string | null;
  playlistIds: string[];
  display: DisplaySettings;
  slideshow: SlideshowSettings;
  behavior: BehaviorSettings;
}

export type RuntimePhase =
  | "idle"
  | "starting"
  | "active"
  | "paused"
  | "restoring"
  | "error";

export interface RuntimeStatus {
  phase: RuntimePhase;
  message: string;
  multicaVersion?: string;
  activeTargets: number;
  lastError?: string;
}

export interface AppSnapshot {
  settings: AppSettings;
  library: MediaItem[];
  runtime: RuntimeStatus;
  dataDirectory: string;
}

export interface ImportResult {
  added: MediaItem[];
  skipped: Array<{ path: string; reason: string }>;
}

export interface DownloadRequest {
  url: string;
  /** true 表示这是随机图片 API，作为动态源保存 */
  dynamic?: boolean;
}

export interface ApplyRequest {
  restartExisting?: boolean;
}

export type SettingsPatch = Partial<Pick<AppSettings, "activeMediaId" | "playlistIds">> & {
  display?: Partial<DisplaySettings>;
  slideshow?: Partial<SlideshowSettings>;
  behavior?: Partial<BehaviorSettings>;
};

export interface BackgroundBridge {
  getSnapshot(): Promise<AppSnapshot>;
  chooseMediaFiles(): Promise<ImportResult>;
  chooseMediaFolder(): Promise<ImportResult>;
  addRemoteMedia(request: DownloadRequest): Promise<ImportResult>;
  refreshMedia(id: string): Promise<AppSnapshot>;
  removeMedia(id: string): Promise<AppSnapshot>;
  setActiveMedia(id: string): Promise<AppSnapshot>;
  updateSettings(patch: SettingsPatch): Promise<AppSnapshot>;
  apply(request?: ApplyRequest): Promise<AppSnapshot>;
  pause(): Promise<AppSnapshot>;
  restore(): Promise<AppSnapshot>;
  openDataDirectory(): Promise<void>;
  showWindow(): Promise<void>;
  onSnapshot(listener: (snapshot: AppSnapshot) => void): () => void;
}

export const DEFAULT_SETTINGS: AppSettings = {
  schemaVersion: 1,
  activeMediaId: null,
  playlistIds: [],
  display: {
    fit: "cover",
    positionX: 50,
    positionY: 50,
    opacity: 0.72,
    blur: 0,
    scale: 1,
    overlayColor: "#101416",
    overlayOpacity: 0.12,
    blockFillOpacity: 0.55,
    homeIntensity: 1,
    taskIntensity: 0.32,
    sidebarOpacity: 0.18,
    surfaceOpacity: 0.12,
    cardOpacity: 0.35,
    composerOpacity: 0.88,
    menuOpacity: 0.9,
    terminalOpacity: 0.9,
    enabledOnHome: true,
    enabledOnTasks: true,
    videoMuted: true,
    videoPlaybackRate: 1,
  },
  slideshow: {
    enabled: false,
    intervalSeconds: 300,
    order: "sequential",
  },
  behavior: {
    closeToTray: true,
    startMinimized: false,
    autoStartWithWindows: false,
    launchMulticaOnApply: true,
  },
};

export const IPC = {
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

