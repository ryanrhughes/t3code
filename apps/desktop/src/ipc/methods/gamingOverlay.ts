import { HostProcessEnvironment, HostProcessPlatform } from "@t3tools/shared/hostProcess";
import * as Electron from "electron";
import * as Effect from "effect/Effect";
import * as Option from "effect/Option";
import * as Schema from "effect/Schema";
import { DesktopGamingOverlayStateSchema, DesktopGamingBadgeSchema } from "@t3tools/contracts";
import * as DesktopEnvironment from "../../app/DesktopEnvironment.ts";
import { startNativeGamingBadge } from "../../window/NativeGamingBadge.ts";
import { startElectronGamingBadge } from "../../window/ElectronGamingBadge.ts";
import * as ElectronWindow from "../../electron/ElectronWindow.ts";
import { GamingOverlay, gamingOverlays } from "../../window/GamingOverlay.ts";
import { startHyprlandBadgeClicks } from "../../window/HyprlandBadgeClicks.ts";
import { enterHyprlandGamingOverlay, runHyprctl } from "../../window/HyprlandGamingOverlay.ts";
import * as DesktopIpc from "../DesktopIpc.ts";
import {
  GAMING_OVERLAY_CHANNEL,
  GAMING_OVERLAY_STATE_CHANNEL,
  GAMING_BADGE_CHANNEL,
  GAMING_OVERLAY_ACTIVATE_CHANNEL,
} from "../channels.ts";

class GamingOverlayError extends Schema.TaggedError<GamingOverlayError>()("GamingOverlayError", {
  message: Schema.String,
}) {}

/** `t3code --gaming-overlay` enters gaming mode in the running app, or at launch. */
export const GAMING_OVERLAY_LAUNCH_FLAG = "--gaming-overlay";

export const hasGamingOverlayLaunchFlag = (argv: ReadonlyArray<string>) =>
  argv.includes(GAMING_OVERLAY_LAUNCH_FLAG);

const NATIVE_BADGE_NAMESPACE = "t3-gaming-badge";

function controller(
  window: Electron.BrowserWindow,
  platform: NodeJS.Platform,
  env: NodeJS.ProcessEnv,
  helperPath: string,
  onCleanupError: (error: unknown) => void,
) {
  const existing = gamingOverlays.get(window);
  if (existing) return existing;
  const overlay = new GamingOverlay(
    window,
    async () => {
      const activate = () => {
        // Open the roster before focusing T3, so the previously selected chat is not marked read.
        if (!window.isDestroyed()) window.webContents.send(GAMING_OVERLAY_ACTIVATE_CHANNEL);
        void overlay
          .action("show")
          .catch((error: unknown) =>
            overlay.badgeFailed(
              error instanceof Error ? error.message : "Could not open gaming chat.",
            ),
          );
      };
      if (platform === "linux" && env.HYPRLAND_INSTANCE_SIGNATURE) {
        // Start the badge first so setup failures cannot strand a window in a scratchpad.
        const badge = await startNativeGamingBadge(
          helperPath,
          activate,
          (message) => void overlay.action("get").then(() => overlay.badgeFailed(message)),
        );
        try {
          const host = await enterHyprlandGamingOverlay({
            pid: process.pid,
            windowId: window.id,
            title: window.getTitle(),
          });
          let clicks: Awaited<ReturnType<typeof startHyprlandBadgeClicks>> | undefined;
          try {
            clicks = await startHyprlandBadgeClicks({
              namespace: NATIVE_BADGE_NAMESPACE,
              handle: `t3_gaming_clicks_${process.pid}_${window.id}`,
              env,
              run: runHyprctl,
              activate,
            });
          } catch (error) {
            await host.restore();
            throw error;
          }
          return {
            ...host,
            updateBadge: badge.update,
            restore: async () => {
              try {
                await clicks.close();
              } finally {
                try {
                  await host.restore();
                } finally {
                  await badge.close();
                }
              }
            },
          };
        } catch (error) {
          await badge.close();
          throw error;
        }
      }
      if (platform === "linux" && env.XDG_SESSION_TYPE === "wayland") {
        throw new Error("Gaming mode currently supports Hyprland or an X11 session on Linux.");
      }
      const shortcut = "CommandOrControl+Shift+Space";
      if (
        !Electron.globalShortcut.register(shortcut, () => {
          if (window.isDestroyed()) return;
          if (window.isVisible() && window.isFocused()) window.hide();
          else {
            window.show();
            window.focus();
          }
        })
      )
        throw new Error("Ctrl+Shift+Space is already in use. Free that shortcut and try again.");
      try {
        window.unmaximize();
        window.setBounds({ width: 520, height: 720 });
        window.setAlwaysOnTop(true, "floating");
      } catch (error) {
        Electron.globalShortcut.unregister(shortcut);
        throw error;
      }
      let badge;
      try {
        badge = await startElectronGamingBadge(activate, platform);
      } catch (error) {
        Electron.globalShortcut.unregister(shortcut);
        throw error;
      }
      return {
        updateBadge: badge.update,
        show: async () => {
          window.show();
          window.focus();
        },
        shortcutLabel: platform === "darwin" ? "⌘⇧Space" : "Ctrl+Shift+Space",
        hide: async () => {
          window.hide();
        },
        restore: async () => {
          Electron.globalShortcut.unregister(shortcut);
          await badge.close();
        },
      };
    },
    (state) => {
      if (!window.isDestroyed()) window.webContents.send(GAMING_OVERLAY_STATE_CHANNEL, state);
    },
  );
  gamingOverlays.set(window, overlay);
  window.webContents.on("render-process-gone", () => {
    // A disconnected renderer can no longer keep the badge's live status honest.
    void overlay.action("exit").catch(onCleanupError);
  });
  window.on("close", (event) => {
    void overlay.close(event).catch(onCleanupError);
  });
  window.once("closed", () => {
    void overlay.action("exit").catch(onCleanupError);
    gamingOverlays.delete(window);
  });
  return overlay;
}

