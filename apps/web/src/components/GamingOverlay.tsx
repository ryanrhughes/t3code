import {
  resolveEnvironmentMachineKind,
  type DesktopGamingBadge,
  type EnvironmentId,
} from "@t3tools/contracts";
import { scopeThreadRef, scopedThreadKey } from "@t3tools/client-runtime/environment";
import { useUiStateStore } from "../uiStateStore";
import { hasUnseenCompletion } from "./Sidebar.logic";
import { useNavigate, useParams } from "@tanstack/react-router";
import { ChevronLeftIcon, MinusIcon, MonitorIcon, SearchIcon, XIcon } from "lucide-react";
import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from "react";
import { useEnvironments } from "../state/environments";
import { useProjects, useThreadShells } from "../state/entities";
import {
  GamingConversationVisible,
  gamingBrowserPreview,
  gamingOverlayAction,
  useGamingOverlay,
} from "../gamingOverlay";
import { EnvironmentMachineIcon } from "./EnvironmentMachineIcon";
import { gamingBadgeSummary, gamingThreadStatus } from "./gamingOverlay.logic";
import { Kbd } from "./ui/kbd";
import { Tooltip, TooltipPopup, TooltipTrigger } from "./ui/tooltip";
import "./gamingOverlay.css";

const CORNER_KEY = "t3code:gaming-badge-corner";
const CORNERS = ["top-right", "top-left", "bottom-right", "bottom-left"] as const;
type Corner = DesktopGamingBadge["corner"];

function readCorner(): Corner {
  try {
    const saved = localStorage.getItem(CORNER_KEY);
    if (CORNERS.includes(saved as Corner)) return saved as Corner;
  } catch {
    /* Storage is optional. */
  }
  return "top-right";
}

function relativeTime(iso: string, now: number) {
  const delta = Math.max(0, now - Date.parse(iso));
  const minutes = Math.floor(delta / 60_000);
  if (minutes < 1) return "now";
  if (minutes < 60) return `${minutes}m`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `${hours}h`;
  return `${Math.floor(hours / 24)}d`;
}

/**
 * Gaming mode is a view of the same renderer: the roster reads the environments,
 * threads and unread markers the normal window already has, and the selected
 * chat is the ordinary ChatView rendered inside the panel.
 */
