import type { DesktopGamingOverlayState } from "@t3tools/contracts";
import { createContext } from "react";
import { create } from "zustand";
import { toastManager } from "./components/ui/toast";

// Lets a development browser inspect the actual overlay UI without native window controls.
export const gamingBrowserPreview =
  import.meta.env.DEV &&
  typeof window !== "undefined" &&
  !window.desktopBridge?.gamingOverlay &&
  new URLSearchParams(window.location.search).has("gaming-preview");

export const GamingConversationVisible = createContext(true);
export const useGamingOverlay = create<DesktopGamingOverlayState>(() => ({
  enabled: gamingBrowserPreview,
  shortcutLabel: gamingBrowserPreview ? "Browser preview" : null,
}));

export async function gamingOverlayAction(action: "enter" | "exit" | "hide") {
  const bridge = window.desktopBridge?.gamingOverlay;
  if (!bridge) {
    if (gamingBrowserPreview && action === "exit") useGamingOverlay.setState({ enabled: false });
    return gamingBrowserPreview;
  }
  try {
    useGamingOverlay.setState(await bridge(action), true);
    return true;
  } catch (error) {
    toastManager.add({
      type: "error",
      title: "Gaming mode",
      description: error instanceof Error ? error.message : "Could not change gaming mode.",
    });
    return false;
  }
}

export async function toggleGamingOverlay() {
  await gamingOverlayAction(useGamingOverlay.getState().enabled ? "exit" : "enter");
}

/** Subscribe before reading, so a native mode change cannot be lost during mount. */
export function subscribeGamingOverlay() {
  const bridge = window.desktopBridge;
  if (!bridge?.gamingOverlay || !bridge.onGamingOverlayState) return;
  let active = true;
  let receivedEvent = false;
  const unsubscribe = bridge.onGamingOverlayState((state) => {
    receivedEvent = true;
    useGamingOverlay.setState(state, true);
  });
  void bridge
    .gamingOverlay("get")
    .then((state) => {
      if (active && !receivedEvent) useGamingOverlay.setState(state, true);
    })
    .catch(() => undefined);
  return () => {
    active = false;
    unsubscribe();
  };
}
