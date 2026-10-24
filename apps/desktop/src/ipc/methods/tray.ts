import * as Effect from "effect/Effect";
import * as Schema from "effect/Schema";

import * as DesktopAppSettings from "../../settings/DesktopAppSettings.ts";
import * as DesktopTray from "../../window/DesktopTray.ts";
import * as IpcChannels from "../channels.ts";
import { makeIpcMethod, makeSyncIpcMethod } from "../DesktopIpc.ts";

export const getTrayEnabled = makeSyncIpcMethod({
  channel: IpcChannels.GET_TRAY_ENABLED_CHANNEL,
  result: Schema.Boolean,
  handler: Effect.fn("desktop.ipc.tray.getEnabled")(function* () {
    const appSettings = yield* DesktopAppSettings.DesktopAppSettings;
    return (yield* appSettings.get).trayEnabled;
  }),
});

export const setTrayEnabled = makeIpcMethod({
  channel: IpcChannels.SET_TRAY_ENABLED_CHANNEL,
  payload: Schema.Boolean,
  result: Schema.Void,
  handler: Effect.fn("desktop.ipc.tray.setEnabled")(function* (enabled) {
    const appSettings = yield* DesktopAppSettings.DesktopAppSettings;
    const tray = yield* DesktopTray.DesktopTray;
    yield* appSettings.setTrayEnabled(enabled);
    yield* tray.sync;
  }),
});
