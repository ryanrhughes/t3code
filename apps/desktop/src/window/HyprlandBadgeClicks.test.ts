import { expect, it, vi } from "vite-plus/test";
import * as NodeEvents from "node:events";
import { startHyprlandBadgeClicks } from "./HyprlandBadgeClicks.ts";

function fakeSocket() {
  const socket = Object.assign(new NodeEvents.EventEmitter(), {
    setEncoding: vi.fn(),
    destroy: vi.fn(),
  });
  return socket as unknown as import("node:net").Socket & typeof socket;
}

it("claims only clicks on the badge and unbinds on close", async () => {
  const socket = fakeSocket();
  const calls: string[][] = [];
  const activate = vi.fn();
  const starting = startHyprlandBadgeClicks({
    namespace: "t3-gaming-badge",
    handle: "t3_gaming_clicks",
    env: { HYPRLAND_INSTANCE_SIGNATURE: "sig", XDG_RUNTIME_DIR: "/run/user/1000" },
    run: async (args) => {
      calls.push([...args]);
      return "ok";
    },
    activate,
    connect: (path) => {
      expect(path).toBe("/run/user/1000/hypr/sig/.socket2.sock");
      queueMicrotask(() => socket.emit("connect"));
      return socket;
    },
  });
  const clicks = await starting;
  const bind = calls[0]?.[1] ?? "";
  expect(bind).toContain('hl.bind("mouse:272"');
  expect(bind).toContain("auto_consuming=true");
  expect(bind).toContain('namespace="t3-gaming-badge"');
  expect(bind).toContain("pass_event=true");
  socket.emit("data", "activewindow>>foo,bar\ncustom>>t3-gam");
  expect(activate).not.toHaveBeenCalled();
  socket.emit("data", "ing-badge\ncustom>>other\n");
  expect(activate).toHaveBeenCalledOnce();
  await clicks.close();
  expect(socket.destroy).toHaveBeenCalledOnce();
  expect(calls.at(-1)?.[1]).toContain("t3_gaming_clicks:unbind()");
  socket.emit("data", "custom>>t3-gaming-badge\n");
  expect(activate).toHaveBeenCalledOnce();
});

it("fails without binding when the compositor rejects the shortcut", async () => {
  const socket = fakeSocket();
  await expect(
    startHyprlandBadgeClicks({
      namespace: "t3-gaming-badge",
      handle: "h",
      env: { HYPRLAND_INSTANCE_SIGNATURE: "sig", XDG_RUNTIME_DIR: "/run/user/1000" },
      run: async () => "error: bad",
      activate: vi.fn(),
      connect: () => {
        queueMicrotask(() => socket.emit("connect"));
        return socket;
      },
    }),
  ).rejects.toThrow("bad");
  expect(socket.destroy).toHaveBeenCalledOnce();
});
