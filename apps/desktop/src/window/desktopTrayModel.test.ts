import type { DesktopTelemetrySetAgentActivity, EnvironmentId, ThreadId } from "@t3tools/contracts";
import { assert, describe, it } from "@effect/vitest";

import {
  buildDesktopTrayModel,
  formatDesktopTrayLabel,
  paintDesktopTrayStatusDot,
  resolveDesktopTrayNotifications,
} from "./desktopTrayModel.ts";

const environmentId = "env-local" as EnvironmentId;

function activity(
  input: Partial<Pick<DesktopTelemetrySetAgentActivity, "active" | "finished">> & {
    readonly environmentId?: EnvironmentId;
  } = {},
): DesktopTelemetrySetAgentActivity {
  return {
    version: 1,
    type: "setAgentActivity",
    environmentId: input.environmentId ?? environmentId,
    active: input.active ?? [],
    finished: input.finished ?? [],
  };
}

const thread = (id: string, phase: "working" | "approval" | "input") => ({
  threadId: id as ThreadId,
  title: `Thread ${id}`,
  projectTitle: "t3code",
  phase,
});

const finished = (
  id: string,
  finishedAt: string,
  outcome: "completed" | "failed" = "completed",
) => ({
  threadId: id as ThreadId,
  title: `Thread ${id}`,
  projectTitle: "t3code",
  outcome,
  finishedAt,
});

describe("buildDesktopTrayModel", () => {
  it("merges backends, flags attention, and orders finished threads newest first", () => {
    const wsl = "env-wsl" as EnvironmentId;
    const model = buildDesktopTrayModel([
      activity({
        active: [thread("a", "working"), thread("b", "approval")],
        finished: [finished("c", "2026-10-23T10:00:00.000Z")],
      }),
      activity({
        environmentId: wsl,
        active: [thread("d", "working")],
        finished: [finished("e", "2026-10-23T11:00:00.000Z", "failed")],
      }),
    ]);

    assert.equal(model.status, "attention");
    assert.equal(model.summary, "1 needs you · 2 working");
    assert.deepEqual(
      model.needsYou.map((item) => [item.threadId, item.phase]),
      [["b", "approval"]],
    );
    assert.deepEqual(
      model.working.map((item) => [item.environmentId, item.threadId]),
      [
        [environmentId, "a"],
        [wsl, "d"],
      ],
    );
    assert.deepEqual(
      model.finished.map((item) => [item.threadId, item.outcome]),
      [
        ["e", "failed"],
        ["c", "completed"],
      ],
    );
  });

  it("reads idle when nothing is running", () => {
    const model = buildDesktopTrayModel([activity()]);
    assert.equal(model.status, "idle");
    assert.equal(model.summary, "No agents running");
  });
});

describe("formatDesktopTrayLabel", () => {
  it("escapes menu mnemonics and truncates long titles", () => {
    assert.equal(
      formatDesktopTrayLabel({ title: "Fix R&D export", projectTitle: "t3code" }),
      "Fix R&&D export · t3code",
    );
    const label = formatDesktopTrayLabel({ title: "x".repeat(100), projectTitle: "t3code" });
    assert.equal(label.length, 60);
    assert.isTrue(label.endsWith("…"));
  });
});

describe("resolveDesktopTrayNotifications", () => {
  it("treats a backend's first report as the baseline", () => {
    const next = new Map([["primary", activity({ active: [thread("a", "approval")] })]]);
    assert.deepEqual(resolveDesktopTrayNotifications(new Map(), next), []);
  });

  it("notifies when a thread starts waiting or finishes, once each", () => {
    const previous = new Map([
      ["primary", activity({ active: [thread("a", "working"), thread("b", "working")] })],
    ]);
    const next = new Map([
      [
        "primary",
        activity({
          active: [thread("a", "input")],
          finished: [finished("b", "2026-10-23T10:00:00.000Z")],
        }),
      ],
    ]);

    assert.deepEqual(resolveDesktopTrayNotifications(previous, next), [
      {
        title: "Input needed",
        body: "Thread a",
        target: { environmentId, threadId: "a" as ThreadId },
      },
      {
        title: "Thread completed",
        body: "Thread b",
        target: { environmentId, threadId: "b" as ThreadId },
      },
    ]);
    assert.deepEqual(resolveDesktopTrayNotifications(next, next), []);
  });
});

describe("paintDesktopTrayStatusDot", () => {
  it("paints the dot color in the bottom-right corner and leaves the rest alone", () => {
    const size = 32;
    const pixels = new Uint8Array(size * size * 4).fill(255);
    paintDesktopTrayStatusDot(pixels, size, { red: 245, green: 158, blue: 11 });

    const corner = ((size - 6) * size + (size - 6)) * 4;
    assert.deepEqual(Array.from(pixels.subarray(corner, corner + 4)), [11, 158, 245, 255]);
    assert.deepEqual(Array.from(pixels.subarray(0, 4)), [255, 255, 255, 255]);
  });
});
