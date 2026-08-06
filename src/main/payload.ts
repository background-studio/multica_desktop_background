import { createHash } from "node:crypto";
import { DisplaySettings, MediaKind } from "../shared/contracts.js";

const BACKGROUND_CSS = String.raw`
html.multica-background-active,
html.multica-background-active body,
html.multica-background-active #root,
html.multica-background-active .bg-app-shell,
html.multica-background-active [data-slot="sidebar-wrapper"],
html.multica-background-active .flex.h-screen.bg-app-shell {
  background: transparent !important;
  background-color: transparent !important;
}

/* 主壳压在媒体层之上，避免内容掉到背景后面。 */
html.multica-background-active body > #root {
  position: relative;
  z-index: 1;
  height: 100%;
  max-height: 100%;
}

#multica-background-layer {
  position: fixed;
  inset: 0;
  z-index: 0;
  overflow: hidden;
  pointer-events: none;
  opacity: calc(var(--cbg-opacity) * var(--cbg-route-intensity));
  background-color: transparent;
  transition: opacity 220ms ease;
}

#multica-background-media,
#multica-background-tile {
  position: absolute;
  left: 0;
  top: 0;
  width: 100%;
  height: 100%;
  transform: scale(var(--cbg-scale));
  filter: blur(var(--cbg-blur));
  transform-origin: center center;
}

#multica-background-media {
  display: block;
  object-fit: var(--cbg-fit);
  object-position: var(--cbg-position-x) var(--cbg-position-y);
}

#multica-background-tile {
  display: none;
  background-image: var(--cbg-media-url);
  background-repeat: repeat;
  background-position: var(--cbg-position-x) var(--cbg-position-y);
  background-size: auto;
}

html.multica-background-fit-tile #multica-background-media { display: none; }
html.multica-background-fit-tile #multica-background-tile { display: block; }

#multica-background-overlay {
  position: absolute;
  inset: 0;
  background: var(--cbg-overlay-color);
  opacity: var(--cbg-overlay-opacity);
}

html.multica-background-home { --cbg-route-intensity: var(--cbg-home-intensity); }
html.multica-background-task { --cbg-route-intensity: var(--cbg-task-intensity); }
html.multica-background-home.multica-background-home-disabled,
html.multica-background-task.multica-background-task-disabled { --cbg-route-intensity: 0; }

/*
 * 侧栏：data-sidebar / sidebar-inner。
 * slider 只加很轻的雾（* 28%），避免高值变成实底罩。
 */
html.multica-background-active [data-sidebar="sidebar"],
html.multica-background-active [data-slot="sidebar-inner"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-sidebar-opacity) * 28%), transparent) !important;
  backdrop-filter: none !important;
  box-shadow: none !important;
}

/* 主画布 / 顶栏 / 卡片壳：外层打底，内部文字层保持可读。 */
html.multica-background-active .bg-page-canvas,
html.multica-background-active header,
html.multica-background-active [data-slot="card"],
html.multica-background-active [data-slot="chat-input-surface"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-surface-opacity) * 28%), transparent) !important;
  backdrop-filter: none !important;
  box-shadow: none !important;
}

/* Electron WCO 把原生窗口按钮叠在同一客户区；顶栏为其预留右侧安全区。 */
html.multica-background-active.multica-background-wco header.relative.shrink-0.h-12 {
  box-sizing: border-box !important;
  padding-right: var(--cbg-wco-safe-right, 0px) !important;
}

/* 看板任务卡片：只调整卡片底色，文字、状态和拖拽命中保持完整不透明。 */
html.multica-background-active
  [role="button"][aria-roledescription="sortable"]
  > a[href*="/issues/"]
  > [class~="bg-surface"] {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-card-opacity) * 100%), transparent) !important;
  box-shadow: none !important;
}

html.multica-background-active [role="dialog"],
html.multica-background-active [role="menu"],
html.multica-background-active [role="listbox"],
html.multica-background-active .bg-surface-raised {
  background: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-menu-opacity) * 100%), transparent) !important;
  background-color: color-mix(in srgb, var(--cbg-surface-color, #191919) calc(var(--cbg-menu-opacity) * 100%), transparent) !important;
  backdrop-filter: none !important;
}

html.multica-background-dark #multica-background-layer {
  background-color: transparent;
}

@media (prefers-reduced-motion: reduce) {
  #multica-background-layer { transition: none; }
}
`;

const REVIEW_SHADOW_STYLE_ID = "multica-background-review-shadow-style";
const REVIEW_SHADOW_CSS = String.raw`
/* Multica MVP 暂不注入 Shadow DOM 审阅样式；保留占位以兼容修订哈希与清理逻辑。 */
:host { background-color: transparent !important; }
`;

