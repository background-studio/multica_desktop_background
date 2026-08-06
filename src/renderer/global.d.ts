import type { BackgroundBridge } from "../shared/contracts";

declare global {
  interface Window {
    backgroundStudio?: BackgroundBridge;
  }
}

export {};