export function GamingOverlay({ children }: { children: ReactNode }) {
  const navigate = useNavigate();
  const params = useParams({ strict: false });
  const { environments } = useEnvironments();
  const threads = useThreadShells();
  const projects = useProjects();
  const visited = useUiStateStore((state) => state.threadLastVisitedAtById);
  const [enteredAt] = useState(() => new Date().toISOString());
  const [previewHidden, setPreviewHidden] = useState(false);
  const [corner, setCorner] = useState<Corner>(readCorner);
  const badgeError = useGamingOverlay((state) => state.badgeError);
  const shortcut = useGamingOverlay((state) => state.shortcutLabel);
  const [rosterOpen, setRosterOpen] = useState(!params.threadId);
  const [query, setQuery] = useState("");
  const [environmentFilter, setEnvironmentFilter] = useState<EnvironmentId | "">("");
  const [needsYouOnly, setNeedsYouOnly] = useState(false);
  // Relative times only need a coarse clock; it ticks while the roster is open.
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!rosterOpen) return;
    const timer = window.setInterval(() => setNow(Date.now()), 30_000);
    return () => window.clearInterval(timer);
  }, [rosterOpen]);
  const openAgents = useCallback(() => {
    setNow(Date.now());
    setPreviewHidden(false);
    setQuery("");
    setEnvironmentFilter("");
    setNeedsYouOnly(false);
    setRosterOpen(true);
  }, []);

  const machineById = useMemo(
    () =>
      new Map(
        environments.map((environment) => [
          environment.environmentId,
          resolveEnvironmentMachineKind(environment.serverConfig),
        ]),
      ),
    [environments],
  );
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

  const offline = environments.filter((env) => env.connection.phase !== "connected");
  const badge = gamingBadgeSummary(roster, offline.length);
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
  useEffect(() => window.desktopBridge?.onGamingOverlayActivate?.(openAgents), [openAgents]);
  const hide = () => {
    if (gamingBrowserPreview) setPreviewHidden(true);
    else void gamingOverlayAction("hide");
  };
  const chooseCorner = (value: Corner) => {
    setCorner(value);
    try {
      localStorage.setItem(CORNER_KEY, value);
    } catch {
      /* Storage is optional. */
    }
  };
  const badgeLabel = `${badge.attention} need you · ${badge.unread} unread · ${badge.working} working · ${badge.offline} offline`;
  const badgeTone = badge.attention
    ? "attention"
    : badge.unread
      ? "unread"
      : badge.working
        ? "working"
        : "idle";
  return (
    <>
      {gamingBrowserPreview && (
        <button
          key={pulseKey}
          className="gaming-preview-badge"
          data-corner={corner}
          data-tone={badgeTone}
          data-pulse={pulseKey > 0 || undefined}
          aria-label={`Open T3 agents: ${badgeLabel}`}
          onClick={openAgents}
        >
          <span className="gaming-preview-badge-mark">T3</span>
          {badge.attention + badge.unread > 0 && (
            <span className="gaming-preview-badge-count">
              {Math.min(99, badge.attention + badge.unread)}
            </span>
          )}
        </button>
      )}
      <section
        className="gaming-overlay dark"
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
          {!rosterOpen && selected ? (
            <button
              type="button"
              className="gaming-overlay-back"
              onClick={() => {
                setNow(Date.now());
                setRosterOpen(true);
              }}
              aria-label="Back to agents"
            >
              <ChevronLeftIcon size={16} aria-hidden />
              <span className="gaming-overlay-title">{selected.thread.title}</span>
            </button>
          ) : (
            <div className="gaming-overlay-heading">
              <span className="gaming-overlay-wordmark" aria-hidden>
                T3
              </span>
              <span className="gaming-overlay-title">Agents</span>
              {attentionCount > 0 && (
                <span className="gaming-overlay-pill" data-tone="attention">
                  {attentionCount} need you
                </span>
              )}
            </div>
          )}
          <div className="gaming-overlay-actions">
            <Tooltip>
              <TooltipTrigger
                render={
                  <button
                    type="button"
                    className="gaming-overlay-icon-button"
                    aria-label="Hide chat"
                    onClick={hide}
                  />
                }
              >
                <MinusIcon size={15} aria-hidden />
              </TooltipTrigger>
              <TooltipPopup side="bottom">Hide · {shortcut ?? "Esc"}</TooltipPopup>
            </Tooltip>
            <Tooltip>
              <TooltipTrigger
                render={
                  <button
                    type="button"
                    className="gaming-overlay-icon-button"
                    aria-label="Hide chat"
                    onClick={hide}
                  />
                }
              >
                <XIcon size={15} aria-hidden />
              </TooltipTrigger>
              <TooltipPopup side="bottom">Hide chat. The badge stays.</TooltipPopup>
            </Tooltip>
          </div>
        </header>
        {badgeError && (
          <p className="gaming-overlay-error" role="alert">
            {badgeError}
          </p>
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
                <label className="gaming-overlay-search">
                  <SearchIcon size={14} aria-hidden />
                  <input
                    aria-label="Find an agent thread"
                    placeholder="Search threads"
                    value={query}
                    onChange={(event) => setQuery(event.target.value)}
                  />
                </label>
                <div className="gaming-overlay-chips" role="group" aria-label="Filter">
                  <button
                    type="button"
                    className="gaming-overlay-chip"
                    data-active={!environmentFilter && !needsYouOnly}
                    onClick={() => {
                      setEnvironmentFilter("");
                      setNeedsYouOnly(false);
                    }}
                  >
                    All
                  </button>
                  <button
                    type="button"
                    className="gaming-overlay-chip"
                    data-active={needsYouOnly}
                    data-tone="attention"
                    onClick={() => setNeedsYouOnly((value) => !value)}
                  >
                    Needs you{attentionCount > 0 ? ` · ${attentionCount}` : ""}
                  </button>
                  {environments.length > 1 &&
                    environments.map((environment) => (
                      <button
                        key={environment.environmentId}
                        type="button"
                        className="gaming-overlay-chip"
                        data-active={environmentFilter === environment.environmentId}
                        data-offline={environment.connection.phase !== "connected" || undefined}
                        aria-label={`${environment.label} (${environment.connection.phase})`}
                        onClick={() =>
                          setEnvironmentFilter((current) =>
                            current === environment.environmentId ? "" : environment.environmentId,
                          )
                        }
                      >
                        <EnvironmentMachineIcon
                          aria-hidden
                          kind={machineById.get(environment.environmentId) ?? "server"}
                          className="size-3"
                        />
                        {environment.label}
                      </button>
                    ))}
                </div>
              </div>
              <div className="gaming-overlay-threads">
                {filtered.length === 0 && (
                  <p className="gaming-overlay-empty">
                    {roster.length
                      ? "No matching threads."
                      : "Threads from every connected environment appear here."}
                  </p>
                )}
                {filtered.slice(0, 100).map(({ thread, environment, project, status }) => (
                  <button
                    key={`${thread.environmentId}:${thread.id}`}
                    type="button"
                    className="gaming-overlay-thread"
                    data-selected={selected?.thread === thread || undefined}
                    data-tone={status.tone}
                    onClick={() => {
                      void navigate({
                        to: "/$environmentId/$threadId",
                        params: { environmentId: thread.environmentId, threadId: thread.id },
                      }).then(() => setRosterOpen(false));
                    }}
                  >
                    <span className="gaming-overlay-thread-dot" aria-hidden />
                    <span className="gaming-overlay-thread-title">{thread.title}</span>
                    <span className="gaming-overlay-thread-time">
                      {relativeTime(thread.updatedAt, now)}
                    </span>
                    <span className="gaming-overlay-thread-meta">
                      <span className="gaming-overlay-status">{status.label}</span>
                      <span className="gaming-overlay-thread-sep" aria-hidden>
                        ·
                      </span>
                      <span className="truncate">
                        {environment?.label ?? "Unavailable"} / {project}
                      </span>
                    </span>
                  </button>
                ))}
                {filtered.length > 100 && (
                  <p className="gaming-overlay-empty">
                    Showing 100 of {filtered.length} threads. Search to narrow the list.
                  </p>
                )}
              </div>
              <footer className="gaming-overlay-footer">
                <span className="gaming-overlay-footer-item">
                  <span className="gaming-overlay-legend" data-tone="attention" /> needs you
                  <span className="gaming-overlay-legend" data-tone="unread" /> new reply
                  <span className="gaming-overlay-legend" data-tone="working" /> working
                </span>
                <label className="gaming-overlay-corner">
                  Badge
                  <select
                    aria-label="Badge position"
                    value={corner}
                    onChange={(event) => chooseCorner(event.target.value as Corner)}
                  >
                    {CORNERS.map((value) => (
                      <option key={value} value={value}>
                        {value.replace("-", " ")}
                      </option>
                    ))}
                  </select>
                </label>
                {shortcut && <Kbd>{shortcut}</Kbd>}
                <Tooltip>
                  <TooltipTrigger
                    render={
                      <button
                        type="button"
                        className="gaming-overlay-exit"
                        onClick={() => void gamingOverlayAction("exit")}
                      />
                    }
                  >
                    <MonitorIcon size={13} aria-hidden /> Exit
                  </TooltipTrigger>
                  <TooltipPopup side="top">Return to the full T3 window</TooltipPopup>
                </Tooltip>
              </footer>
            </div>
          )}
        </div>
      </section>
    </>
  );
}
