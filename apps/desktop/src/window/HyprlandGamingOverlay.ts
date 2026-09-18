// @effect-diagnostics nodeBuiltinImport:off -- Hyprland's IPC CLI is the compositor boundary.
import * as NodeChildProcess from "node:child_process";
import * as Schema from "effect/Schema";

const Clients = Schema.Array(
  Schema.Struct({
    address: Schema.String,
    pid: Schema.Int,
    title: Schema.String,
    floating: Schema.Boolean,
    workspace: Schema.Struct({ id: Schema.Int }),
    at: Schema.Tuple([Schema.Finite, Schema.Finite]),
    size: Schema.Tuple([Schema.Finite, Schema.Finite]),
  }),
);
const Binds = Schema.Array(
  Schema.Struct({
    modmask: Schema.Int,
    key: Schema.String,
    keycode: Schema.optional(Schema.Int),
    dispatcher: Schema.String,
    arg: Schema.String,
  }),
);
const Monitors = Schema.Array(
  Schema.Struct({
    specialWorkspace: Schema.Struct({ name: Schema.String }),
  }),
);

const decodeClients = Schema.decodeUnknownSync(Clients);
const decodeBinds = Schema.decodeUnknownSync(Binds);
const decodeMonitors = Schema.decodeUnknownSync(Monitors);

export type Hyprctl = (args: readonly string[]) => Promise<string>;

export const runHyprctl: Hyprctl = (args) =>
  new Promise((resolve, reject) => {
    NodeChildProcess.execFile(
      "hyprctl",
      [...args],
      { timeout: 5_000, maxBuffer: 2 * 1024 * 1024 },
      (error, stdout) => {
        if (error) reject(new Error(`Hyprland gaming overlay: ${error.message}`));
        else resolve(stdout.trim());
      },
    );
  });

