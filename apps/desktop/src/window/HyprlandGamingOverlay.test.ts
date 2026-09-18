import { describe, expect, it } from "vite-plus/test";
import { enterHyprlandGamingOverlay, type Hyprctl } from "./HyprlandGamingOverlay.ts";

const identity = { pid: 42, windowId: 7, title: "T3 Code" };
const workspace = "t3-gaming-42-7";
const originalClient = {
  address: "0xabc",
  pid: 42,
  title: "T3 Code",
  floating: false,
  workspace: { id: 3 },
  at: [20, 40],
  size: [1100, 780],
};
function compositor(
  options: { lua?: boolean; occupied?: boolean; failMove?: boolean; ambiguous?: boolean } = {},
) {
  let binds = options.occupied
    ? [{ modmask: 5, key: "space", dispatcher: "exec", arg: "my-command" }]
    : [];
  const calls: string[][] = [];
  let visible = false;
  const run: Hyprctl = async (args) => {
    calls.push([...args]);
    if (args[0] === "-j") {
      if (args[1] === "clients")
        return JSON.stringify(
          options.ambiguous ? [originalClient, originalClient] : [originalClient],
        );
      if (args[1] === "binds") return JSON.stringify(binds);
      if (args[1] === "monitors")
        return JSON.stringify([
          { specialWorkspace: { name: visible ? `special:${workspace}` : "" } },
        ]);
    }
    if (
      args[1] === "togglespecialworkspace" ||
      (args[0] === "eval" &&
        args[1]?.startsWith("local result=") &&
        args[1].includes("toggle_special"))
    )
      visible = !visible;
    if (args[0] === "eval" && !options.lua) return "unknown request";
    if (args[0] === "keyword" && args[1] === "bind") {
      binds = [
        ...binds,
        {
          modmask: options.occupied ? 12 : 5,
          key: "space",
          dispatcher: "togglespecialworkspace",
          arg: workspace,
        },
      ];
    }
    if (options.failMove && args[1] === "movetoworkspacesilent") return "move failed";
    return "ok";
  };
  return {
    run,
    calls,
    replaceBindings: () => {
      binds = [{ modmask: 5, key: "space", dispatcher: "exec", arg: "new-user-command" }];
    },
  };
}

describe("Hyprland gaming overlay", () => {
  it.each([false, true])(
    "badge activation shows a hidden panel without closing an already open one (Lua: %s)",
    async (lua) => {
      const { run, calls } = compositor({ lua });
      const host = await enterHyprlandGamingOverlay(identity, run);
      await host.hide();
      const start = calls.length;
      await host.show();
      await host.show();
      const showing = calls.slice(start);
      if (lua) {
        expect(showing.filter((args) => args[1]?.includes("toggle_special"))).toHaveLength(1);
        expect(
          showing.filter((args) => args[1]?.includes('hl.dsp.focus({window="address:0xabc"})')),
        ).toHaveLength(2);
      } else {
        expect(showing.filter((args) => args[1] === "togglespecialworkspace")).toHaveLength(1);
        expect(showing.filter((args) => args[1] === "focuswindow")).toHaveLength(2);
      }
      await host.restore();
    },
  );
  it("moves only the identified T3 window and restores its workspace and tiling", async () => {
    const { run, calls } = compositor();
    const host = await enterHyprlandGamingOverlay(identity, run);
    expect(host.shortcutLabel).toBe("Ctrl+Shift+Space");
    expect(calls).toContainEqual([
      "dispatch",
      "movetoworkspacesilent",
      `special:${workspace},address:0xabc`,
    ]);
    await host.restore();
    expect(calls).toContainEqual(["dispatch", "movetoworkspacesilent", "3,address:0xabc"]);
    expect(calls).toContainEqual(["dispatch", "settiled", "address:0xabc"]);
    expect(calls).toContainEqual(["keyword", "unbind", "CTRL SHIFT,space"]);
  });

  it("chooses an unused shortcut without replacing an existing binding", async () => {
    const { run, calls } = compositor({ occupied: true });
    const host = await enterHyprlandGamingOverlay(identity, run);
    expect(host.shortcutLabel).toBe("Ctrl+Alt+Space");
    await host.restore();
    expect(calls).not.toContainEqual(["keyword", "unbind", "CTRL SHIFT,space"]);
  });

  it("does not remove a binding replaced by a user config reload", async () => {
    const { run, calls, replaceBindings } = compositor();
    const host = await enterHyprlandGamingOverlay(identity, run);
    replaceBindings();
    await host.restore();
    expect(calls.filter((args) => args[1] === "unbind")).toEqual([]);
  });

  it("rolls back a partial setup failure", async () => {
    const { run, calls } = compositor({ failMove: true });
    await expect(enterHyprlandGamingOverlay(identity, run)).rejects.toThrow("move failed");
    expect(calls).toContainEqual(["dispatch", "settiled", "address:0xabc"]);
    expect(calls).toContainEqual(["keyword", "unbind", "CTRL SHIFT,space"]);
  });

  it("refuses an ambiguous window match before mutating the desktop", async () => {
    const { run, calls } = compositor({ ambiguous: true });
    await expect(enterHyprlandGamingOverlay(identity, run)).rejects.toThrow("identify T3's window");
    expect(calls.every((args) => args[0] === "-j")).toBe(true);
  });

  it("uses Lua binding ownership on modern Hyprland", async () => {
    const { run, calls } = compositor({ lua: true });
    const host = await enterHyprlandGamingOverlay(identity, run);
    await host.restore();
    expect(calls.some((args) => args[0] === "keyword")).toBe(false);
    expect(calls.some((args) => args[1]?.includes("_G.t3_gaming_42_7:unbind()"))).toBe(true);
    expect(calls.some((args) => args[1]?.includes('workspace="3"'))).toBe(true);
  });
});
