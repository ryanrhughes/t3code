import { DEFAULT_CLIENT_SETTINGS, type DesktopTelemetrySetAgentActivity } from "@t3tools/contracts";
import * as Context from "effect/Context";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Option from "effect/Option";
import type * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";
import * as Electron from "electron";

import * as DesktopAssets from "../app/DesktopAssets.ts";
import * as DesktopEnvironment from "../app/DesktopEnvironment.ts";
import { makeComponentLogger } from "../app/DesktopObservability.ts";
import * as ElectronApp from "../electron/ElectronApp.ts";
import * as ElectronWindow from "../electron/ElectronWindow.ts";
import * as DesktopAppSettings from "../settings/DesktopAppSettings.ts";
import * as DesktopClientSettings from "../settings/DesktopClientSettings.ts";
import * as DesktopTelemetryPublisher from "../telemetry/DesktopTelemetryPublisher.ts";
import * as DesktopWindow from "./DesktopWindow.ts";
import {
  buildDesktopTrayModel,
  paintDesktopTrayStatusDot,
  resolveDesktopTrayNotifications,
  type DesktopTrayModel,
  type DesktopTrayStatus,
  type DesktopTrayThread,
} from "./desktopTrayModel.ts";

const { logInfo, logWarning } = makeComponentLogger("desktop-tray");

const MAX_SECTION_ITEMS = 8;
const STATUS_DOT_COLORS = {
  attention: { red: 245, green: 158, blue: 11 },
  working: { red: 59, green: 130, blue: 246 },
} as const;

/**
 * The system tray (menu bar on macOS) icon. While it is showing, closing the
 * last window leaves the app and its backends running; the icon lists what
 * the local agents are doing and notifies when one needs you or finishes.
 */
export class DesktopTray extends Context.Service<
  DesktopTray,
  {
    /** Shows or removes the icon to match the `trayEnabled` setting. */
    readonly sync: Effect.Effect<void>;
    /** Whether the icon is showing, so closing the last window should not quit. */
    readonly isActive: Effect.Effect<boolean>;
    /** Shows the icon if enabled and follows agent activity. Call once the app is ready. */
    readonly start: Effect.Effect<void, never, Scope.Scope>;
  }
>()("@t3tools/desktop/window/DesktopTray") {}

