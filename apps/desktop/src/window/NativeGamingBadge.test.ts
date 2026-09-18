import { afterEach, expect, it, vi } from "vite-plus/test";
import * as NodeEvents from "node:events";
import * as NodeStream from "node:stream";

const { spawn } = vi.hoisted(() => ({ spawn: vi.fn() }));
vi.mock("node:child_process", () => ({ spawn }));
import { startNativeGamingBadge } from "./NativeGamingBadge.ts";

function fixture() {
  const child = Object.assign(new NodeEvents.EventEmitter(), {
    stdin: new NodeStream.PassThrough(),
    stdout: new NodeStream.PassThrough(),
    stderr: new NodeStream.PassThrough(),
    kill: vi.fn(),
  });
  spawn.mockReturnValue(child);
  const activate = vi.fn(),
    failed = vi.fn();
  return {
    child,
    activate,
    failed,
    start: () => startNativeGamingBadge("/helper", activate, failed),
  };
}
afterEach(() => {
  vi.useRealTimers();
  vi.clearAllMocks();
});

it("waits for readiness, handles split receipts and keeps status updates on private stdin", async () => {
  const { child, start, activate, failed } = fixture();
  const starting = start();
  child.stdout.write('{"event":"rea');
  child.stdout.write('dy"}\n');
  const badge = await starting;
  const status = {
    attention: 2,
    unread: 1,
    working: 3,
    offline: 1,
    corner: "bottom-left",
    pulse: true,
  } as const;
  badge.update(status);
  expect(JSON.parse(child.stdin.read().toString())).toEqual({ command: "update", status });
  child.stdout.write('{"event":"activate"}\n');
  expect(activate).toHaveBeenCalledOnce();
  const closed = badge.close();
  expect(child.stdin.read().toString()).toBe('{"command":"close"}\n');
  child.stdout.write('{"event":"activate"}\n');
  child.emit("close");
  await closed;
  await badge.close();
  expect(activate).toHaveBeenCalledOnce();
  expect(failed).not.toHaveBeenCalled();
});

it("reports a lost badge without treating it as a successful close", async () => {
  const { child, start, failed } = fixture();
  const starting = start();
  child.stdout.write('{"event":"ready"}\n');
  const badge = await starting;
  child.stderr.write("Wayland disconnected");
  child.emit("close");
  expect(failed).toHaveBeenCalledWith(expect.stringContaining("Wayland disconnected"));
  await badge.close();
});

it("bounds startup and kills only its own unresponsive child", async () => {
  vi.useFakeTimers();
  const { child, start } = fixture();
  const starting = start();
  const rejection = expect(starting).rejects.toThrow("could not appear");
  await vi.advanceTimersByTimeAsync(5000);
  expect(child.stdin.read().toString()).toContain('"close"');
  await vi.advanceTimersByTimeAsync(1000);
  expect(child.kill).toHaveBeenCalledOnce();
  child.emit("close");
  await rejection;
});

it("rejects launch failure instead of leaving gaming mode partially enabled", async () => {
  const { child, start } = fixture();
  const starting = start();
  const rejection = expect(starting).rejects.toThrow("could not start");
  child.emit("error", new Error("ENOENT"));
  child.emit("close");
  await rejection;
});
