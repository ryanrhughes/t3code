import * as Duration from "effect/Duration";
import * as Effect from "effect/Effect";
import * as Option from "effect/Option";
import * as Schedule from "effect/Schedule";
import * as Scope from "effect/Scope";

import type * as Electron from "electron";

import * as ElectronApp from "../electron/ElectronApp.ts";
import * as ElectronWindow from "../electron/ElectronWindow.ts";
import {
  enterGamingOverlayAtLaunch,
  hasGamingOverlayLaunchFlag,
} from "../ipc/methods/gamingOverlay.ts";
import { makeComponentLogger } from "./DesktopObservability.ts";

const { logInfo, logWarning } = makeComponentLogger("desktop-gaming-launch");

type LaunchWindow = Pick<Electron.BrowserWindow, "isVisible" | "isDestroyed" | "once">;

/**
 * `t3code --gaming-overlay` is the gaming launcher. On a cold start it enters
 * gaming mode once the main window is on screen; while the app is already
 * running, Electron forwards the second instance's argv here and the running
 * app enters (or re-shows) the overlay instead of opening a second window.
 * The overlay reuses the renderer, so every environment, thread, draft, and
 * unread marker the desktop app already has is what the badge reflects.
 */
export const registerGamingOverlayLaunch = Effect.fn("desktop.gamingLaunch.register")(
  function* (input: {
    readonly argv: ReadonlyArray<string>;
    readonly waitForVisibleWindow?: Effect.Effect<Option.Option<LaunchWindow>>;
  }) {
    const electronApp = yield* ElectronApp.ElectronApp;
    const electronWindow = yield* ElectronWindow.ElectronWindow;
    const context = yield* Effect.context<
      Effect.Services<typeof enterGamingOverlayAtLaunch> | Scope.Scope
    >();
    const runPromise = Effect.runPromiseWith(context);

    // The compositor identifies the window by pid and title, so it must be
    // mapped before entering. Poll the window's visibility rather than the
    // compositor: Hyprland maps it within a frame of Electron showing it.
    const waitForVisibleWindow =
      input.waitForVisibleWindow ??
      Effect.gen(function* () {
        const window = yield* electronWindow.main;
        if (Option.isNone(window) || window.value.isDestroyed()) return Option.none();
        if (window.value.isVisible()) return Option.some<LaunchWindow>(window.value);
        return Option.none<LaunchWindow>();
      }).pipe(
        Effect.repeat({
          until: Option.isSome,
          schedule: Schedule.spaced(Duration.millis(250)),
        }),
        Effect.timeoutOption(Duration.seconds(60)),
        Effect.map(Option.flatten),
      );

    const enter = Effect.gen(function* () {
      const window = yield* waitForVisibleWindow;
      if (Option.isNone(window)) {
        yield* logWarning("gaming overlay launch skipped: no visible main window");
        return;
      }
      // A freshly shown window can still be a frame away from the compositor's
      // client list; retry identification briefly instead of failing the launch.
      const entered = yield* enterGamingOverlayAtLaunch.pipe(
        Effect.retry({
          times: 15,
          while: (error) => error.message.includes("Could not identify"),
          schedule: Schedule.spaced(Duration.millis(200)),
        }),
        Effect.catchCause((cause) =>
          logWarning("gaming overlay launch failed", { cause }).pipe(Effect.as(false)),
        ),
      );
      if (entered) yield* logInfo("gaming overlay entered from launcher");
    });

    if (hasGamingOverlayLaunchFlag(input.argv)) {
      yield* Effect.forkScoped(enter);
    }
    yield* electronApp.on("second-instance", (_event: unknown, argv: unknown) => {
      if (Array.isArray(argv) && hasGamingOverlayLaunchFlag(argv as string[])) {
        void runPromise(enter);
      }
    });
  },
);
