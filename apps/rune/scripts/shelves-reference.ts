// Runs the web's sidebar rules on edge cases into crates/t3-client/tests/fixtures/shelves.json,
// which tests/shelves.rs replays. From the repo root: node apps/rune/scripts/shelves-reference.ts
import * as NodeFS from "node:fs";
import * as NodeModule from "node:module";
import * as NodePath from "node:path";
import * as NodeURL from "node:url";

const repoRoot = NodePath.resolve(import.meta.dirname, "../../..");
const out = NodePath.resolve(
  import.meta.dirname,
  "../crates/t3-client/tests/fixtures/shelves.json",
);
const webUrl = NodeURL.pathToFileURL(NodePath.resolve(repoRoot, "apps/web/package.json")).href;
const scriptsUrl = NodeURL.pathToFileURL(import.meta.dirname).href;

// apps/rune has no node_modules; resolve packages as apps/web does.
NodeModule.registerHooks({
  resolve(specifier, context, next) {
    const parent = context.parentURL ?? "";
    if (parent.startsWith(scriptsUrl) && !specifier.startsWith(".") && !specifier.includes(":")) {
      return next(specifier, { ...context, parentURL: webUrl });
    }
    return next(specifier, context);
  },
});

const { presentThreadShell } = await import("@t3tools/client-runtime/state/models");
const { OrchestrationV2ThreadShell } = await import("@t3tools/contracts");
const Schema = await import("effect/Schema");
// As shells arrive off the wire: JSON decoded by the contract.
const decodeShell = Schema.decodeUnknownSync(Schema.toCodecJson(OrchestrationV2ThreadShell));
const { effectiveSnoozed, threadWokeAt } =
  await import("@t3tools/client-runtime/state/thread-settled");
const { sortPinnedThreadsByOrderKey } = await import("@t3tools/client-runtime/state/thread-sort");
// Sidebar.logic.ts imports the web's React modules, which Node can't load. Its
// `hasUnseenCompletion` is pure, so run that function's own source, types stripped.
function webFunction(file: string, name: string) {
  const source = NodeFS.readFileSync(NodePath.resolve(repoRoot, file), "utf8");
  const start = source.indexOf(`export function ${name}(`);
  if (start < 0) throw new Error(`${name} not found in ${file}`);
  let depth = 0;
  let end = source.indexOf("{", start);
  for (; end < source.length; end++) {
    if (source[end] === "{") depth++;
    if (source[end] === "}" && --depth === 0) break;
  }
  const js = NodeModule.stripTypeScriptTypes(
    source.slice(start, end + 1).replace("export function", "function"),
  );
  return new Function(`${js}; return ${name};`)();
}
const resolveSidebarThreadStatus = webFunction(
  "apps/web/src/components/Sidebar.logic.ts",
  "resolveSidebarThreadStatus",
);
const { createThreadNotificationGate } = await import(
  NodeURL.pathToFileURL(
    NodePath.resolve(repoRoot, "apps/web/src/components/ThreadNotificationCoordinator.logic.ts"),
  ).href
);
const hasUnseenCompletion = webFunction(
  "apps/web/src/components/Sidebar.logic.ts",
  "hasUnseenCompletion",
);

const NOW = "2026-11-07T12:00:00.000Z";

