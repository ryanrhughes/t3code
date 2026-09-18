import { describe, expect, it, vi } from "vite-plus/test";
import { GamingOverlay, type GamingWindow } from "./GamingOverlay.ts";

function setup() {
  const original = { x: 80, y: 90, width: 1100, height: 780 };
  let bounds = { ...original };
  let minimum = [840, 620];
  let onTop = false;
  let visible = true;
  const window: GamingWindow = {
    getBounds: () => bounds,
    getNormalBounds: () => ({ ...original }),
    getMinimumSize: () => minimum,
    isMaximized: () => false,
    isFullScreen: () => false,
    isAlwaysOnTop: () => onTop,
    setMinimumSize: (w, h) => {
      minimum = [w, h];
    },
    setBounds: (next) => {
      bounds = { ...bounds, ...next };
    },
    setAlwaysOnTop: (next) => {
      onTop = next;
    },
    setFullScreen: vi.fn(),
    maximize: vi.fn(),
    unmaximize: vi.fn(),
    show: () => {
      visible = true;
    },
    hide: () => {
      visible = false;
    },
    focus: vi.fn(),
    isVisible: () => visible,
    isDestroyed: () => false,
  };
  const restore = vi.fn(async () => undefined);
  const connect = vi.fn(async () => {
    window.setBounds({ width: 520, height: 720 });
    window.setAlwaysOnTop(true);
    return {
      shortcutLabel: "Ctrl+Shift+Space",
      hide: async () => window.hide(),
      show: async () => window.show(),
      restore,
    };
  });
  const publish = vi.fn();
  const overlay = new GamingOverlay(window, connect, publish);
  return { overlay, window, connect, restore, publish, original };
}

describe("gaming overlay window lifecycle", () => {
  it("closes only the compact chat and keeps the badge available to reopen it", async () => {
    const { overlay, window, restore } = setup();
    const event = { preventDefault: vi.fn() };
    await overlay.action("enter");
    const closing = overlay.close(event);
    expect(event.preventDefault).toHaveBeenCalledOnce();
    await closing;
    expect(window.isVisible()).toBe(false);
    expect(overlay.state.enabled).toBe(true);
    expect(restore).not.toHaveBeenCalled();
    await overlay.action("show");
    expect(window.isVisible()).toBe(true);
    await overlay.action("exit");
    event.preventDefault.mockClear();
    await overlay.close(event);
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(restore).toHaveBeenCalledOnce();
  });

  it("hides without losing the mode and restores the original window on exit", async () => {
    const { overlay, window, restore, original } = setup();
    expect((await overlay.action("enter")).enabled).toBe(true);
    expect(overlay.ownsWindowBounds).toBe(true);
    expect(window.getBounds().width).toBe(520);
    await overlay.action("hide");
    expect(window.isVisible()).toBe(false);
    expect(overlay.state.enabled).toBe(true);
    await overlay.action("show");
    expect(window.isVisible()).toBe(true);
    await overlay.action("exit");
    expect(window.getBounds()).toEqual(original);
    expect(window.getMinimumSize()).toEqual([840, 620]);
    expect(window.isAlwaysOnTop()).toBe(false);
    expect(window.isVisible()).toBe(true);
    expect(overlay.ownsWindowBounds).toBe(false);
    expect(restore).toHaveBeenCalledOnce();
  });

  it("serializes rapid enter and exit without registering duplicate shortcuts", async () => {
    const { overlay, connect, restore } = setup();
    await Promise.all([
      overlay.action("enter"),
      overlay.action("enter"),
      overlay.action("exit"),
      overlay.action("exit"),
    ]);
    expect(connect).toHaveBeenCalledOnce();
    expect(restore).toHaveBeenCalledOnce();
    expect(overlay.state.enabled).toBe(false);
  });

  it("restores sizing when setup fails and permits another attempt", async () => {
    const { overlay, window, connect, original } = setup();
    connect.mockRejectedValueOnce(new Error("shortcut occupied"));
    await expect(overlay.action("enter")).rejects.toThrow("shortcut occupied");
    expect(window.getBounds()).toEqual(original);
    expect(window.getMinimumSize()).toEqual([840, 620]);
    expect(overlay.ownsWindowBounds).toBe(false);
    expect((await overlay.action("enter")).enabled).toBe(true);
  });

  it("keeps an unsuccessful exit retryable instead of lying about the mode", async () => {
    const { overlay, restore } = setup();
    await overlay.action("enter");
    restore.mockRejectedValueOnce(new Error("compositor unavailable"));
    await expect(overlay.action("exit")).rejects.toThrow("compositor unavailable");
    expect(overlay.state.enabled).toBe(true);
    await overlay.action("exit");
    expect(overlay.state.enabled).toBe(false);
  });

  it("does not race an asynchronous fullscreen transition", async () => {
    const { overlay, window, connect } = setup();
    window.isFullScreen = () => true;
    await expect(overlay.action("enter")).rejects.toThrow("Leave T3's fullscreen");
    expect(connect).not.toHaveBeenCalled();
    expect(window.setFullScreen).not.toHaveBeenCalled();
    expect(overlay.ownsWindowBounds).toBe(false);
  });
});

it("restores a maximized window's normal bounds as well as its maximized state", async () => {
  const { overlay, window, original } = setup();
  window.setBounds({ x: 0, y: 0, width: 1920, height: 1080 });
  window.isMaximized = () => true;
  await overlay.action("enter");
  expect(window.unmaximize).toHaveBeenCalledOnce();
  await overlay.action("exit");
  expect(window.getBounds()).toEqual(original);
  expect(window.maximize).toHaveBeenCalledOnce();
});