/** Owns only this window's scratchpad and a temporary, previously unused binding. */
export async function enterHyprlandGamingOverlay(
  input: { pid: number; windowId: number; title: string },
  run: Hyprctl = runHyprctl,
) {
  const [clients, binds] = await Promise.all([
    run(["-j", "clients"]).then((raw) => decodeClients(JSON.parse(raw))),
    run(["-j", "binds"]).then((raw) => decodeBinds(JSON.parse(raw))),
  ]);
  const matches = clients.filter(
    (client) => client.pid === input.pid && client.title === input.title,
  );
  const client = matches.length === 1 ? matches[0] : undefined;
  if (!client || !/^0x[\da-f]+$/i.test(client.address)) {
    throw new Error("Could not identify T3's window in Hyprland. Focus T3 and try again.");
  }
  if (client.workspace.id <= 0) {
    throw new Error("Move T3 to a regular workspace before entering gaming mode.");
  }
  // Do not replace a user's binding, including bindings expressed as keycodes.
  const candidates = [
    { mask: 5, mods: "CTRL SHIFT", lua: "CTRL + SHIFT + space", label: "Ctrl+Shift+Space" },
    { mask: 12, mods: "CTRL ALT", lua: "CTRL + ALT + space", label: "Ctrl+Alt+Space" },
    { mask: 65, mods: "SUPER SHIFT", lua: "SUPER + SHIFT + space", label: "Super+Shift+Space" },
  ];
  const key = candidates.find(
    (candidate) =>
      !binds.some(
        (bind) =>
          bind.modmask === candidate.mask &&
          (bind.key.toLowerCase() === "space" || bind.keycode === 65),
      ),
  );
  if (!key)
    throw new Error("Gaming shortcuts are already assigned. Free Ctrl+Shift+Space and try again.");

  const name = `t3-gaming-${input.pid}-${input.windowId}`;
  const address = `address:${client.address}`;
  const handle = `t3_gaming_${input.pid}_${input.windowId}`;
  const lua = await run(["eval", "assert(hl and hl.dsp)"]).then(
    (reply) => reply === "ok",
    () => false,
  );
  const command = async (args: string[]) => {
    const reply = await run(args);
    if (reply !== "ok") throw new Error(`Hyprland gaming overlay: ${reply}`);
  };
  const dispatch = (legacy: string[], expression: string) =>
    command(
      lua
        ? [
            "eval",
            `local result=hl.dispatch(${expression}) if result and result.ok==false then error(result.error or "Compositor action failed") end`,
          ]
        : ["dispatch", ...legacy],
    );
  let bound = false;
  let moved = false;
  let floated = false;
  const hide = async () => {
    const monitors = decodeMonitors(JSON.parse(await run(["-j", "monitors"])));
    if (monitors.some((monitor) => monitor.specialWorkspace.name === `special:${name}`)) {
      await dispatch(
        ["togglespecialworkspace", name],
        `hl.dsp.workspace.toggle_special("${name}")`,
      );
    }
  };
  const show = async () => {
    const monitors = decodeMonitors(JSON.parse(await run(["-j", "monitors"])));
    if (!monitors.some((monitor) => monitor.specialWorkspace.name === `special:${name}`)) {
      await dispatch(
        ["togglespecialworkspace", name],
        `hl.dsp.workspace.toggle_special("${name}")`,
      );
    }
    await dispatch(["focuswindow", address], `hl.dsp.focus({window="${address}"})`);
  };
  const restore = async () => {
    const failures: unknown[] = [];
    const attempt = async (action: () => Promise<void>) => {
      try {
        await action();
      } catch (error) {
        failures.push(error);
      }
    };
    if (moved) {
      await attempt(hide);
      await attempt(() =>
        dispatch(
          ["movetoworkspacesilent", `${client.workspace.id},${address}`],
          `hl.dsp.window.move({window="${address}",workspace="${client.workspace.id}",follow=false})`,
        ),
      );
    }
    if (floated) {
      if (client.floating) {
        await attempt(() =>
          dispatch(
            ["resizewindowpixel", `exact ${client.size[0]} ${client.size[1]},${address}`],
            `hl.dsp.window.resize({window="${address}",x=${client.size[0]},y=${client.size[1]}})`,
          ),
        );
        await attempt(() =>
          dispatch(
            ["movewindowpixel", `exact ${client.at[0]} ${client.at[1]},${address}`],
            `hl.dsp.window.move({window="${address}",x=${client.at[0]},y=${client.at[1]}})`,
          ),
        );
      } else {
        await attempt(() =>
          dispatch(
            ["settiled", address],
            `hl.dsp.window.float({window="${address}",action="off"})`,
          ),
        );
      }
    }
    if (bound) {
      await attempt(async () => {
        if (lua) {
          await command([
            "eval",
            `if _G.${handle} then _G.${handle}:unbind() _G.${handle}=nil end`,
          ]);
        } else {
          const current = decodeBinds(JSON.parse(await run(["-j", "binds"])));
          const matching = current.filter(
            (bind) => bind.modmask === key.mask && bind.key.toLowerCase() === "space",
          );
          // A config reload may have replaced our temporary binding. Never remove its replacement.
          if (
            matching.length === 1 &&
            matching[0]?.dispatcher === "togglespecialworkspace" &&
            matching[0].arg === name
          ) {
            await command(["keyword", "unbind", `${key.mods},space`]);
          }
        }
        bound = false;
      });
    }
    if (failures.length)
      throw new AggregateError(
        failures,
        "Could not fully restore the Hyprland window. Exit gaming mode again to retry.",
      );
    moved = false;
    floated = false;
  };

  try {
    await command(
      lua
        ? ["eval", `_G.${handle}=hl.bind("${key.lua}",hl.dsp.workspace.toggle_special("${name}"))`]
        : ["keyword", "bind", `${key.mods},space,togglespecialworkspace,${name}`],
    );
    bound = true;
    await dispatch(
      ["setfloating", address],
      `hl.dsp.window.float({window="${address}",action="on"})`,
    );
    floated = true;
    await dispatch(
      ["movetoworkspacesilent", `special:${name},${address}`],
      `hl.dsp.window.move({window="${address}",workspace="special:${name}",follow=false})`,
    );
    moved = true;
    await dispatch(
      ["resizewindowpixel", `exact 520 720,${address}`],
      `hl.dsp.window.resize({window="${address}",x=520,y=720})`,
    );
    await dispatch(["togglespecialworkspace", name], `hl.dsp.workspace.toggle_special("${name}")`);
  } catch (error) {
    try {
      await restore();
    } catch (restoreError) {
      throw new AggregateError(
        [error, restoreError],
        "Gaming mode setup failed and Hyprland could not fully restore the window. Reload your Hyprland configuration to clear the temporary shortcut.",
        { cause: restoreError },
      );
    }
    throw error;
  }
  return { shortcutLabel: key.label, hide, show, restore };
}