export interface PayloadInput {
  mediaUrl: string;
  mediaKind: MediaKind;
  display: DisplaySettings;
  revision: string;
}

export function buildRendererPayload(input: PayloadInput) {
  const revision = createHash("sha256")
    .update(input.revision)
    .update(BACKGROUND_CSS)
    .update(REVIEW_SHADOW_CSS)
    .digest("hex");
  const serialized = JSON.stringify({ ...input, revision }).replace(/</g, "\\u003c");
  const css = JSON.stringify(BACKGROUND_CSS);
  const reviewShadowCss = JSON.stringify(REVIEW_SHADOW_CSS);
  const reviewShadowStyleId = JSON.stringify(REVIEW_SHADOW_STYLE_ID);
  return String.raw`((config, cssText, reviewShadowCssText, reviewShadowStyleId) => {
    const STATE = "__MULTICA_BACKGROUND_STUDIO__";
    const STYLE_ID = "multica-background-style";
    const LAYER_ID = "multica-background-layer";
    const REVIEW_HOST_SELECTOR = "diffs-container";
    const ROOT_CLASSES = [
      "multica-background-active", "multica-background-home", "multica-background-task",
      "multica-background-home-disabled", "multica-background-task-disabled",
      "multica-background-fit-tile", "multica-background-dark", "multica-background-wco"
    ];
    const ROOT_PROPERTIES = [
      "--cbg-opacity", "--cbg-blur", "--cbg-scale", "--cbg-fit",
      "--cbg-position-x", "--cbg-position-y", "--cbg-overlay-color",
      "--cbg-overlay-opacity", "--cbg-home-intensity", "--cbg-task-intensity",
      "--cbg-route-intensity", "--cbg-sidebar-opacity", "--cbg-surface-opacity",
      "--cbg-composer-opacity", "--cbg-menu-opacity", "--cbg-terminal-opacity",
      "--cbg-block-fill-opacity", "--cbg-media-url", "--cbg-surface-color",
      "--cbg-wco-safe-right", "--cbg-card-opacity"
    ];

    const previous = window[STATE];
    if (previous?.cleanup) {
      previous.cleanup();
    } else {
      if (previous?.observer) previous.observer.disconnect();
      if (previous?.timer) clearInterval(previous.timer);
      previous?.wco?.removeEventListener?.("geometrychange", previous?.wcoGeometry);
      previous?.layer?.remove();
      if (previous?.blobUrl) URL.revokeObjectURL(previous.blobUrl);
    }
    let scheduled = null;
    let shadowPatch = null;

    const blobUrl = (() => {
      const comma = config.mediaUrl.indexOf(",");
      if (!config.mediaUrl.startsWith("data:") || comma < 0) return config.mediaUrl;
      const binary = atob(config.mediaUrl.slice(comma + 1));
      const bytes = new Uint8Array(binary.length);
      for (let index = 0; index < binary.length; index += 1) bytes[index] = binary.charCodeAt(index);
      const mime = /^data:([^;,]+)/.exec(config.mediaUrl)?.[1] || "application/octet-stream";
      return URL.createObjectURL(new Blob([bytes], { type: mime }));
    })();

    const installReviewShadowStyle = (host, shadow = host?.shadowRoot) => {
      if (!shadow) return false;
      let shadowStyle = shadow.getElementById(reviewShadowStyleId);
      if (!shadowStyle) {
        shadowStyle = document.createElement("style");
        shadowStyle.id = reviewShadowStyleId;
      }
      if (shadowStyle.dataset.cbgRevision !== config.revision) {
        shadowStyle.textContent = reviewShadowCssText;
        shadowStyle.dataset.cbgRevision = config.revision;
      }
      shadow.appendChild(shadowStyle);
      return true;
    };

    const syncFullWindowMedia = (layer, media, tile) => {
      const viewH = Math.max(Number(window.innerHeight) || 0, 1);
      layer.style.position = "fixed";
      layer.style.inset = "0";
      layer.style.overflow = "hidden";
      layer.style.zIndex = "0";
      for (const node of [media, tile]) {
        if (!node) continue;
        node.style.position = "absolute";
        node.style.left = "0";
        node.style.top = "0";
        node.style.width = "100%";
        node.style.height = viewH + "px";
      }
    };

    const cleanup = () => {
      const state = window[STATE];
      state?.observer?.disconnect();
      if (state?.timer) clearInterval(state.timer);
      state?.wco?.removeEventListener?.("geometrychange", state?.wcoGeometry);
      if (scheduled) cancelAnimationFrame(scheduled);
      if (shadowPatch?.prototype.attachShadow === shadowPatch.wrapped) {
        shadowPatch.prototype.attachShadow = shadowPatch.original;
      }
      document.getElementById(LAYER_ID)?.remove();
      document.getElementById(STYLE_ID)?.remove();
      document.querySelectorAll("diffs-container").forEach((host) => {
        host.shadowRoot?.getElementById(reviewShadowStyleId)?.remove();
      });
      document.documentElement?.classList.remove(...ROOT_CLASSES);
      for (const property of ROOT_PROPERTIES) document.documentElement?.style.removeProperty(property);
      if (state?.blobUrl) URL.revokeObjectURL(state.blobUrl);
      delete window[STATE];
      return true;
    };

    const patchAttachShadow = () => {
      const prototype = Element.prototype;
      const original = prototype.attachShadow;
      const wrapped = function(init) {
        const shadow = original.call(this, init);
        if (this.localName === REVIEW_HOST_SELECTOR) {
          queueMicrotask(() => installReviewShadowStyle(this, shadow));
          requestAnimationFrame(() => installReviewShadowStyle(this, shadow));
        }
        return shadow;
      };
      prototype.attachShadow = wrapped;
      return { prototype, original, wrapped };
    };
    shadowPatch = patchAttachShadow();

    const detectAppearance = () => {
      const root = document.documentElement;
      const classText = ((root?.className || "") + " " + (document.body?.className || ""))
        .toLowerCase()
        .replace(/\bmultica-background-[a-z-]+\b/g, "");
      if (/\b(?:dark|theme-dark)\b/.test(classText)) return "dark";
      if (/\b(?:light|theme-light)\b/.test(classText)) return "light";
      const dataTheme = (
        root?.getAttribute("data-theme") || root?.getAttribute("data-appearance") ||
        document.body?.getAttribute("data-theme") || ""
      ).toLowerCase();
      if (dataTheme.includes("dark")) return "dark";
      if (dataTheme.includes("light")) return "light";
      try {
        return window.matchMedia("(prefers-color-scheme: dark)").matches ? "dark" : "light";
      } catch {}
      return "light";
    };

    const install = () => {
      const root = document.documentElement;
      if (!root) return false;

      const setClass = (name, on) => {
        if (root.classList.contains(name) !== on) root.classList.toggle(name, on);
      };
      const setProp = (name, value) => {
        if (root.style.getPropertyValue(name) !== value) root.style.setProperty(name, value);
      };

      const dark = detectAppearance() === "dark";
      setClass("multica-background-dark", dark);
      setProp("--cbg-surface-color", dark ? "#191919" : "#ffffff");
      const wco = navigator.windowControlsOverlay;
      const titlebarRect = wco?.visible ? wco.getTitlebarAreaRect?.() : null;
      const wcoVisible = Boolean(titlebarRect && titlebarRect.width > 0);
      const safeRight = wcoVisible
        ? Math.max(0, window.innerWidth - (titlebarRect.x + titlebarRect.width))
        : 0;
      setClass("multica-background-wco", wcoVisible);
      setProp("--cbg-wco-safe-right", safeRight + "px");

      let style = document.getElementById(STYLE_ID);
      if (!style) {
        style = document.createElement("style");
        style.id = STYLE_ID;
        (document.head || root).appendChild(style);
      }
      if (style.dataset.cbgRevision !== config.revision) {
        style.textContent = cssText;
        style.dataset.cbgRevision = config.revision;
      }

      let layer = document.getElementById(LAYER_ID);
      if (!layer && document.body) {
        layer = document.createElement("div");
        layer.id = LAYER_ID;
        const media = document.createElement(config.mediaKind === "video" ? "video" : "img");
        media.id = "multica-background-media";
        media.setAttribute("aria-hidden", "true");
        if (config.mediaKind === "video") {
          media.autoplay = true;
          media.loop = true;
          media.muted = Boolean(config.display.videoMuted);
          media.defaultMuted = Boolean(config.display.videoMuted);
          media.playsInline = true;
          media.playbackRate = Number(config.display.videoPlaybackRate) || 1;
        }
        media.src = blobUrl;
        media.addEventListener("error", () => cleanup());
        const tile = document.createElement("div");
        tile.id = "multica-background-tile";
        const overlay = document.createElement("div");
        overlay.id = "multica-background-overlay";
        layer.append(media, tile, overlay);
        document.body.prepend(layer);
        if (config.mediaKind === "video") media.play().catch(() => undefined);
      }
      if (layer) {
        syncFullWindowMedia(
          layer,
          document.getElementById("multica-background-media"),
          document.getElementById("multica-background-tile"),
        );
      }

      setClass("multica-background-active", true);
      setClass("multica-background-fit-tile", config.display.fit === "tile" && config.mediaKind === "image");
      setClass("multica-background-home-disabled", !config.display.enabledOnHome);
      setClass("multica-background-task-disabled", !config.display.enabledOnTasks);
      setProp("--cbg-opacity", String(config.display.opacity));
      setProp("--cbg-blur", config.display.blur + "px");
      setProp("--cbg-scale", String(config.display.scale));
      setProp("--cbg-fit", config.display.fit === "tile" ? "cover" : config.display.fit);
      setProp("--cbg-position-x", config.display.positionX + "%");
      setProp("--cbg-position-y", config.display.positionY + "%");
      setProp("--cbg-overlay-color", config.display.overlayColor);
      setProp("--cbg-overlay-opacity", String(config.display.overlayOpacity));
      setProp("--cbg-block-fill-opacity", String(config.display.blockFillOpacity));
      setProp("--cbg-home-intensity", String(config.display.homeIntensity));
      setProp("--cbg-task-intensity", String(config.display.taskIntensity));
      setProp("--cbg-sidebar-opacity", String(config.display.sidebarOpacity));
      setProp("--cbg-surface-opacity", String(config.display.surfaceOpacity));
      setProp("--cbg-card-opacity", String(config.display.cardOpacity));
      setProp("--cbg-composer-opacity", String(config.display.composerOpacity));
      setProp("--cbg-menu-opacity", String(config.display.menuOpacity));
      setProp("--cbg-terminal-opacity", String(config.display.terminalOpacity));
      setProp("--cbg-media-url", 'url("' + String(blobUrl).replace(/["\\\n\r]/g, "") + '")');

      // Multica 页面统一按“页面”强度走 home 通道；空白恢复页走 task 通道便于单独关掉。
      const blank = /\/blank(?:\?|$)/.test(location.pathname + location.search);
      setClass("multica-background-home", !blank);
      setClass("multica-background-task", blank);
      return true;
    };

    const scheduleInstall = () => {
      if (scheduled) return;
      scheduled = requestAnimationFrame(() => { scheduled = null; install(); });
    };
    const observer = new MutationObserver(scheduleInstall);
    observer.observe(document.documentElement, {
      childList: true,
      subtree: true,
      attributes: true,
      attributeFilter: ["class", "data-theme", "data-appearance"],
    });
    const timer = setInterval(install, 4000);
    const wco = navigator.windowControlsOverlay;
    const wcoGeometry = () => scheduleInstall();
    wco?.addEventListener?.("geometrychange", wcoGeometry);
    window[STATE] = {
      revision: config.revision, cleanup, observer, timer, layer: null, blobUrl,
      wco, wcoGeometry,
    };
    install();
    window[STATE].layer = document.getElementById(LAYER_ID);
    return { installed: true, revision: config.revision, mediaKind: config.mediaKind };
  })(${serialized}, ${css}, ${reviewShadowCss}, ${reviewShadowStyleId})`;
}

