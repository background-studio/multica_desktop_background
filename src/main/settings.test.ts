import { mkdtemp, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS } from "../shared/contracts.js";
import { mergeSettings, normalizeSettings, SettingsStore } from "./settings.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map((directory) => rm(directory, { recursive: true, force: true })));
});

describe("normalizeSettings", () => {
  it("clamps numeric fields and rejects invalid enum/color values", () => {
    const settings = normalizeSettings({
      activeMediaId: "media-1",
      display: {
        fit: "invalid",
        opacity: 9,
        blur: -4,
        overlayColor: "red; background:url(x)",
        positionX: 41,
      },
      slideshow: { intervalSeconds: 2, order: "sideways" },
    });
    expect(settings.activeMediaId).toBe("media-1");
    expect(settings.display.fit).toBe(DEFAULT_SETTINGS.display.fit);
    expect(settings.display.opacity).toBe(1);
    expect(settings.display.blur).toBe(0);
    expect(settings.display.positionX).toBe(41);
    expect(settings.display.overlayColor).toBe(DEFAULT_SETTINGS.display.overlayColor);
    expect(settings.slideshow.intervalSeconds).toBe(10);
    expect(settings.slideshow.order).toBe("sequential");
  });

  it("merges scoped patches without resetting unrelated controls", () => {
    const updated = mergeSettings(DEFAULT_SETTINGS, { display: { opacity: 0.35 }, slideshow: { enabled: true } });
    expect(updated.display.opacity).toBe(0.35);
    expect(updated.display.blur).toBe(DEFAULT_SETTINGS.display.blur);
    expect(updated.slideshow.enabled).toBe(true);
    expect(updated.behavior).toEqual(DEFAULT_SETTINGS.behavior);
  });

  it("allows every interface opacity control to reach zero", () => {
    const settings = normalizeSettings({
      display: {
        sidebarOpacity: 0,
        surfaceOpacity: 0,
        composerOpacity: 0,
        menuOpacity: 0,
        terminalOpacity: 0,
      },
    });
    expect(settings.display.sidebarOpacity).toBe(0);
    expect(settings.display.surfaceOpacity).toBe(0);
    expect(settings.display.composerOpacity).toBe(0);
    expect(settings.display.menuOpacity).toBe(0);
    expect(settings.display.terminalOpacity).toBe(0);
  });
});

describe("SettingsStore", () => {
  it("round-trips UTF-8 data through transactional writes", async () => {
    const directory = await mkdtemp(path.join(os.tmpdir(), "multica-background-settings-"));
    temporaryDirectories.push(directory);
    const store = new SettingsStore(directory);
    await store.load();
    await store.patch({ activeMediaId: "背景-一号", display: { opacity: 0.48 } });
    const reopened = new SettingsStore(directory);
    await reopened.load();
    expect(reopened.value.activeMediaId).toBe("背景-一号");
    expect(reopened.value.display.opacity).toBe(0.48);
  });
});

