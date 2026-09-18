import type { DesktopGamingBadge } from "@t3tools/contracts";
import { scopeThreadRef, scopedThreadKey } from "@t3tools/client-runtime/environment";
import { useUiStateStore } from "../uiStateStore";
import { hasUnseenCompletion } from "./Sidebar.logic";
import { connectionStatusText } from "@t3tools/client-runtime/connection";
import { useNavigate, useParams } from "@tanstack/react-router";
import { Gamepad2Icon, ListIcon, MinusIcon, MonitorIcon, SettingsIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useEnvironments } from "../state/environments";
import { useProjects, useThreadShells } from "../state/entities";
import {
  GamingConversationVisible,
  gamingBrowserPreview,
  gamingOverlayAction,
  useGamingOverlay,
} from "../gamingOverlay";
import { gamingBadgeSummary, gamingThreadStatus } from "./gamingOverlay.logic";
import { Button } from "./ui/button";
import "./gamingOverlay.css";

export function GamingOverlay({ children }: { children: ReactNode }) {
  const navigate = useNavigate();
  const params = useParams({ strict: false });
  const { environments } = useEnvironments();
  const threads = useThreadShells();
  const projects = useProjects();
  const visited = useUiStateStore((state) => state.threadLastVisitedAtById);
  const [enteredAt] = useState(() => new Date().toISOString());
  const [previewHidden, setPreviewHidden] = useState(false);
  const [corner, setCorner] = useState<DesktopGamingBadge["corner"]>(() => {
    try {
      const saved = localStorage.getItem("t3code:gaming-badge-corner");
      if (
        saved === "top-left" ||
        saved === "top-right" ||
        saved === "bottom-left" ||
        saved === "bottom-right"
      )
        return saved;
    } catch {
      /* Storage is optional. */
    }
    return "top-right";
  });
  const badgeError = useGamingOverlay((state) => state.badgeError);
  const shortcut = useGamingOverlay((state) => state.shortcutLabel);
  const [rosterOpen, setRosterOpen] = useState(!params.threadId);
  const [query, setQuery] = useState("");
  const [environmentFilter, setEnvironmentFilter] = useState("");
  const [needsYouOnly, setNeedsYouOnly] = useState(false);
  const roster = useMemo(() => {
    const byEnvironment = new Map(
      environments.map((environment) => [environment.environmentId, environment]),
    );
    const byProject = new Map(
      projects.map((project) => [`${project.environmentId}:${project.id}`, project.title]),
    );
    return threads
      .filter((thread) => thread.archivedAt === null)
      .map((thread) => {
        const environment = byEnvironment.get(thread.environmentId);
        const project = byProject.get(`${thread.environmentId}:${thread.projectId}`) ?? "Project";
        const key = scopedThreadKey(scopeThreadRef(thread.environmentId, thread.id));
        const unread = hasUnseenCompletion({ ...thread, lastVisitedAt: visited[key] ?? enteredAt });
        const status = gamingThreadStatus(
          thread,
          environment?.connection.phase === "connected",
          unread,
        );
        return {
          key,
          thread,
          environment,
          project,
          status,
          completedAt: thread.latestTurn?.completedAt ?? null,
          turnId: thread.latestTurn?.turnId ?? null,
        };
      })
      .sort(
        (a, b) =>
          a.status.rank - b.status.rank || b.thread.updatedAt.localeCompare(a.thread.updatedAt),
      );
  }, [environments, projects, threads, visited, enteredAt]);
  const attentionCount = roster.filter(
    (row) => row.status.tone === "attention" || row.status.tone === "error",
  ).length;
  const filtered = roster.filter(
    (row) =>
      (!environmentFilter || row.thread.environmentId === environmentFilter) &&
      (!needsYouOnly || row.status.tone === "attention" || row.status.tone === "error") &&
      `${row.thread.title} ${row.project} ${row.environment?.label ?? ""}`
        .toLowerCase()
        .includes(query.toLowerCase()),
  );
  const selected = roster.find(
    (row) => row.thread.id === params.threadId && row.thread.environmentId === params.environmentId,
  );

  const badge = gamingBadgeSummary(
    roster,
    environments.filter((env) => env.connection.phase !== "connected").length,
  );
  const alertsKey = JSON.stringify(badge.alerts);
  const previousAlerts = useRef(new Set<string>());
  const [pulseKey, setPulseKey] = useState(0);
  useEffect(() => {
    const alerts: string[] = JSON.parse(alertsKey);
    const pulse =
      alerts.some((alert) => !previousAlerts.current.has(alert)) &&
      !window.matchMedia("(prefers-reduced-motion: reduce)").matches;
    previousAlerts.current = new Set(alerts);
    if (pulse) setPulseKey((key) => key + 1);
    void window.desktopBridge
      ?.setGamingBadge?.({
        attention: badge.attention,
        unread: badge.unread,
        working: badge.working,
        offline: badge.offline,
        corner,
        pulse,
      })
      .catch(() => undefined);
  }, [badge.attention, badge.unread, badge.working, badge.offline, alertsKey, corner]);
  const openAgents = useCallback(() => {
    setPreviewHidden(false);
    setQuery("");
    setEnvironmentFilter("");
    setNeedsYouOnly(false);
    setRosterOpen(true);
  }, []);
  useEffect(() => window.desktopBridge?.onGamingOverlayActivate?.(openAgents), [openAgents]);
  const hide = () => {
    if (gamingBrowserPreview) setPreviewHidden(true);
    else void gamingOverlayAction("hide");
  };
  const badgeLabel = `${badge.attention} need you · ${badge.unread} unread · ${badge.working} working · ${badge.offline} offline`;
  return (
    <>
      {gamingBrowserPreview && (
        <button
          key={pulseKey}
          className="gaming-preview-badge"
          data-corner={corner}
          data-tone={
            badge.attention
              ? "attention"
              : badge.unread
                ? "unread"
                : badge.working
                  ? "working"
                  : "idle"
          }
          data-pulse={pulseKey > 0 || undefined}
          aria-label={`Open T3 agents: ${badgeLabel}`}
          onClick={openAgents}
        >
          <strong>T3</strong>
          <small>{badge.attention + badge.unread || "•"}</small>
        </button>
      )}
      <section
        className="gaming-overlay"
        style={previewHidden ? { visibility: "hidden" } : undefined}
        aria-label="T3 gaming overlay"
        data-browser-preview={
          (import.meta.env.DEV && !window.desktopBridge?.gamingOverlay) || undefined
        }
        onKeyDown={(event) => {
          if (event.key !== "Escape" || event.defaultPrevented || event.nativeEvent.isComposing)
            return;
          if (
            event.target instanceof Element &&
            event.target.closest('[role="dialog"], [role="menu"], [role="listbox"]')
          )
            return;
          event.preventDefault();
          if (rosterOpen && params.threadId) setRosterOpen(false);
          else hide();
        }}
      >
        <header className="gaming-overlay-titlebar">
          <Gamepad2Icon size={17} aria-hidden />
          <span>
            T3 <span className="gaming-overlay-subtitle">Agent whispers</span>
          </span>
        </header>
        <nav className="gaming-overlay-toolbar" aria-label="Gaming controls">
          <Button
            size="sm"
            variant="ghost"
            onClick={() => setRosterOpen(!rosterOpen)}
            aria-expanded={rosterOpen}
          >
            <ListIcon size={15} /> Agents{attentionCount > 0 ? ` · ${attentionCount} need you` : ""}
          </Button>
          <div className="flex-1" />
          <Button
            size="icon-sm"
            variant="ghost"
            title="Exit gaming mode"
            aria-label="Exit gaming mode"
            onClick={() => void gamingOverlayAction("exit")}
          >
            <MonitorIcon />
          </Button>
          {(window.desktopBridge?.gamingOverlay || gamingBrowserPreview) && (
            <Button
              size="icon-sm"
              variant="ghost"
              title={`Back to game · ${shortcut ?? "Hide"}`}
              aria-label="Hide gaming overlay"
              onClick={hide}
            >
              <MinusIcon />
            </Button>
          )}
        </nav>
        {badgeError && (
          <p className="gaming-overlay-error" role="alert">
            {badgeError}
          </p>
        )}
        {!rosterOpen && selected && (
          <div className="gaming-overlay-chat-title">{selected.thread.title}</div>
        )}
        <div className="gaming-overlay-content">
          <div className="gaming-overlay-conversation" inert={rosterOpen}>
            <GamingConversationVisible value={!rosterOpen && !previewHidden}>
              {children}
            </GamingConversationVisible>
          </div>
          {rosterOpen && (
            <div className="gaming-overlay-roster">
              <div className="gaming-overlay-filters">
                <input
                  aria-label="Find an agent thread"
                  placeholder="Find a thread, project, or environment…"
                  value={query}
                  onChange={(event) => setQuery(event.target.value)}
                />
                <div className="flex items-center gap-3">
                  <select
                    aria-label="Environment"
                    value={environmentFilter}
                    onChange={(event) => setEnvironmentFilter(event.target.value)}
                  >
                    <option value="">All environments</option>
                    {environments.map((environment) => (
                      <option key={environment.environmentId} value={environment.environmentId}>
                        {environment.label} · {connectionStatusText(environment.connection)}
                      </option>
                    ))}
                  </select>
                  <label className="flex shrink-0 items-center gap-1 text-xs">
                    <input
                      type="checkbox"
                      checked={needsYouOnly}
                      onChange={(event) => setNeedsYouOnly(event.target.checked)}
                    />{" "}
                    Needs you
                  </label>
                </div>
              </div>
              <div className="gaming-overlay-threads">
                {filtered.length === 0 && (
                  <p className="p-6 text-center text-sm text-muted-foreground">
                    {roster.length
                      ? "No matching threads."
                      : "Your active threads will appear here. Add environments in Connections."}
                  </p>
                )}
                {filtered.slice(0, 100).map(({ thread, environment, project, status }) => (
                  <button
                    key={`${thread.environmentId}:${thread.id}`}
                    className="gaming-overlay-thread"
                    data-selected={selected?.thread === thread}
                    onClick={() => {
                      void navigate({
                        to: "/$environmentId/$threadId",
                        params: { environmentId: thread.environmentId, threadId: thread.id },
                      }).then(() => setRosterOpen(false));
                    }}
                  >
                    <span className="gaming-overlay-thread-title">{thread.title}</span>
                    <span className="gaming-overlay-status" data-tone={status.tone}>
                      {status.label}
                    </span>
                    <span className="gaming-overlay-thread-meta">
                      {environment?.label ?? "Unavailable environment"} · {project}
                    </span>
                  </button>
                ))}
                {filtered.length > 100 && (
                  <p className="p-3 text-xs text-muted-foreground">
                    Showing 100 of {filtered.length} threads. Search to narrow the list.
                  </p>
                )}
              </div>
              <div className="gaming-overlay-badge-settings">
                <label>
                  Badge position{" "}
                  <select
                    aria-label="Badge position"
                    value={corner}
                    onChange={(event) => {
                      const value = event.target.value as DesktopGamingBadge["corner"];
                      setCorner(value);
                      try {
                        localStorage.setItem("t3code:gaming-badge-corner", value);
                      } catch {
                        /* Storage is optional. */
                      }
                    }}
                  >
                    {(["top-right", "top-left", "bottom-right", "bottom-left"] as const).map(
                      (value) => (
                        <option key={value} value={value}>
                          {value.replace("-", " ")}
                        </option>
                      ),
                    )}
                  </select>
                </label>
                <p>
                  <span data-tone="attention">Gold: needs you</span> ·{" "}
                  <span data-tone="unread">Green: new reply</span> · Blue: working
                </p>
              </div>
              <Button
                size="sm"
                variant="ghost"
                onClick={() => {
                  void gamingOverlayAction("exit").then((exited) => {
                    if (exited) return navigate({ to: "/settings/connections" });
                  });
                }}
              >
                <SettingsIcon size={14} /> Manage environments
              </Button>
            </div>
          )}
        </div>
        <footer className="gaming-overlay-footer">
          <span className="truncate">
            {selected
              ? `${selected.environment?.label ?? "Offline"} · ${selected.status.label}`
              : "Your agents, alongside your adventure"}
          </span>
          <kbd>{shortcut}</kbd>
        </footer>
      </section>
    </>
  );
}