// A real captured thread shell, scrubbed; each case overrides the fields the rules read.
function shell(over: Record<string, unknown>) {
  return {
    activeOrderKey: null,
    activeProviderThreadId: null,
    activeRunId: null,
    activityRunStartedAt: null,
    activityRunStatus: null,
    archivedAt: null,
    autoSettleDisabledAt: null,
    blockedOnThreadWait: false,
    branch: null,
    branchPullRequest: null,
    createdAt: "2026-11-01T00:00:00.000Z",
    createdBy: "system",
    creationSource: "server",
    deletedAt: null,
    forkedFrom: null,
    goal: null,
    hasActionableProposedPlan: false,
    id: "t",
    interactionMode: "default",
    itemCount: 1,
    lastError: null,
    lastErrorClass: null,
    lastErrorCode: null,
    lastVisitedAt: null,
    latestRunCompletedAt: null,
    latestRunId: "r1",
    latestRunRequestedAt: null,
    latestRunStartedAt: null,
    latestUserMessageAt: null,
    latestVisibleMessage: null,
    limitRecovery: null,
    lineage: { parentThreadId: null, relationshipToParent: null, rootThreadId: "t" },
    linkedPullRequest: null,
    modelSelection: { instanceId: "codex", model: "m" },
    opens: null,
    pendingBackgroundTasks: [],
    pendingGuestMessageCount: 0,
    pendingRuntimeRequest: null,
    pinOrderKey: null,
    pinnedAt: null,
    projectId: "p",
    providerInstanceHistory: ["codex"],
    providerInstanceId: "codex",
    pullRequests: [],
    runtimeMode: "full-access",
    settledAt: null,
    settledOverride: null,
    snoozedAt: null,
    snoozedUntil: null,
    status: "idle",
    title: "Thread",
    titleRegeneration: null,
    unsettledAt: null,
    updatedAt: "2026-11-07T10:00:00.000Z",
    usageLimitResetAt: null,
    visibleItemCount: 1,
    worktreePath: null,
    ...over,
  };
}

/** A case's shell; `omit` drops a key, as an older server that doesn't send it. */
function build({ omit, ...over }: Record<string, unknown>) {
  const s: Record<string, unknown> = shell(over);
  if (typeof omit === "string") delete s[omit];
  return s;
}

const request = (kind: string, createdAt = "2026-11-07T11:00:00.000Z") => ({
  id: "q",
  kind,
  createdAt,
});
const task = (kind: string) => ({ taskId: `task-${kind}`, kind });
const snoozed = {
  snoozedUntil: "2026-11-07T18:00:00.000Z",
  snoozedAt: "2026-11-07T09:00:00.000Z",
};

const snoozeCases: Record<string, Record<string, unknown>> = {
  "not snoozed": {},
  "snoozed until later": snoozed,
  "wake time passed": { ...snoozed, snoozedUntil: "2026-11-07T11:59:59.000Z" },
  "approval raises its hand": { ...snoozed, pendingRuntimeRequest: request("command") },
  "auth refresh raises its hand": { ...snoozed, pendingRuntimeRequest: request("auth_refresh") },
  "question asked after snoozing": { ...snoozed, pendingRuntimeRequest: request("user_input") },
  "question asked before snoozing": {
    ...snoozed,
    pendingRuntimeRequest: request("user_input", "2026-11-07T08:00:00.000Z"),
  },
  "question with no snoozedAt": {
    snoozedUntil: snoozed.snoozedUntil,
    pendingRuntimeRequest: request("user_input", "2026-11-07T08:00:00.000Z"),
  },
  "guest message": { ...snoozed, pendingGuestMessageCount: 2 },
  "failed after snoozing": {
    ...snoozed,
    status: "failed",
    latestRunCompletedAt: "2026-11-07T10:30:00.000Z",
  },
  "failed before snoozing": {
    ...snoozed,
    status: "failed",
    latestRunCompletedAt: "2026-11-07T08:30:00.000Z",
  },
  "failed with no completion time": { ...snoozed, status: "failed", latestRunCompletedAt: null },
  "failed, completion omitted": { ...snoozed, status: "failed", omit: "latestRunCompletedAt" },
  "failed run, newer activity running": {
    ...snoozed,
    status: "failed",
    activityRunStatus: "running",
    latestRunCompletedAt: "2026-11-07T10:30:00.000Z",
  },
  "failed with background work waiting": {
    ...snoozed,
    status: "failed",
    pendingBackgroundTasks: [task("subagent")],
    latestRunCompletedAt: "2026-11-07T10:30:00.000Z",
  },
  "failed with no run at all": { ...snoozed, status: "failed", latestRunId: null },
  "failed, no run, provider thread": {
    ...snoozed,
    status: "failed",
    latestRunId: null,
    activeProviderThreadId: "pt",
  },
  "unawaited command doesn't park": {
    ...snoozed,
    status: "failed",
    pendingBackgroundTasks: [task("command")],
    latestRunCompletedAt: "2026-11-07T10:30:00.000Z",
  },
};

