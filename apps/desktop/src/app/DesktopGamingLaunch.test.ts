import { assert, describe, it } from "@effect/vitest";
import * as Effect from "effect/Effect";
import * as Option from "effect/Option";
import { vi } from "vite-plus/test";

import * as ElectronApp from "../electron/ElectronApp.ts";
import * as ElectronWindow from "../electron/ElectronWindow.ts";

const { enterMock } = vi.hoisted(() => ({ enterMock: vi.fn() }));
vi.mock("../ipc/methods/gamingOverlay.ts", () => ({
  GAMING_OVERLAY_LAUNCH_FLAG: "--gaming-overlay",
  hasGamingOverlayLaunchFlag: (argv: ReadonlyArray<string>) => argv.includes("--gaming-overlay"),
  enterGamingOverlayAtLaunch: Effect.suspend(() => enterMock() as Effect.Effect<boolean, Error>),
}));

import { registerGamingOverlayLaunch } from "./DesktopGamingLaunch.ts";

function harness() {
  const listeners = new Map<string, (...args: unknown[]) => void>();
  const electronApp = {
    on: (eventName: string, listener: (...args: unknown[]) => void) =>
      Effect.sync(() => {
        listeners.set(eventName, listener);
      }),
  } as unknown as ElectronApp.ElectronApp["Service"];
  const electronWindow = {} as ElectronWindow.ElectronWindow["Service"];
  const visibleWindow = Effect.succeed(
    Option.some({ isVisible: () => true, isDestroyed: () => false, once: vi.fn() }),
  );
  return { listeners, electronApp, electronWindow, visibleWindow };
}

describe("gaming overlay launcher", () => {
  it.live("enters gaming mode at startup only when the launch flag is present", () => {
    enterMock.mockReset();
    enterMock.mockReturnValue(Effect.succeed(true));
    const { electronApp, electronWindow, visibleWindow } = harness();
    return Effect.gen(function* () {
      yield* Effect.scoped(
        registerGamingOverlayLaunch({ argv: ["electron", "."], waitForVisibleWindow: visibleWindow }),
      );
      assert.equal(enterMock.mock.calls.length, 0);
      yield* Effect.scoped(
        Effect.gen(function* () {
          yield* registerGamingOverlayLaunch({
            argv: ["electron", "--gaming-overlay"],
            waitForVisibleWindow: visibleWindow,
          });
          yield* Effect.yieldNow;
        }),
      );
      assert.equal(enterMock.mock.calls.length, 1);
    }).pipe(
      Effect.provideService(ElectronApp.ElectronApp, electronApp),
      Effect.provideService(ElectronWindow.ElectronWindow, electronWindow),
    );
  });

  it.live("re-enters from a second instance carrying the flag and ignores other argv", () => {
    enterMock.mockReset();
    enterMock.mockReturnValue(Effect.succeed(true));
    const { listeners, electronApp, electronWindow, visibleWindow } = harness();
    return Effect.gen(function* () {
      yield* Effect.scoped(
        Effect.gen(function* () {
          yield* registerGamingOverlayLaunch({ argv: [], waitForVisibleWindow: visibleWindow });
          const listener = listeners.get("second-instance");
          assert.isDefined(listener);
          listener!({}, ["t3code", "some-file.txt"], "/cwd");
          listener!({}, ["t3code", "--gaming-overlay"], "/cwd");
          yield* Effect.sleep("10 millis");
        }),
      );
      assert.equal(enterMock.mock.calls.length, 1);
    }).pipe(
      Effect.provideService(ElectronApp.ElectronApp, electronApp),
      Effect.provideService(ElectronWindow.ElectronWindow, electronWindow),
    );
  });

  it.live("retries while the compositor cannot identify the window yet", () => {
    enterMock.mockReset();
    enterMock
      .mockReturnValueOnce(Effect.fail(new Error("Could not identify T3's window in Hyprland.")))
      .mockReturnValueOnce(Effect.succeed(true));
    const { listeners, electronApp, electronWindow, visibleWindow } = harness();
    return Effect.gen(function* () {
      yield* Effect.scoped(
        Effect.gen(function* () {
          yield* registerGamingOverlayLaunch({ argv: [], waitForVisibleWindow: visibleWindow });
          listeners.get("second-instance")!({}, ["--gaming-overlay"], "/cwd");
          yield* Effect.sleep("400 millis");
        }),
      );
      assert.equal(enterMock.mock.calls.length, 2);
    }).pipe(
      Effect.provideService(ElectronApp.ElectronApp, electronApp),
      Effect.provideService(ElectronWindow.ElectronWindow, electronWindow),
    );
  });
});
