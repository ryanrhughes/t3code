import type { DesktopGamingOverlayState, DesktopGamingBadge } from "@t3tools/contracts";
import type * as Electron from "electron";

export type GamingWindow = Pick<
  Electron.BrowserWindow,
  | "getNormalBounds"
  | "getBounds"
  | "getMinimumSize"
  | "isMaximized"
  | "isFullScreen"
  | "isAlwaysOnTop"
  | "setMinimumSize"
  | "setBounds"
  | "setAlwaysOnTop"
  | "setFullScreen"
  | "maximize"
  | "unmaximize"
  | "show"
  | "hide"
  | "focus"
  | "isVisible"
  | "isDestroyed"
>;

interface OverlayHost {
  shortcutLabel: string;
  hide: () => Promise<void>;
  show: () => Promise<void>;
  updateBadge?: (status: DesktopGamingBadge) => void;
  restore: () => Promise<void>;
}

/** A reversible mode of the existing window: no second renderer or connection pool. */
export class GamingOverlay {
  state: DesktopGamingOverlayState = { enabled: false, shortcutLabel: null };
  private saved:
    | {
        bounds: Electron.Rectangle;
        minimum: number[];
        maximized: boolean;
        fullscreen: boolean;
        onTop: boolean;
      }
    | undefined;
  private host: OverlayHost | undefined;
  private queue: Promise<unknown> = Promise.resolve();

  private readonly window: GamingWindow;
  private readonly connectHost: () => Promise<OverlayHost>;
  private readonly publish: (state: DesktopGamingOverlayState) => void;

  constructor(
    window: GamingWindow,
    connectHost: () => Promise<OverlayHost>,
    publish: (state: DesktopGamingOverlayState) => void,
  ) {
    this.window = window;
    this.connectHost = connectHost;
    this.publish = publish;
  }

  get ownsWindowBounds() {
    return this.saved !== undefined;
  }

  /** True when the last `enter` found the mode already active. */
  previouslyEnabled = false;

  async close(event: Pick<Electron.Event, "preventDefault">) {
    if (!this.ownsWindowBounds) return;
    event.preventDefault();
    // Closing the compact chat returns to the game; the badge and draft stay alive.
    await this.action("hide");
  }

  updateBadge(status: DesktopGamingBadge) {
    this.host?.updateBadge?.(status);
  }

  badgeFailed(message: string) {
    if (!this.state.enabled) return;
    this.state = { ...this.state, badgeError: message };
    this.publish(this.state);
  }

  action(action: "get" | "enter" | "exit" | "hide" | "show"): Promise<DesktopGamingOverlayState> {
    const next = this.queue.then(async () => {
      if (action === "enter") this.previouslyEnabled = this.saved !== undefined;
      if (action === "enter" && !this.saved) await this.enter();
      if (action === "exit" && this.saved) await this.exit();
      if (action === "show" && this.host) await this.host.show();
      if (action === "hide" && this.host) await this.host.hide();
      return this.state;
    });
    this.queue = next.catch(() => undefined);
    return next;
  }

  private async enter() {
    const window = this.window;
    this.saved = {
      bounds: window.isMaximized() ? window.getNormalBounds() : window.getBounds(),
      minimum: window.getMinimumSize(),
      maximized: window.isMaximized(),
      fullscreen: window.isFullScreen(),
      onTop: window.isAlwaysOnTop(),
    };
    try {
      window.setMinimumSize(380, 460);
      // Fullscreen transitions are asynchronous on several compositors. Require an
      // ordinary window instead of racing the compositor's fullscreen restoration.
      if (this.saved.fullscreen)
        throw new Error("Leave T3's fullscreen mode before entering gaming mode.");
      if (this.saved.maximized) window.unmaximize();
      this.host = await this.connectHost();
      this.state = { enabled: true, shortcutLabel: this.host.shortcutLabel };
      this.publish(this.state);
    } catch (error) {
      this.restoreWindow();
      throw error;
    }
  }

  private restoreWindow() {
    const saved = this.saved;
    if (!saved) return;
    if (!this.window.isDestroyed()) {
      this.window.setAlwaysOnTop(saved.onTop);
      this.window.setMinimumSize(saved.minimum[0] ?? 840, saved.minimum[1] ?? 620);
      this.window.setBounds(saved.bounds);
      if (saved.maximized) this.window.maximize();
      this.window.show();
      this.window.focus();
    }
    this.saved = undefined;
  }

  private async exit() {
    await this.host?.restore();
    this.host = undefined;
    this.restoreWindow();
    this.state = { enabled: false, shortcutLabel: null };
    this.publish(this.state);
  }
}

export const gamingOverlays = new WeakMap<Electron.BrowserWindow, GamingOverlay>();