const unreadCases: Record<string, Record<string, unknown>> = {
  "never visited": { latestRunCompletedAt: "2026-11-07T10:00:00.000Z" },
  "visited before it finished": {
    latestRunCompletedAt: "2026-11-07T10:00:00.000Z",
    lastVisitedAt: "2026-11-07T09:00:00.000Z",
  },
  "visited after it finished": {
    latestRunCompletedAt: "2026-11-07T10:00:00.000Z",
    lastVisitedAt: "2026-11-07T10:05:00.000Z",
  },
  "visited null": { latestRunCompletedAt: "2026-11-07T10:00:00.000Z", lastVisitedAt: null },
  "still running": { status: "running", lastVisitedAt: "2026-11-07T09:00:00.000Z" },
  "idle, completion omitted": {
    lastVisitedAt: "2026-11-07T09:00:00.000Z",
    omit: "latestRunCompletedAt",
  },
  "completed with a completion time": {
    status: "completed",
    latestRunCompletedAt: "2026-11-07T10:00:00.000Z",
    lastVisitedAt: "2026-11-07T09:00:00.000Z",
  },
  "visit omitted (older server)": {
    latestRunCompletedAt: "2026-11-07T10:00:00.000Z",
    omit: "lastVisitedAt",
  },
  "no run": { latestRunId: null, lastVisitedAt: "2026-11-07T09:00:00.000Z" },
};

const statusCases: Record<string, Record<string, unknown>> = {
  idle: {},
  "never ran": { latestRunId: null },
  running: { status: "running" },
  preparing: { status: "preparing" },
  "waiting on another thread": { status: "running", blockedOnThreadWait: true },
  "parked on a subagent": { status: "running", pendingBackgroundTasks: [task("subagent")] },
  approval: { status: "waiting", pendingRuntimeRequest: request("command") },
  question: { status: "waiting", pendingRuntimeRequest: request("user_input") },
  "auth refresh": { status: "running", pendingRuntimeRequest: request("auth_refresh") },
  "guest waiting": { pendingGuestMessageCount: 1 },
  failed: { status: "failed" },
  "usage limit": { status: "failed", lastErrorClass: "usage_limit" },
};

// The coordinator's per-thread observation (ThreadNotificationCoordinator.tsx `observeThreads`).
function observation(s: Record<string, unknown>) {
  const thread = present(s);
  let status = resolveSidebarThreadStatus(thread);
  if (status === "ready" && thread.latestRun?.status === "failed") status = "failed";
  const attention = ["input", "request", "approval", "failed", "limited"].includes(status)
    ? `${thread.latestRun?.runId ?? ""}:${status}`
    : null;
  const completedAt = Date.parse(thread.latestRun?.completedAt ?? "");
  const settled = status === "ready" && thread.latestRun?.status === "completed";
  return {
    attention,
    settled,
    completedAt: settled && Number.isFinite(completedAt) ? completedAt : null,
    archived: thread.archivedAt !== null,
    quiet: s.latestRunStopRequested === true,
  };
}

