import type {
  DesktopAgentActivityFinishedThread,
  DesktopOpenThreadTarget,
  DesktopTelemetrySetAgentActivity,
} from "@t3tools/contracts";

const MAX_LABEL_LENGTH = 60;
const MAX_FINISHED = 5;

export interface DesktopTrayThread extends DesktopOpenThreadTarget {
  readonly label: string;
}

export type DesktopTrayStatus = "attention" | "working" | "idle";

export interface DesktopTrayModel {
  readonly status: DesktopTrayStatus;
  readonly summary: string;
  readonly needsYou: ReadonlyArray<DesktopTrayThread & { readonly phase: "approval" | "input" }>;
  readonly working: ReadonlyArray<DesktopTrayThread>;
  readonly finished: ReadonlyArray<
    DesktopTrayThread & { readonly outcome: DesktopAgentActivityFinishedThread["outcome"] }
  >;
}

export interface DesktopTrayNotification {
  readonly title: string;
  readonly body: string;
  readonly target: DesktopOpenThreadTarget;
}

/** Electron menus read `&` as a mnemonic marker on Windows and Linux. */
export function formatDesktopTrayLabel(thread: {
  readonly title: string;
  readonly projectTitle: string;
}): string {
  const full = `${thread.title} · ${thread.projectTitle}`;
  const label =
    full.length > MAX_LABEL_LENGTH ? `${full.slice(0, MAX_LABEL_LENGTH - 1).trimEnd()}…` : full;
  return label.replaceAll("&", "&&");
}

/** Merges the activity every attached backend reported into what the tray shows. */
export function buildDesktopTrayModel(
  sources: Iterable<DesktopTelemetrySetAgentActivity>,
): DesktopTrayModel {
  const needsYou: Array<DesktopTrayModel["needsYou"][number]> = [];
  const working: Array<DesktopTrayThread> = [];
  const finished: Array<DesktopTrayModel["finished"][number] & { readonly finishedAt: string }> =
    [];
  for (const source of sources) {
    const { environmentId } = source;
    for (const thread of source.active) {
      const item = {
        environmentId,
        threadId: thread.threadId,
        label: formatDesktopTrayLabel(thread),
      };
      if (thread.phase === "working") working.push(item);
      else needsYou.push({ ...item, phase: thread.phase });
    }
    for (const thread of source.finished) {
      finished.push({
        environmentId,
        threadId: thread.threadId,
        label: formatDesktopTrayLabel(thread),
        outcome: thread.outcome,
        finishedAt: thread.finishedAt,
      });
    }
  }
  finished.sort((left, right) => right.finishedAt.localeCompare(left.finishedAt));
  const parts = [
    ...(needsYou.length > 0
      ? [`${needsYou.length} ${needsYou.length === 1 ? "needs" : "need"} you`]
      : []),
    ...(working.length > 0 ? [`${working.length} working`] : []),
  ];
  return {
    status: needsYou.length > 0 ? "attention" : working.length > 0 ? "working" : "idle",
    summary: parts.length > 0 ? parts.join(" · ") : "No agents running",
    needsYou,
    working,
    finished: finished
      .slice(0, MAX_FINISHED)
      .map(({ finishedAt: _finishedAt, ...thread }) => thread),
  };
}

/**
 * Notifications for what changed since a backend's previous report, with the
 * same titles the renderer uses. A backend's first report is the baseline and
 * notifies nothing.
 */
export function resolveDesktopTrayNotifications(
  previous: ReadonlyMap<string, DesktopTelemetrySetAgentActivity>,
  next: ReadonlyMap<string, DesktopTelemetrySetAgentActivity>,
): ReadonlyArray<DesktopTrayNotification> {
  const notifications: Array<DesktopTrayNotification> = [];
  for (const [sourceId, source] of next) {
    const prior = previous.get(sourceId);
    if (prior === undefined) continue;
    const { environmentId } = source;
    const priorPhases = new Map(prior.active.map((thread) => [thread.threadId, thread.phase]));
    for (const thread of source.active) {
      if (thread.phase === "working" || priorPhases.get(thread.threadId) === thread.phase) continue;
      notifications.push({
        title: thread.phase === "approval" ? "Approval needed" : "Input needed",
        body: thread.title,
        target: { environmentId, threadId: thread.threadId },
      });
    }
    const priorFinished = new Set(
      prior.finished.map((thread) => `${thread.threadId}:${thread.finishedAt}`),
    );
    for (const thread of source.finished) {
      if (priorFinished.has(`${thread.threadId}:${thread.finishedAt}`)) continue;
      notifications.push({
        title: thread.outcome === "failed" ? "Thread failed" : "Thread completed",
        body: thread.title,
        target: { environmentId, threadId: thread.threadId },
      });
    }
  }
  return notifications;
}

/**
 * Paints a status dot into the bottom-right corner of a square BGRA bitmap,
 * clearing a thin ring around it so it stays legible on any panel color.
 */
export function paintDesktopTrayStatusDot(
  pixels: Uint8Array,
  size: number,
  color: { readonly red: number; readonly green: number; readonly blue: number },
): void {
  const radius = size * 0.22;
  const ring = Math.max(1, size * 0.06);
  const center = size - radius - 0.5;
  const clamp = (value: number) => Math.min(1, Math.max(0, value));
  for (let y = 0; y < size; y++) {
    for (let x = 0; x < size; x++) {
      const distance = Math.hypot(x + 0.5 - center, y + 0.5 - center);
      const cutout = clamp(radius + ring + 0.5 - distance);
      if (cutout === 0) continue;
      const offset = (y * size + x) * 4;
      // The ring is at least a pixel wide, so the icon is fully cleared
      // wherever the dot has any coverage.
      const coverage = clamp(radius + 0.5 - distance);
      // Electron bitmaps carry premultiplied alpha.
      if (coverage > 0) {
        pixels[offset] = Math.round(color.blue * coverage);
        pixels[offset + 1] = Math.round(color.green * coverage);
        pixels[offset + 2] = Math.round(color.red * coverage);
        pixels[offset + 3] = Math.round(coverage * 255);
      } else {
        for (let channel = 0; channel < 4; channel++) {
          pixels[offset + channel] = Math.round(pixels[offset + channel]! * (1 - cutout));
        }
      }
    }
  }
}
