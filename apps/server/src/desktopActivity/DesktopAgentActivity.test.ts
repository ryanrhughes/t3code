import type { EnvironmentId, OrchestrationV2ThreadShell, ThreadId } from "@t3tools/contracts";
import { ProviderInstanceId, RuntimeRequestId } from "@t3tools/contracts";
import { describe, expect, it } from "@effect/vitest";
import * as DateTime from "effect/DateTime";

import {
  applyDesktopAgentActivityThread,
  emptyDesktopAgentActivityState,
  MAX_FINISHED_THREADS,
  type DesktopAgentActivityState,
} from "./DesktopAgentActivity.ts";

const environmentId = "env-local" as EnvironmentId;
const threadId = "thread-1" as ThreadId;
const updatedAt = DateTime.makeUnsafe("2026-10-23T10:00:00.000Z");
const completedAt = DateTime.makeUnsafe("2026-10-23T10:01:00.000Z");

function shell(
  overrides: Partial<
    Pick<
      OrchestrationV2ThreadShell,
      | "id"
      | "status"
      | "pendingRuntimeRequest"
      | "settledOverride"
      | "archivedAt"
      | "latestRunStopRequested"
      | "latestRunCompletedAt"
    >
  > = {},
) {
  const id = overrides.id ?? threadId;
  return {
    id,
    lineage: { rootThreadId: id, parentThreadId: null, relationshipToParent: null },
    title: "Fix the login flow",
    modelSelection: { instanceId: ProviderInstanceId.make("codex"), model: "gpt-5.4" },
    status: "running" as const,
    pendingRuntimeRequest: null,
    settledOverride: null,
    archivedAt: null,
    updatedAt,
    ...overrides,
  };
}

const done = (overrides: Parameters<typeof shell>[0] = {}) =>
  shell({ status: "completed", latestRunCompletedAt: completedAt, ...overrides });

const apply = (state: DesktopAgentActivityState, thread: ReturnType<typeof shell> | null) =>
  applyDesktopAgentActivityThread(
    state,
    thread?.id ?? threadId,
    thread === null ? null : { environmentId, projectTitle: "t3code", thread },
  );

describe("applyDesktopAgentActivityThread", () => {
  it("lists a working thread and moves it to finished when its run completes", () => {
    const working = apply(emptyDesktopAgentActivityState, shell());
    expect([...working.active.values()]).toEqual([
      { threadId, title: "Fix the login flow", projectTitle: "t3code", phase: "working" },
    ]);

    const finished = apply(working, done());
    expect(finished.active.size).toBe(0);
    expect(finished.finished).toEqual([
      {
        threadId,
        title: "Fix the login flow",
        projectTitle: "t3code",
        outcome: "completed",
        finishedAt: "2026-10-23T10:01:00.000Z",
      },
    ]);
  });

  it("splits pending requests into approvals and input", () => {
    const request = (kind: "command" | "user_input") => ({
      id: RuntimeRequestId.make(`request-${kind}`),
      kind,
      createdAt: updatedAt,
    });
    const approval = apply(
      emptyDesktopAgentActivityState,
      shell({ pendingRuntimeRequest: request("command") }),
    );
    expect(approval.active.get(threadId)?.phase).toBe("approval");
    const input = apply(
      emptyDesktopAgentActivityState,
      shell({ pendingRuntimeRequest: request("user_input") }),
    );
    expect(input.active.get(threadId)?.phase).toBe("input");
  });

  it("does not report history as finished, only threads it saw working", () => {
    expect(apply(emptyDesktopAgentActivityState, done()).finished).toEqual([]);
  });

  it("records failures and drops a thread the user stopped", () => {
    const working = apply(emptyDesktopAgentActivityState, shell());
    expect(apply(working, shell({ status: "failed" })).finished[0]?.outcome).toBe("failed");

    const stopped = apply(working, done({ latestRunStopRequested: true }));
    expect(stopped.active.size).toBe(0);
    expect(stopped.finished).toEqual([]);
  });

  it("removes a finished thread when it runs again, is settled, archived, or deleted", () => {
    const finished = apply(apply(emptyDesktopAgentActivityState, shell()), done());
    expect(apply(finished, shell()).finished).toEqual([]);
    expect(apply(finished, done({ settledOverride: "settled" })).finished).toEqual([]);
    expect(apply(finished, done({ archivedAt: completedAt })).finished).toEqual([]);
    expect(apply(finished, null).finished).toEqual([]);
  });

  it("keeps only the most recent finished threads", () => {
    let state = emptyDesktopAgentActivityState;
    for (let index = 0; index < MAX_FINISHED_THREADS + 2; index++) {
      const id = `thread-${index}` as ThreadId;
      state = apply(apply(state, shell({ id })), done({ id }));
    }
    expect(state.finished.map((entry) => entry.threadId)).toEqual([
      "thread-6",
      "thread-5",
      "thread-4",
      "thread-3",
      "thread-2",
    ]);
  });
});