/** @public Service construction is part of the canonical Effect module API. */
export const make = Effect.gen(function* () {
  const appSettings = yield* DesktopAppSettings.DesktopAppSettings;
  const clientSettings = yield* DesktopClientSettings.DesktopClientSettings;
  const assets = yield* DesktopAssets.DesktopAssets;
  const environment = yield* DesktopEnvironment.DesktopEnvironment;
  const electronApp = yield* ElectronApp.ElectronApp;
  const electronWindow = yield* ElectronWindow.ElectronWindow;
  const desktopWindow = yield* DesktopWindow.DesktopWindow;
  const publisher = yield* DesktopTelemetryPublisher.DesktopTelemetryPublisher;
  const runFork = Effect.runForkWith(yield* Effect.context<never>());

  let tray: Electron.Tray | null = null;
  let renderedStatus: DesktopTrayStatus | null = null;
  let model = buildDesktopTrayModel([]);
  let sources: ReadonlyMap<string, DesktopTelemetrySetAgentActivity> = new Map();
  const icons = new Map<DesktopTrayStatus, Electron.NativeImage>();

  const run = <E>(effect: Effect.Effect<void, E>) =>
    void runFork(
      effect.pipe(
        Effect.catchCause((cause) => logWarning("tray action failed", { cause: String(cause) })),
      ),
    );

  const loadBaseIcon = Effect.gen(function* () {
    const iconPath = (yield* assets.iconPaths).png;
    if (Option.isNone(iconPath)) return null;
    const size = environment.platform === "darwin" ? 18 : 32;
    const image = Electron.nativeImage.createFromPath(iconPath.value);
    return image.isEmpty() ? null : image.resize({ width: size, height: size, quality: "best" });
  });

  const iconFor = (base: Electron.NativeImage, status: DesktopTrayStatus) => {
    const cached = icons.get(status);
    if (cached !== undefined) return cached;
    let icon = base;
    if (status !== "idle") {
      const { width } = base.getSize();
      const pixels = base.toBitmap();
      paintDesktopTrayStatusDot(pixels, width, STATUS_DOT_COLORS[status]);
      icon = Electron.nativeImage.createFromBitmap(pixels, { width, height: width });
    }
    icons.set(status, icon);
    return icon;
  };

  const threadItems = (
    title: string,
    threads: ReadonlyArray<DesktopTrayThread & { readonly prefix?: string }>,
  ): Array<Electron.MenuItemConstructorOptions> => {
    if (threads.length === 0) return [];
    const shown = threads.slice(0, MAX_SECTION_ITEMS);
    return [
      { type: "separator" },
      { label: title, enabled: false },
      ...shown.map((thread) => ({
        label: thread.prefix === undefined ? thread.label : `${thread.prefix} ${thread.label}`,
        click: () =>
          run(
            desktopWindow.openThread({
              environmentId: thread.environmentId,
              threadId: thread.threadId,
            }),
          ),
      })),
      ...(threads.length > shown.length
        ? [
            {
              label: `${threads.length - shown.length} more…`,
              click: () => run(desktopWindow.activate),
            },
          ]
        : []),
    ];
  };

  const buildMenu = (current: DesktopTrayModel) =>
    Electron.Menu.buildFromTemplate([
      { label: current.summary, enabled: false },
      ...threadItems(
        "Needs you",
        current.needsYou.map((thread) => ({
          ...thread,
          prefix: thread.phase === "approval" ? "Approve:" : "Reply:",
        })),
      ),
      ...threadItems("Working", current.working),
      ...threadItems(
        "Recently finished",
        current.finished.map((thread) => ({
          ...thread,
          prefix: thread.outcome === "failed" ? "✕" : "✓",
        })),
      ),
      { type: "separator" },
      { label: "Open T3 Code", click: () => run(desktopWindow.activate) },
      { label: "Quit T3 Code", click: () => run(electronApp.quit) },
    ]);

  const render = Effect.sync(() => {
    if (tray === null || tray.isDestroyed()) return;
    const base = icons.get("idle");
    if (base !== undefined && renderedStatus !== model.status) {
      tray.setImage(iconFor(base, model.status));
      renderedStatus = model.status;
    }
    tray.setToolTip(`T3 Code · ${model.summary}`);
    tray.setContextMenu(buildMenu(model));
  });

  const sync = Effect.gen(function* () {
    const enabled = (yield* appSettings.get).trayEnabled;
    if (!enabled) {
      if (tray !== null) {
        tray.destroy();
        tray = null;
        renderedStatus = null;
        yield* logInfo("tray removed");
      }
      return;
    }
    if (tray !== null) return;
    const base = icons.get("idle") ?? (yield* loadBaseIcon);
    if (base === null) {
      yield* logWarning("tray unavailable; no app icon found");
      return;
    }
    icons.set("idle", base);
    tray = new Electron.Tray(base);
    renderedStatus = "idle";
    // Windows and most Linux panels activate on a plain click; macOS opens the menu.
    if (environment.platform !== "darwin") tray.on("click", () => run(desktopWindow.activate));
    yield* render;
    yield* logInfo("tray shown");
  }).pipe(
    Effect.catchCause((cause) => logWarning("tray sync failed", { cause: String(cause) })),
    Effect.withSpan("desktop.tray.sync"),
  );

  // With no window open there is no renderer to notify, so the tray does it.
  const notify = Effect.fn("desktop.tray.notify")(function* (
    next: ReadonlyMap<string, DesktopTelemetrySetAgentActivity>,
  ) {
    const notifications = resolveDesktopTrayNotifications(sources, next);
    if (notifications.length === 0 || tray === null || !Electron.Notification.isSupported()) return;
    if (Option.isSome(yield* electronWindow.currentMainOrFirst)) return;
    const mode = Option.match(yield* clientSettings.get.pipe(Effect.orElseSucceed(Option.none)), {
      onNone: () => DEFAULT_CLIENT_SETTINGS.notificationMode,
      onSome: (settings) => settings.notificationMode,
    });
    if (mode !== "notifications" && mode !== "notifications-and-sound") return;
    for (const { title, body, target } of notifications) {
      const notification = new Electron.Notification({
        title,
        body,
        silent: mode === "notifications",
      });
      notification.on("click", () => run(desktopWindow.openThread(target)));
      notification.show();
    }
  });

  const start = Effect.gen(function* () {
    yield* sync;
    yield* Stream.runForEach(publisher.agentActivity, (next) =>
      notify(next).pipe(
        Effect.catchCause((cause) =>
          logWarning("tray notification failed", { cause: String(cause) }),
        ),
        Effect.andThen(
          Effect.sync(() => {
            sources = next;
            model = buildDesktopTrayModel(next.values());
          }),
        ),
        Effect.andThen(render),
      ),
    ).pipe(Effect.forkScoped);
    yield* Effect.addFinalizer(() =>
      Effect.sync(() => {
        tray?.destroy();
        tray = null;
      }),
    );
  }).pipe(Effect.withSpan("desktop.tray.start"));

  return DesktopTray.of({
    sync,
    isActive: Effect.sync(() => tray !== null && !tray.isDestroyed()),
    start,
  });
});

export const layer = Layer.effect(DesktopTray, make);
