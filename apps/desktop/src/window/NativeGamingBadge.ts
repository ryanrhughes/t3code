// @effect-diagnostics nodeBuiltinImport:off -- The native Wayland surface is owned through private stdio.
// @effect-diagnostics globalTimers:off -- Only startup and child shutdown have deadlines.
import * as NodeChildProcess from "node:child_process";
import type { DesktopGamingBadge } from "@t3tools/contracts";
import * as Schema from "effect/Schema";

const decodeEvent = Schema.decodeUnknownSync(
  Schema.fromJsonString(
    Schema.Struct({
      event: Schema.Literals(["ready", "activate", "done"]),
    }),
  ),
);

export interface GamingBadge {
  update: (status: DesktopGamingBadge) => void;
  close: () => Promise<void>;
}

export async function startNativeGamingBadge(
  executable: string,
  activate: () => void,
  failed: (message: string) => void,
): Promise<GamingBadge> {
  const child = NodeChildProcess.spawn(executable, ["gaming-badge"], {
    stdio: ["pipe", "pipe", "pipe"],
  });
  const ready = Promise.withResolvers<void>();
  const exited = Promise.withResolvers<void>();
  let closing = false;
  let closed = false;
  let started = false;
  let buffer = "";
  let stderr = "";
  let killTimer: ReturnType<typeof setTimeout> | undefined;
  const close = async () => {
    if (!closing && !closed) {
      closing = true;
      child.stdin.end('{"command":"close"}\n');
      killTimer = setTimeout(() => child.kill(), 1000);
      killTimer.unref();
    }
    await exited.promise;
  };
  const fail = (message: string) => {
    ready.reject(new Error(message));
    if (started && !closing) failed(message);
    void close();
  };
  const deadline = setTimeout(
    () => fail("The gaming badge could not appear. Check your Wayland session and try again."),
    5000,
  );
  deadline.unref();
  child.stdin.on("error", () =>
    fail("The gaming badge disconnected. Exit and re-enter gaming mode to reconnect."),
  );
  child.once("error", () =>
    fail("The native gaming badge could not start. Install a desktop build with the Linux helper."),
  );
  child.stderr.setEncoding("utf8");
  child.stderr.on("data", (chunk: string) => {
    stderr = (stderr + chunk).slice(-1024);
  });
  child.once("close", () => {
    closed = true;
    clearTimeout(deadline);
    if (killTimer) clearTimeout(killTimer);
    const message = `The gaming badge stopped.${stderr.trim() ? ` ${stderr.trim()}` : " Exit and re-enter gaming mode to reconnect."}`;
    ready.reject(new Error(message));
    if (started && !closing) failed(message);
    exited.resolve();
  });
  child.stdout.setEncoding("utf8");
  child.stdout.on("data", (chunk: string) => {
    buffer += chunk;
    if (buffer.length > 8192) return fail("Invalid response from the gaming badge.");
    let end: number;
    while ((end = buffer.indexOf("\n")) !== -1) {
      const line = buffer.slice(0, end);
      buffer = buffer.slice(end + 1);
      try {
        const { event } = decodeEvent(line);
        if (event === "ready") {
          clearTimeout(deadline);
          ready.resolve();
        }
        if (event === "activate" && started && !closing) activate();
        if (event === "done" && !closing)
          fail("The gaming badge closed. Exit and re-enter gaming mode to reconnect.");
      } catch {
        fail("Invalid response from the gaming badge.");
      }
    }
  });
  try {
    await ready.promise;
  } catch (error) {
    await close();
    throw error;
  }
  if (closed || closing) {
    await close();
    throw new Error("The gaming badge closed during startup.");
  }
  started = true;
  return {
    update(status) {
      if (!closing && !closed)
        child.stdin.write(`${JSON.stringify({ command: "update", status })}\n`);
    },
    close,
  };
}
