import type { OrchestrationThreadShell } from "@t3tools/contracts";

type StatusThread = Pick<
  OrchestrationThreadShell,
  | "hasPendingApprovals"
  | "hasPendingUserInput"
  | "hasActionableProposedPlan"
  | "latestTurn"
  | "session"
  | "backgroundLiveness"
>;

/** Connection health takes precedence over cached activity from an unreachable environment. */
export function gamingThreadStatus(thread: StatusThread, connected: boolean, unread = false) {
  if (!connected) return { label: "Offline", tone: "muted", rank: 5 } as const;
  if (
    thread.hasPendingApprovals ||
    thread.hasPendingUserInput ||
    thread.hasActionableProposedPlan
  ) {
    return { label: "Needs you", tone: "attention", rank: 0 } as const;
  }
  if (thread.session?.status === "error" || thread.latestTurn?.state === "error") {
    return { label: "Error", tone: "error", rank: 1 } as const;
  }
  if (
    thread.latestTurn?.state === "running" ||
    thread.session?.status === "starting" ||
    thread.session?.status === "running" ||
    thread.backgroundLiveness === "working"
  ) {
    return { label: "Working", tone: "working", rank: 2 } as const;
  }
  if (thread.backgroundLiveness === "monitoring")
    return { label: "Monitoring", tone: "working", rank: 2 } as const;
  if (unread) return { label: "New reply", tone: "unread", rank: 2 } as const;
  if (thread.latestTurn?.state === "completed")
    return { label: "Finished", tone: "ready", rank: 3 } as const;
  if (thread.latestTurn?.state === "interrupted")
    return { label: "Stopped", tone: "muted", rank: 4 } as const;
  return { label: "Ready", tone: "muted", rank: 4 } as const;
}

export function gamingBadgeSummary(
  rows: ReadonlyArray<{
    key: string;
    status: ReturnType<typeof gamingThreadStatus>;
    completedAt: string | null;
    turnId: string | null;
  }>,
  offline: number,
) {
  let attention = 0,
    unread = 0,
    working = 0;
  const alerts: string[] = [];
  for (const row of rows) {
    if (row.status.tone === "attention" || row.status.tone === "error") {
      attention++;
      alerts.push(`${row.key}:${row.status.label}:${row.turnId ?? ""}`);
    } else if (row.status.tone === "unread") {
      unread++;
      alerts.push(`${row.key}:${row.completedAt ?? ""}`);
    } else if (row.status.tone === "working") working++;
  }
  return { attention, unread, working, offline, alerts: alerts.sort() };
}
