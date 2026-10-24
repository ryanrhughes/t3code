import {
  DesktopTelemetrySetAgentActivity,
  type DesktopAgentActivityFinishedThread,
  type DesktopAgentActivityPhase,
  type DesktopAgentActivityThread,
  type EnvironmentId,
  type OrchestrationV2ThreadShell,
  type ProjectId,
  type ThreadId,
} from "@t3tools/contracts";
import {
  projectThreadAwarenessV2,
  type AgentAwarenessPhase,
  type ProjectThreadAwarenessV2Input,
} from "@t3tools/shared/agentAwareness";
import { makeDrainableWorker } from "@t3tools/shared/DrainableWorker";
import * as Cause from "effect/Cause";
import * as Context from "effect/Context";
import * as DateTime from "effect/DateTime";
import * as Effect from "effect/Effect";
import * as Layer from "effect/Layer";
import * as Option from "effect/Option";
import * as Schema from "effect/Schema";
import type * as Scope from "effect/Scope";
import * as Stream from "effect/Stream";

import { ServerConfig } from "../config.ts";
import * as ServerEnvironment from "../environment/ServerEnvironment.ts";
import * as ThreadManagement from "../orchestration-v2/ThreadManagementService.ts";
import * as ProjectService from "../project/ProjectService.ts";
import { shouldPublishAgentAwarenessEvent } from "../relay/AgentAwarenessRelay.ts";
import { DesktopTelemetryReceiver } from "../resourceTelemetry/DesktopTelemetryReceiver.ts";
import { forkParked } from "../serverActivation.ts";

/**
 * Feeds the desktop tray. A desktop-supervised server keeps a small snapshot
 * of which threads are working or waiting on the user, plus the last few that
 * finished, and pushes it down the desktop control channel whenever it
 * changes. The desktop main process renders it without needing a renderer.
 */
export class DesktopAgentActivity extends Context.Service<
  DesktopAgentActivity,
  {
    readonly start: () => Effect.Effect<void, never, Scope.Scope>;
  }
>()("t3/desktopActivity/DesktopAgentActivity") {}

export const MAX_FINISHED_THREADS = 5;

const DesktopAgentActivitySnapshot = Schema.Struct({
  active: DesktopTelemetrySetAgentActivity.fields.active,
  finished: DesktopTelemetrySetAgentActivity.fields.finished,
});
const sameSnapshot = Schema.toEquivalence(DesktopAgentActivitySnapshot);

type ThreadActivityInput = ProjectThreadAwarenessV2Input["thread"] &
  Pick<OrchestrationV2ThreadShell, "archivedAt" | "latestRunCompletedAt">;

export interface DesktopAgentActivityState {
  readonly active: ReadonlyMap<ThreadId, DesktopAgentActivityThread>;
  readonly finished: ReadonlyArray<DesktopAgentActivityFinishedThread>;
}

export const emptyDesktopAgentActivityState: DesktopAgentActivityState = {
  active: new Map(),
  finished: [],
};

function trayPhase(phase: AgentAwarenessPhase): DesktopAgentActivityPhase | null {
  switch (phase) {
    case "starting":
    case "running":
      return "working";
    case "waiting_for_approval":
      return "approval";
    case "waiting_for_input":
      return "input";
    case "completed":
    case "failed":
    case "stale":
      return null;
  }
}

/**
 * Folds the latest shell of one thread into the snapshot, using the same
 * awareness projection as T3 Connect's agent activity. `input` is null when
 * the thread or its project is gone. A finish only counts once the thread was
 * seen working, so history from before startup never shows up.
 */
