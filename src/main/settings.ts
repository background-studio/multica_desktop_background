import { promises as fs } from "node:fs";
import path from "node:path";
import {
  AppSettings,
  DEFAULT_SETTINGS,
  FitMode,
  SettingsPatch,
  SlideshowOrder,
} from "../shared/contracts.js";

const FIT_MODES = new Set<FitMode>(["cover", "contain", "fill", "tile"]);
const SLIDESHOW_ORDERS = new Set<SlideshowOrder>(["sequential", "random"]);

const clamp = (value: unknown, minimum: number, maximum: number, fallback: number) => {
  const parsed = Number(value);
  return Number.isFinite(parsed) ? Math.min(maximum, Math.max(minimum, parsed)) : fallback;
};

const asBoolean = (value: unknown, fallback: boolean) =>
  typeof value === "boolean" ? value : fallback;

const asHexColor = (value: unknown, fallback: string) =>
  typeof value === "string" && /^#[0-9a-f]{6}$/i.test(value) ? value.toLowerCase() : fallback;

export function normalizeSettings(value: unknown): AppSettings {
  const raw = value && typeof value === "object" ? value as Record<string, unknown> : {};
  const display = raw.display && typeof raw.display === "object"
    ? raw.display as Record<string, unknown>
    : {};
  const slideshow = raw.slideshow && typeof raw.slideshow === "object"
    ? raw.slideshow as Record<string, unknown>
    : {};
  const behavior = raw.behavior && typeof raw.behavior === "object"
    ? raw.behavior as Record<string, unknown>
    : {};
  const fit = FIT_MODES.has(display.fit as FitMode)
    ? display.fit as FitMode
    : DEFAULT_SETTINGS.display.fit;
  const order = SLIDESHOW_ORDERS.has(slideshow.order as SlideshowOrder)
    ? slideshow.order as SlideshowOrder
    : DEFAULT_SETTINGS.slideshow.order;
  const stringList = (candidate: unknown) => Array.isArray(candidate)
    ? [...new Set(candidate.filter((entry): entry is string => typeof entry === "string" && entry.length <= 120))]
    : [];

  return {
    schemaVersion: 1,
    activeMediaId: typeof raw.activeMediaId === "string" && raw.activeMediaId.length <= 120
      ? raw.activeMediaId
      : null,
    playlistIds: stringList(raw.playlistIds),
    display: {
      fit,
      positionX: clamp(display.positionX, 0, 100, DEFAULT_SETTINGS.display.positionX),
      positionY: clamp(display.positionY, 0, 100, DEFAULT_SETTINGS.display.positionY),
      opacity: clamp(display.opacity, 0, 1, DEFAULT_SETTINGS.display.opacity),
      blur: clamp(display.blur, 0, 40, DEFAULT_SETTINGS.display.blur),
      scale: clamp(display.scale, 1, 1.3, DEFAULT_SETTINGS.display.scale),
      overlayColor: asHexColor(display.overlayColor, DEFAULT_SETTINGS.display.overlayColor),
      overlayOpacity: clamp(display.overlayOpacity, 0, 0.9, DEFAULT_SETTINGS.display.overlayOpacity),
      blockFillOpacity: clamp(
        display.blockFillOpacity,
        0,
        1,
        DEFAULT_SETTINGS.display.blockFillOpacity,
      ),
      homeIntensity: clamp(display.homeIntensity, 0, 1, DEFAULT_SETTINGS.display.homeIntensity),
      taskIntensity: clamp(display.taskIntensity, 0, 1, DEFAULT_SETTINGS.display.taskIntensity),
      sidebarOpacity: clamp(display.sidebarOpacity, 0, 1, DEFAULT_SETTINGS.display.sidebarOpacity),
      surfaceOpacity: clamp(display.surfaceOpacity, 0, 1, DEFAULT_SETTINGS.display.surfaceOpacity),
      composerOpacity: clamp(display.composerOpacity, 0, 1, DEFAULT_SETTINGS.display.composerOpacity),
      menuOpacity: clamp(display.menuOpacity, 0, 1, DEFAULT_SETTINGS.display.menuOpacity),
      terminalOpacity: clamp(display.terminalOpacity, 0, 1, DEFAULT_SETTINGS.display.terminalOpacity),
      enabledOnHome: asBoolean(display.enabledOnHome, DEFAULT_SETTINGS.display.enabledOnHome),
      enabledOnTasks: asBoolean(display.enabledOnTasks, DEFAULT_SETTINGS.display.enabledOnTasks),
      videoMuted: asBoolean(display.videoMuted, DEFAULT_SETTINGS.display.videoMuted),
      videoPlaybackRate: clamp(display.videoPlaybackRate, 0.25, 2, DEFAULT_SETTINGS.display.videoPlaybackRate),
    },
    slideshow: {
      enabled: asBoolean(slideshow.enabled, DEFAULT_SETTINGS.slideshow.enabled),
      intervalSeconds: Math.round(clamp(
        slideshow.intervalSeconds,
        10,
        86400,
        DEFAULT_SETTINGS.slideshow.intervalSeconds,
      )),
      order,
    },
    behavior: {
      closeToTray: asBoolean(behavior.closeToTray, DEFAULT_SETTINGS.behavior.closeToTray),
      startMinimized: asBoolean(behavior.startMinimized, DEFAULT_SETTINGS.behavior.startMinimized),
      autoStartWithWindows: asBoolean(
        behavior.autoStartWithWindows,
        DEFAULT_SETTINGS.behavior.autoStartWithWindows,
      ),
      launchMulticaOnApply: asBoolean(
        behavior.launchMulticaOnApply,
        DEFAULT_SETTINGS.behavior.launchMulticaOnApply,
      ),
    },
  };
}