export const REMOVE_RENDERER_PAYLOAD = String.raw`(() => {
  const state = window.__MULTICA_BACKGROUND_STUDIO__;
  if (state?.cleanup) return state.cleanup();
  document.getElementById("multica-background-layer")?.remove();
  document.getElementById("multica-background-style")?.remove();
  document.documentElement?.classList.remove(
    "multica-background-active", "multica-background-home", "multica-background-task",
    "multica-background-home-disabled", "multica-background-task-disabled",
    "multica-background-fit-tile", "multica-background-wco"
  );
  document.documentElement?.style.removeProperty("--cbg-wco-safe-right");
  document.documentElement?.style.removeProperty("--cbg-card-opacity");
  delete window.__MULTICA_BACKGROUND_STUDIO__;
  return true;
})()`;

export function earlyPayloadFor(payload: string, revision: string) {
  const safeRevision = JSON.stringify(revision);
  return String.raw`(() => {
    const revision = ${safeRevision};
    const run = () => {
      if (!document.documentElement) return false;
      try { ${payload}; return true; } catch { return false; }
    };
    if (!run()) {
      const observer = new MutationObserver(() => {
        if (run()) observer.disconnect();
      });
      observer.observe(document.documentElement || document, { childList: true, subtree: true });
      setTimeout(() => observer.disconnect(), 30000);
    }
    return revision;
  })()`;
}