export function applyDesktopAgentActivityThread(
  state: DesktopAgentActivityState,
  threadId: ThreadId,
  input: {
    readonly environmentId: EnvironmentId;
    readonly projectTitle: string;
    readonly thread: ThreadActivityInput;
  } | null,
): DesktopAgentActivityState {
  const active = new Map(state.active);
  const wasActive = active.delete(threadId);
  const others = state.finished.filter((entry) => entry.threadId !== threadId);
  const awareness =
    input === null || input.thread.archivedAt !== null
      ? null
      : projectThreadAwarenessV2({
          environmentId: input.environmentId,
          project: { title: input.projectTitle },
          thread: input.thread,
        });
  // Stopped, settled, archived, or deleted threads leave both lists.
  if (input === null || awareness === null) {
    return { active, finished: others };
  }
  const title = awareness.threadTitle;
  const projectTitle = awareness.projectTitle;
  const phase = trayPhase(awareness.phase);
  if (phase !== null) {
    active.set(threadId, { threadId, title, projectTitle, phase });
    return { active, finished: others };
  }
  if (awareness.phase !== "completed" && awareness.phase !== "failed") {
    return { active, finished: others };
  }
  if (wasActive) {
    const entry = {
      threadId,
      title,
      projectTitle,
      outcome: awareness.phase,
      finishedAt: DateTime.formatIso(input.thread.latestRunCompletedAt ?? input.thread.updatedAt),
    };
    return { active, finished: [entry, ...others].slice(0, MAX_FINISHED_THREADS) };
  }
  // Keep an earlier finish in place, picking up renames.
  return {
    active,
    finished: state.finished.map((entry) =>
      entry.threadId === threadId ? { ...entry, title, projectTitle } : entry,
    ),
  };
}

/** @public Service construction is part of the canonical Effect module API. */
export const make = Effect.gen(function* () {
  const config = yield* ServerConfig;
  const receiver = yield* DesktopTelemetryReceiver;
  const serverEnvironment = yield* ServerEnvironment.ServerEnvironment;
  const threads = yield* ThreadManagement.ThreadManagementService;
  const projects = yield* ProjectService.ProjectService;

  const start: DesktopAgentActivity["Service"]["start"] = Effect.fn("DesktopAgentActivity.start")(
    function* () {
      // Only a server the desktop app supervises has a tray to feed.
      if (config.desktopTelemetryControlFd === undefined) return;
      const environmentId = yield* serverEnvironment.getEnvironmentId;
      let state = emptyDesktopAgentActivityState;
      let sent: typeof DesktopAgentActivitySnapshot.Type | null = null;
      // Threads already queued, so a burst of events on one thread costs one refresh.
      const queued = new Set<ThreadId>();

      const send = Effect.gen(function* () {
        const snapshot = { active: [...state.active.values()], finished: state.finished };
        if (sent !== null && sameSnapshot(sent, snapshot)) return;
        yield* receiver.setAgentActivity({ environmentId, ...snapshot });
        sent = snapshot;
      });

      const loadAll = Effect.gen(function* () {
        const [projectSnapshot, shellSnapshot] = yield* Effect.all([
          projects.snapshot,
          threads.getShellSnapshot(),
        ]);
        const projectTitles = new Map<ProjectId, string>(
          projectSnapshot.projects.map((project) => [project.id, project.title]),
        );
        let next = emptyDesktopAgentActivityState;
        for (const thread of shellSnapshot.threads) {
          const projectTitle = projectTitles.get(thread.projectId);
          if (projectTitle === undefined) continue;
          next = applyDesktopAgentActivityThread(next, thread.id, {
            environmentId,
            projectTitle,
            thread,
          });
        }
        state = next;
      });

      const refreshThread = Effect.fn("DesktopAgentActivity.refreshThread")(function* (
        threadId: ThreadId,
      ) {
        queued.delete(threadId);
        const thread = yield* threads.getThreadShell(threadId);
        const project = thread === null ? Option.none() : yield* projects.getById(thread.projectId);
        state = applyDesktopAgentActivityThread(
          state,
          threadId,
          thread !== null && Option.isSome(project)
            ? { environmentId, projectTitle: project.value.title, thread }
            : null,
        );
      });

      const process = (item: ThreadId | "all") =>
        (item === "all" ? loadAll : refreshThread(item)).pipe(
          Effect.andThen(send),
          Effect.catchCause((cause) =>
            Effect.logWarning("desktop agent activity update failed", {
              cause: Cause.pretty(cause),
            }),
          ),
        );

      yield* forkParked(
        Effect.gen(function* () {
          const worker = yield* makeDrainableWorker(process);
          yield* Stream.runForEach(threads.streamDomainEvents, (event) => {
            if (queued.has(event.threadId) || !shouldPublishAgentAwarenessEvent(event)) {
              return Effect.void;
            }
            queued.add(event.threadId);
            return worker.enqueue(event.threadId);
          }).pipe(Effect.forkScoped);
          // Let the event subscription attach before the initial load so a
          // change between the two is not lost.
          yield* Effect.yieldNow;
          yield* worker.enqueue("all");
        }),
      );
    },
  );

  return DesktopAgentActivity.of({ start });
});

export const layer = Layer.effect(DesktopAgentActivity, make);