const resolveController = Effect.gen(function* () {
  const windows = yield* ElectronWindow.ElectronWindow;
  const platform = yield* HostProcessPlatform;
  const env = yield* HostProcessEnvironment;
  const environment = yield* DesktopEnvironment.DesktopEnvironment;
  const helperPath = environment.path.join(
    environment.isPackaged ? environment.resourcesPath : environment.appRoot,
    environment.isPackaged ? "hyprland-capture" : "native/hyprland-snap-shot/target/release",
    "t3-hyprland-snap-shot",
  );
  const runFork = Effect.runForkWith(yield* Effect.context<never>());
  const window = yield* windows.main;
  if (Option.isNone(window)) return Option.none();
  return Option.some(
    controller(window.value, platform, env, helperPath, (error) => {
      runFork(Effect.logError("Gaming overlay cleanup failed", error));
    }),
  );
});

/**
 * Enters gaming mode from the launcher without going through the renderer: the
 * roster opens as soon as the badge is up, and the chat starts hidden so the
 * game keeps focus. A mapped window is required; Hyprland identifies it by pid.
 */
export const enterGamingOverlayAtLaunch = Effect.gen(function* () {
  const overlay = yield* resolveController;
  if (Option.isNone(overlay)) return false;
  const control = overlay.value;
  const enter = async () => {
    const state = await control.action("enter");
    if (state.enabled && !control.previouslyEnabled) await control.action("hide");
  };
  yield* Effect.tryPromise({
    try: enter,
    catch: (error) =>
      new GamingOverlayError({
        message: error instanceof Error ? error.message : "Could not enter gaming mode.",
      }),
  });
  return true;
});

export const gamingOverlay = DesktopIpc.makeIpcMethod({
  channel: GAMING_OVERLAY_CHANNEL,
  payload: Schema.Literals(["get", "enter", "exit", "hide", "show"]),
  result: DesktopGamingOverlayStateSchema,
  handler: Effect.fn("desktop.ipc.gamingOverlay")(function* (action) {
    const overlay = yield* resolveController;
    if (Option.isNone(overlay))
      return yield* new GamingOverlayError({ message: "T3's main window is not available." });
    return yield* Effect.tryPromise({
      try: () => overlay.value.action(action),
      catch: (error) =>
        new GamingOverlayError({
          message: error instanceof Error ? error.message : "Could not change gaming mode.",
        }),
    });
  }),
});

export const setGamingBadge = DesktopIpc.makeIpcMethod({
  channel: GAMING_BADGE_CHANNEL,
  payload: DesktopGamingBadgeSchema,
  result: Schema.Void,
  handler: Effect.fn("desktop.ipc.setGamingBadge")(function* (status) {
    const windows = yield* ElectronWindow.ElectronWindow;
    const window = yield* windows.main;
    if (Option.isSome(window)) gamingOverlays.get(window.value)?.updateBadge(status);
  }),
});
