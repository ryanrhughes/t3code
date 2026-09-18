import { expect, it } from "vite-plus/test";
import { gamingBadgeSummary, gamingThreadStatus } from "./gamingOverlay.logic";

const idle = {
  hasPendingApprovals: false,
  hasPendingUserInput: false,
  hasActionableProposedPlan: false,
  latestTurn: null,
  session: null,
};

it("does not present cached approvals as live when an environment is disconnected", () => {
  expect(gamingThreadStatus({ ...idle, hasPendingApprovals: true }, false).label).toBe("Offline");
});

it.each(["hasPendingApprovals", "hasPendingUserInput", "hasActionableProposedPlan"] as const)(
  "prioritizes %s over ongoing background work",
  (field) => {
    expect(
      gamingThreadStatus({ ...idle, [field]: true, backgroundLiveness: "working" }, true),
    ).toMatchObject({ label: "Needs you", rank: 0 });
  },
);

it("keeps monitoring distinct from active work and idle", () => {
  expect(gamingThreadStatus({ ...idle, backgroundLiveness: "monitoring" }, true).label).toBe(
    "Monitoring",
  );
  expect(gamingThreadStatus({ ...idle, backgroundLiveness: "working" }, true).label).toBe(
    "Working",
  );
  expect(gamingThreadStatus(idle, true).label).toBe("Ready");
});

it("prioritizes fresh replies below requests for input, and never reports them online when disconnected", () => {
  expect(gamingThreadStatus(idle, true, true).label).toBe("New reply");
  expect(gamingThreadStatus(idle, false, true).label).toBe("Offline");
  expect(gamingThreadStatus({ ...idle, hasPendingUserInput: true }, true, true).label).toBe(
    "Needs you",
  );
});

it("aggregates alerts across environments without collapsing identical thread IDs", () => {
  const rows = [
    {
      key: "env-a/thread",
      status: gamingThreadStatus(idle, true, true),
      completedAt: "2026-09-16T12:00:00Z",
      turnId: "turn",
    },
    {
      key: "env-b/thread",
      status: gamingThreadStatus({ ...idle, hasPendingUserInput: true }, true),
      completedAt: null,
      turnId: "turn",
    },
    {
      key: "offline/thread",
      status: gamingThreadStatus({ ...idle, hasPendingApprovals: true }, false),
      completedAt: null,
      turnId: "turn",
    },
  ];
  const summary = gamingBadgeSummary(rows, 1);
  expect(summary).toMatchObject({ attention: 1, unread: 1, working: 0, offline: 1 });
  expect(summary.alerts).toHaveLength(2);
  expect(gamingBadgeSummary(rows.toReversed(), 1)).toEqual(summary);
  expect(gamingBadgeSummary(rows.slice(1), 1)).toMatchObject({ attention: 1, unread: 0 });
});
