import { describe, expect, it } from "vitest";
import { DEFAULT_SETTINGS } from "../shared/contracts.js";
import { buildRendererPayload, earlyPayloadFor, REMOVE_RENDERER_PAYLOAD } from "./payload.js";

describe("renderer payload", () => {
  it("contains Multica shell selectors and an inert decorative layer", () => {
    const payload = buildRendererPayload({
      mediaUrl: "http://127.0.0.1:9444/token/media/id",
      mediaKind: "video",
      display: DEFAULT_SETTINGS.display,
      revision: "revision-1",
    });
    expect(payload).toContain("multica-background-layer");
    expect(payload).toContain("multica-background-media");
    expect(payload).toContain("multica-background-tile");
    expect(payload).toContain("multica-background-overlay");
    expect(payload).toContain("pointer-events: none");
    expect(payload).toContain("#root");
    expect(payload).toContain('[data-sidebar="sidebar"]');
    expect(payload).toContain(".bg-page-canvas");
    expect(payload).toContain(".bg-page-canvas .bg-background");
    expect(payload).toContain(".bg-card");
    expect(payload).toContain('[data-slot="sidebar-inner"]');
    expect(payload).toContain('[data-sidebar="menu-button"][data-active]');
    expect(payload).toContain('[data-sidebar="menu-button"]:hover');
    expect(payload).toContain('[data-sidebar="menu-button"]:focus-visible');
    expect(payload).toContain("--cbg-sidebar-opacity");
    expect(payload).toContain("--cbg-surface-opacity");
    expect(payload).toContain("--cbg-menu-opacity");
    expect(payload).toContain("--cbg-wco-safe-right");
    expect(payload).toContain("--cbg-card-opacity");
    expect(payload).toContain('aria-roledescription="sortable"');
    expect(payload).toContain('a[href*="/issues/"]');
    expect(payload).toContain("windowControlsOverlay");
    expect(payload).toContain("geometrychange");
    expect(payload).toContain("multica-background-wco");
    expect(payload).toContain("[role=");
    expect(payload).toContain("syncFullWindowMedia");
    expect(payload).not.toContain("outerHeight");
    expect(payload).not.toContain("chromeH");
    expect(payload).toContain("multica-background-home");
    expect(payload).toContain("multica-background-task");
    expect(payload).toContain("media.playbackRate");
    expect(payload).toContain("__MULTICA_BACKGROUND_STUDIO__");
    expect(payload).toContain("requestAnimationFrame");
    expect(payload).not.toContain("}, 200)");
    expect(payload).not.toContain("backdrop-filter: blur");
    expect(payload).not.toContain(".notion-");
    expect(payload).not.toContain("markNativeCovers");
    expect(payload).not.toContain("markBlockFills");
    expect(payload).not.toContain("__NOTION_BACKGROUND_STUDIO__");
  });

  it("serializes media URLs instead of interpolating executable source", () => {
    const payload = buildRendererPayload({
      mediaUrl: "http://127.0.0.1/media/\";window.pwned=true;//",
      mediaKind: "image",
      display: DEFAULT_SETTINGS.display,
      revision: "safe",
    });
    expect(payload).toContain(JSON.stringify("http://127.0.0.1/media/\";window.pwned=true;//"));
    expect(payload).not.toContain('src = "http://127.0.0.1/media/"');
  });

  it("keeps cleanup and early payload reversible", () => {
    expect(REMOVE_RENDERER_PAYLOAD).toContain("cleanup");
    expect(REMOVE_RENDERER_PAYLOAD).toContain("__MULTICA_BACKGROUND_STUDIO__");
    const early = earlyPayloadFor("window.test = true", "revision-1");
    expect(early).toContain("revision-1");
    expect(early).toContain("MutationObserver");
  });
});