export function mergeSettings(settings: AppSettings, patch: SettingsPatch): AppSettings {
  return normalizeSettings({
    ...settings,
    ...patch,
    display: { ...settings.display, ...patch.display },
    slideshow: { ...settings.slideshow, ...patch.slideshow },
    behavior: { ...settings.behavior, ...patch.behavior },
  });
}

async function writeJsonTransaction(filePath: string, value: unknown) {
  const directory = path.dirname(filePath);
  const temporary = `${filePath}.${process.pid}.${Date.now()}.tmp`;
  const backup = `${filePath}.bak`;
  await fs.mkdir(directory, { recursive: true });
  const handle = await fs.open(temporary, "wx");
  try {
    await handle.writeFile(`${JSON.stringify(value, null, 2)}\n`, "utf8");
    await handle.sync();
  } finally {
    await handle.close();
  }
  let hadOriginal = false;
  try {
    await fs.rm(backup, { force: true });
    try {
      await fs.rename(filePath, backup);
      hadOriginal = true;
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") throw error;
    }
    await fs.rename(temporary, filePath);
    await fs.rm(backup, { force: true });
  } catch (error) {
    await fs.rm(temporary, { force: true });
    if (hadOriginal) {
      await fs.rm(filePath, { force: true });
      await fs.rename(backup, filePath);
    }
    throw error;
  }
}

export class SettingsStore {
  readonly filePath: string;
  #settings: AppSettings = structuredClone(DEFAULT_SETTINGS);

  constructor(dataDirectory: string) {
    this.filePath = path.join(dataDirectory, "settings.json");
  }

  get value() {
    return structuredClone(this.#settings);
  }

  async load() {
    try {
      const raw = JSON.parse(await fs.readFile(this.filePath, "utf8"));
      this.#settings = normalizeSettings(raw);
    } catch (error) {
      if ((error as NodeJS.ErrnoException).code !== "ENOENT") {
        const invalid = `${this.filePath}.invalid-${Date.now()}`;
        await fs.rename(this.filePath, invalid).catch(() => undefined);
      }
      this.#settings = structuredClone(DEFAULT_SETTINGS);
      await this.save(this.#settings);
    }
    return this.value;
  }

  async save(settings: AppSettings) {
    this.#settings = normalizeSettings(settings);
    await writeJsonTransaction(this.filePath, this.#settings);
    return this.value;
  }

  async patch(patch: SettingsPatch) {
    return this.save(mergeSettings(this.#settings, patch));
  }
}