const done = (at: string, over: Record<string, unknown> = {}) => ({
  status: "completed",
  latestRunCompletedAt: at,
  ...over,
});
const working = { status: "running", latestRunCompletedAt: null };
const gateCases: Record<string, Array<Record<string, unknown>>> = {
  "finishes while watched": [working, done("2026-11-07T11:00:00.000Z")],
  "already finished when first seen": [done("2026-11-07T11:00:00.000Z")],
  "asks for approval": [working, { status: "waiting", pendingRuntimeRequest: request("command") }],
  "the same approval again": [
    working,
    { status: "waiting", pendingRuntimeRequest: request("command") },
    { status: "waiting", pendingRuntimeRequest: request("command"), itemCount: 2 },
  ],
  "asks a question": [working, { status: "waiting", pendingRuntimeRequest: request("user_input") }],
  "stopped by the user": [
    working,
    done("2026-11-07T11:00:00.000Z", { latestRunStopRequested: true }),
  ],
  archived: [
    { ...working, archivedAt: "2026-11-07T10:00:00.000Z" },
    done("2026-11-07T11:00:00.000Z", { archivedAt: "2026-11-07T10:00:00.000Z" }),
  ],
  fails: [working, { status: "failed", latestRunCompletedAt: "2026-11-07T11:00:00.000Z" }],
  "hits a usage limit": [working, { status: "failed", lastErrorClass: "usage_limit" }],
  "auth refresh stays quiet": [
    working,
    { status: "running", pendingRuntimeRequest: request("auth_refresh") },
  ],
  "two runs finish": [
    done("2026-11-07T10:00:00.000Z"),
    { ...working, latestRunId: "r2" },
    done("2026-11-07T11:00:00.000Z", { latestRunId: "r2" }),
    done("2026-11-07T11:00:00.000Z", { latestRunId: "r2", itemCount: 3 }),
  ],
};

function runGate(steps: Array<Record<string, unknown>>) {
  const fired: string[] = [];
  const gate = createThreadNotificationGate((_key: string, kind: string) => fired.push(kind));
  let prior: ReturnType<typeof observation> | undefined;
  for (const over of steps) {
    const next = observation(build(over));
    gate.observe("t", prior, next, { archived: next.archived, quiet: next.quiet });
    prior = next;
  }
  return fired;
}

const pinned = [
  shell({ id: "b", pinnedAt: NOW, pinOrderKey: "a1", createdAt: "2026-11-02T00:00:00.000Z" }),
  shell({ id: "a", pinnedAt: NOW, pinOrderKey: "a1", createdAt: "2026-11-03T00:00:00.000Z" }),
  shell({ id: "c", pinnedAt: NOW, pinOrderKey: "a0", createdAt: "2026-11-01T00:00:00.000Z" }),
  shell({ id: "d", pinnedAt: NOW, createdAt: "2026-11-01T00:00:00.000Z" }),
  shell({ id: "e", pinnedAt: NOW, createdAt: "2026-11-05T00:00:00.000Z" }),
  shell({ id: "f", pinnedAt: NOW, createdAt: "2026-11-05T00:00:00.000Z" }),
];

const present = (s: Record<string, unknown>) => presentThreadShell("env", decodeShell(s));

const fixture = {
  now: NOW,
  snoozed: Object.entries(snoozeCases).map(([name, over]) => {
    const s = build(over);
    return {
      name,
      shell: s,
      snoozed: effectiveSnoozed(present(s), { now: NOW }),
      woke: threadWokeAt(present(s), { now: NOW }),
    };
  }),
  unread: Object.entries(unreadCases).map(([name, over]) => {
    const s = build(over);
    return { name, shell: s, unread: hasUnseenCompletion(present(s)) };
  }),
  status: Object.entries(statusCases).map(([name, over]) => {
    const s = build(over);
    return { name, shell: s, status: resolveSidebarThreadStatus(present(s)) };
  }),
  gate: Object.entries(gateCases).map(([name, steps]) => ({
    name,
    steps: steps.map(build),
    notices: runGate(steps),
  })),
  pinned: {
    shells: pinned,
    order: sortPinnedThreadsByOrderKey(pinned.map(present)).map((t) => t.id),
  },
};

NodeFS.mkdirSync(NodePath.dirname(out), { recursive: true });
NodeFS.writeFileSync(out, `${JSON.stringify(fixture, null, 2)}\n`);
console.log(
  `wrote ${out}: ${fixture.snoozed.length} snooze, ${fixture.unread.length} unread, ${pinned.length} pinned`,
);
