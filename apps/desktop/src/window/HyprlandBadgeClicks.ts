// @effect-diagnostics nodeBuiltinImport:off -- Hyprland's event socket is the compositor boundary.
import * as NodeNet from "node:net";
import * as NodePath from "node:path";
import type { Hyprctl } from "./HyprlandGamingOverlay.ts";

/**
 * Delivers badge clicks while a game holds an active pointer constraint.
 *
 * Hyprland resolves pointer input against the constrained surface before it
 * hit-tests layer surfaces, so a fullscreen game with a locked or confined
 * cursor swallows every click aimed at the badge. Keybinds still run first,
 * so a temporary left-button binding checks the cursor against the badge's
 * layer geometry, reports a hit as a socket2 custom event, and passes every
 * other click to the game untouched. The bind is auto-consuming: it only
 * swallows the click it claimed.
 */
export interface HyprlandBadgeClicks {
  close: () => Promise<void>;
}

const EVENT = "t3-gaming-badge";

function luaBind(handle: string, namespace: string) {
  // Lua runs inside Hyprland's 100ms keybind budget; only compare geometry here.
  return [
    `_G.${handle}=hl.bind("mouse:272",function()`,
    "local c=hl.get_cursor_pos()",
    `for _,l in ipairs(hl.get_layers({namespace="${namespace}"})) do`,
    "if l.mapped then local dx=c.x-(l.x+l.w/2) local dy=c.y-(l.y+l.h/2)",
    "if dx*dx+dy*dy<=(l.w/2)*(l.w/2) then",
    `hl.dispatch(hl.dsp.event("${EVENT}")) return {ok=true,pass_event=false} end end end`,
    "return {ok=false,pass_event=true} end,{auto_consuming=true})",
  ].join(" ");
}

export async function startHyprlandBadgeClicks(input: {
  namespace: string;
  handle: string;
  env: NodeJS.ProcessEnv;
  run: Hyprctl;
  activate: () => void;
  connect?: (path: string) => NodeNet.Socket;
}): Promise<HyprlandBadgeClicks> {
  const signature = input.env.HYPRLAND_INSTANCE_SIGNATURE;
  const runtimeDir = input.env.XDG_RUNTIME_DIR;
  if (!signature || !runtimeDir) throw new Error("Hyprland's event socket is not available.");
  const socketPath = NodePath.join(runtimeDir, "hypr", signature, ".socket2.sock");
  const connect = input.connect ?? ((path) => NodeNet.createConnection(path));
  const socket = connect(socketPath);
  socket.setEncoding("utf8");
  let buffer = "";
  let closed = false;
  socket.on("data", (chunk: string) => {
    buffer += chunk;
    let end: number;
    while ((end = buffer.indexOf("\n")) !== -1) {
      const line = buffer.slice(0, end);
      buffer = buffer.slice(end + 1);
      if (line === `custom>>${EVENT}` && !closed) input.activate();
    }
    if (buffer.length > 4096) buffer = "";
  });
  socket.on("error", () => undefined);
  await new Promise<void>((resolve, reject) => {
    socket.once("connect", resolve);
    socket.once("error", reject);
  });
  const reply = await input.run(["eval", luaBind(input.handle, input.namespace)]);
  if (reply !== "ok") {
    socket.destroy();
    throw new Error(`Hyprland gaming overlay: ${reply}`);
  }
  return {
    close: async () => {
      if (closed) return;
      closed = true;
      socket.destroy();
      const reply = await input.run([
        "eval",
        `if _G.${input.handle} then _G.${input.handle}:unbind() _G.${input.handle}=nil end`,
      ]);
      if (reply !== "ok") throw new Error(`Hyprland gaming overlay: ${reply}`);
    },
  };
}
