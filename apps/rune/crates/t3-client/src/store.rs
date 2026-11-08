//! Client-side reducers for the shell and thread streams. They mirror
//! `packages/client-runtime/src/state/shellReducer.ts` and `orchestrationV2Projection.ts`:
//! snapshots replace, deltas at or below the cursor are dropped, entities upsert by id.
use crate::protocol::*;

#[derive(Clone, Debug, Default)]
pub struct ShellState {
    pub projects: Vec<ProjectShell>,
    /// Active (non-archived) threads only.
    pub threads: Vec<ThreadShell>,
    pub sequence: u64,
    pub loaded: bool,
    pub synchronized: bool,
    /// Settled threads the snapshot left out (`omitOlderSettled`), and the pages read so far.
    pub older: Option<OlderSettled>,
}

/// Older settled threads below the snapshot's cut, read a page at a time with
/// `orchestration.getOlderSettledThreads` (`olderSettledThreads.ts` on the web).
#[derive(Clone, Debug, Default)]
pub struct OlderSettled {
    /// The cut: settled before this and not kept for another reason.
    pub before: String,
    /// How many the server left out, across projects.
    pub total: u64,
    /// Pages read so far, newest settle first.
    pub loaded: Vec<ThreadShell>,
    pub exhausted: bool,
    pub loading: bool,
}

/// Older settled pages hold at most this many.
pub const OLDER_SETTLED_PAGE: usize = 50;

impl ShellState {
    pub fn is_below_cut(&self, t: &ThreadShell) -> bool {
        let Some(o) = &self.older else { return false };
        let cut = crate::shelves::iso_ms(&o.before);
        t.is_settled()
            && t.settled_at
                .as_deref()
                .and_then(crate::shelves::iso_ms)
                .zip(cut)
                .is_some_and(|(at, cut)| at < cut)
    }

    /// The Settled count, as the web's: local settled rows above the cut, plus the server's
    /// count of the ones it left out.
    pub fn settled_count(&self) -> u64 {
        let local = self
            .settled_threads()
            .iter()
            .filter(|t| !self.is_below_cut(t))
            .count() as u64;
        local + self.older.as_ref().map_or(0, |o| o.total)
    }

    /// Loaded older rows the live shell doesn't hold itself.
    pub fn older_rows(&self) -> Vec<&ThreadShell> {
        let Some(o) = &self.older else {
            return Vec::new();
        };
        o.loaded
            .iter()
            .filter(|t| !self.threads.iter().any(|l| l.id == t.id))
            .collect()
    }

    /// Where the next page starts: the cut, then just after the last row read.
    pub fn older_cursor(&self) -> Option<serde_json::Value> {
        let o = self.older.as_ref().filter(|o| !o.exhausted)?;
        Some(match o.loaded.last() {
            None => serde_json::json!({ "settledAt": o.before }),
            Some(t) => serde_json::json!({
                "settledAt": t.settled_at.clone().unwrap_or_else(|| o.before.clone()),
                "threadId": t.id,
            }),
        })
    }

    /// While more pages remain, rows settled before this aren't known yet; a merged list shows
    /// other machines' older rows only above it.
    pub fn older_frontier(&self) -> Option<i64> {
        let o = self.older.as_ref().filter(|o| !o.exhausted)?;
        let at = o
            .loaded
            .last()
            .and_then(|t| t.settled_at.clone())
            .unwrap_or_else(|| o.before.clone());
        crate::shelves::iso_ms(&at)
    }

    pub fn add_older_page(&mut self, before: &str, threads: Vec<ThreadShell>) {
        let Some(o) = self.older.as_mut().filter(|o| o.before == before) else {
            return;
        };
        o.loading = false;
        o.exhausted = threads.len() < OLDER_SETTLED_PAGE;
        for t in threads {
            if !o.loaded.iter().any(|l| l.id == t.id) {
                o.loaded.push(t);
            }
        }
    }

    /// Returns true when visible state changed.
    pub fn apply(&mut self, item: ShellStreamItem) -> bool {
        match item {
            ShellStreamItem::Synchronized => {
                self.synchronized = true;
                false
            }
            // A snapshot with `resolvedRepositoryIdentityRoots` is an enrichment, not a reset: it
            // carries no threads and only refreshes projects' repository identity
            // (`mergeShellSnapshotProjects`), which this client doesn't show. Keep everything.
            ShellStreamItem::Snapshot {
                resolved_repository_identity_roots: Some(_),
                ..
            } => false,
            ShellStreamItem::Snapshot {
                snapshot,
                resolved_repository_identity_roots: None,
            } => {
                self.sequence = snapshot.snapshot_sequence;
                self.projects = snapshot.projects;
                self.threads = snapshot.threads;
                self.older = snapshot.older_settled.map(|o| {
                    let total = o.counts.iter().map(|c| c.count).sum();
                    OlderSettled {
                        before: o.before,
                        total,
                        exhausted: total == 0,
                        ..Default::default()
                    }
                });
                self.loaded = true;
                true
            }
            ShellStreamItem::ProjectUpdated { sequence, project } => {
                if !self.advance(sequence) {
                    return false;
                }
                upsert(&mut self.projects, project, |p| &p.id);
                true
            }
            ShellStreamItem::ProjectRemoved {
                sequence,
                project_id,
            } => {
                if !self.advance(sequence) {
                    return false;
                }
                self.projects.retain(|p| p.id != project_id);
                true
            }
            ShellStreamItem::ThreadUpdated {
                sequence,
                location,
                thread,
            } => {
                if !self.advance(sequence) {
                    return false;
                }
                if location == "archive" {
                    self.threads.retain(|t| t.id != thread.id);
                } else {
                    upsert(&mut self.threads, thread, |t| &t.id);
                }
                true
            }
            ShellStreamItem::ThreadRemoved {
                sequence,
                thread_id,
                ..
            } => {
                if !self.advance(sequence) {
                    return false;
                }
                self.threads.retain(|t| t.id != thread_id);
                true
            }
            ShellStreamItem::Unknown => false,
        }
    }

    fn advance(&mut self, sequence: u64) -> bool {
        if sequence <= self.sequence {
            return false;
        }
        self.sequence = sequence;
        true
    }

    pub fn thread(&self, id: &str) -> Option<&ThreadShell> {
        self.threads.iter().find(|t| t.id == id)
    }

    pub fn project(&self, id: &str) -> Option<&ProjectShell> {
        self.projects.iter().find(|p| p.id == id)
    }

    /// Sidebar rows for a project, newest activity first.
    /// A project's threads in the sidebar, newest first; settled ones sit on the Settled shelf.
    pub fn project_threads(&self, project_id: &str) -> Vec<&ThreadShell> {
        let mut rows: Vec<_> = self
            .threads
            .iter()
            .filter(|t| t.project_id == project_id && t.is_sidebar_row() && !t.is_settled())
            .collect();
        rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        rows
    }

    /// The Settled shelf, as the web's `sortSettledThreads`: most recently settled first, ties
    /// by id. A thread without `settledAt` sorts by its last update.
    pub fn settled_threads(&self) -> Vec<&ThreadShell> {
        let mut rows: Vec<_> = self
            .threads
            .iter()
            .filter(|t| t.is_sidebar_row() && t.is_settled())
            .collect();
        let at = |t: &ThreadShell| t.settled_at.clone().unwrap_or_else(|| t.updated_at.clone());
        rows.sort_by(|a, b| at(b).cmp(&at(a)).then_with(|| a.id.cmp(&b.id)));
        rows
    }

    /// Projects in sidebar order: pinned first, then most recently active.
    pub fn sidebar_projects(&self) -> Vec<&ProjectShell> {
        let latest = |p: &ProjectShell| {
            self.threads
                .iter()
                .filter(|t| t.project_id == p.id)
                .map(|t| t.updated_at.as_str())
                .max()
                .unwrap_or(p.updated_at.as_str())
                .to_owned()
        };
        let mut rows: Vec<_> = self
            .projects
            .iter()
            .filter(|p| p.archived_at.is_none())
            .map(|p| (p.pinned_at.is_some(), latest(p), p))
            .collect();
        rows.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        rows.into_iter().map(|(_, _, p)| p).collect()
    }
}

#[derive(Clone, Debug, Default)]
pub struct ThreadState {
    pub thread: ThreadShell,
    pub runs: Vec<Run>,
    pub provider_turns: Vec<ProviderTurn>,
    pub checkpoints: Vec<Checkpoint>,
    pub runtime_requests: Vec<RuntimeRequest>,
    /// Local turn items, by id.
    pub turn_items: Vec<TurnItem>,
    /// Conversation messages, by id; queued ones have no turn item yet.
    pub messages: Vec<crate::protocol::Message>,
    /// What the timeline renders, in order. Inherited rows (fork history) keep their snapshot
    /// order ahead of local rows; local rows sort by `(ordinal, id)`.
    pub visible: Vec<ProjectedTurnItem>,
    pub sequence: u64,
    pub synchronized: bool,
    pub deleted: bool,
}

impl ThreadState {
    pub fn from_snapshot(sequence: u64, p: ThreadProjection) -> ThreadState {
        ThreadState {
            thread: p.thread,
            runs: p.runs,
            provider_turns: p.provider_turns,
            checkpoints: p.checkpoints,
            runtime_requests: p.runtime_requests,
            turn_items: p.turn_items,
            visible: p.visible_turn_items,
            messages: p.messages,
            sequence,
            synchronized: false,
            deleted: false,
        }
    }

    /// Applies one event; returns true when visible state changed.
    pub fn apply(&mut self, sequence: u64, thread_id: &str, event: ThreadEvent) -> bool {
        if sequence <= self.sequence || thread_id != self.thread.id {
            return false;
        }
        // Unknown events still advance the resume cursor.
        self.sequence = sequence;
        match event {
            ThreadEvent::Thread(t) => self.thread = t,
            ThreadEvent::Deleted => self.deleted = true,
            ThreadEvent::Run(run) => {
                upsert(&mut self.runs, run, |r| &r.id);
                self.refresh_visibility();
            }
            ThreadEvent::RuntimeRequest(req) => upsert(&mut self.runtime_requests, req, |r| &r.id),
            ThreadEvent::Message(m) => upsert(&mut self.messages, m, |m| &m.id),
            ThreadEvent::ProviderTurn(turn) => {
                upsert_rev(&mut self.provider_turns, turn, |t| &t.id)
            }
            ThreadEvent::Checkpoint(c) => upsert_rev(&mut self.checkpoints, c, |c| &c.id),
            ThreadEvent::TurnItem(item) => {
                let interrupt = item.kind == "run_interrupt_request";
                let visible = self.is_visible(&item);
                upsert_rev(&mut self.turn_items, item.clone(), |i| &i.id);
                if interrupt {
                    self.refresh_visibility();
                } else if visible {
                    self.upsert_visible(item);
                } else {
                    self.visible.retain(|r| r.source_item_id != item.id);
                }
            }
            ThreadEvent::Other => return false,
        }
        true
    }

    /// `isOrchestrationV2TurnItemVisible` without the superseded-interrupt pairing, which only
    /// hides a legacy steer marker.
    fn is_visible(&self, item: &TurnItem) -> bool {
        let run = item
            .run_id
            .as_deref()
            .and_then(|id| self.runs.iter().find(|r| r.id == id));
        match run {
            Some(r) if r.status == "rolled_back" => false,
            Some(r)
                if r.status == "cancelled"
                    && item.kind == "user_message"
                    && item.input_intent.as_deref() == Some("queued_turn") =>
            {
                false
            }
            _ => true,
        }
    }

    fn upsert_visible(&mut self, item: TurnItem) {
        // Streamed updates land on the newest rows; search from the end.
        if let Some(i) = self
            .visible
            .iter()
            .rposition(|r| r.source_item_id == item.id && r.visibility == "local")
        {
            if self.visible[i].item.ordinal == item.ordinal {
                self.visible[i].item = item;
                return;
            }
            self.visible.remove(i);
        }
        let key = (item.ordinal, item.id.clone());
        let at = self
            .visible
            .iter()
            .position(|r| {
                r.visibility == "local"
                    && (r.item.ordinal, r.item.id.as_str()) > (key.0, key.1.as_str())
            })
            .unwrap_or(self.visible.len());
        self.visible.insert(
            at,
            ProjectedTurnItem {
                position: 0,
                visibility: "local".into(),
                source_thread_id: item.thread_id.clone(),
                source_item_id: item.id.clone(),
                item,
            },
        );
        for (i, row) in self.visible.iter_mut().enumerate() {
            row.position = i as u64;
        }
    }

    fn refresh_visibility(&mut self) {
        let inherited: Vec<_> = self
            .visible
            .iter()
            .filter(|r| r.visibility != "local")
            .cloned()
            .collect();
        let mut local: Vec<_> = self
            .turn_items
            .iter()
            .filter(|i| self.is_visible(i))
            .cloned()
            .collect();
        local.sort_by(|a, b| (a.ordinal, &a.id).cmp(&(b.ordinal, &b.id)));
        self.visible = inherited;
        self.visible
            .extend(local.into_iter().map(|item| ProjectedTurnItem {
                position: 0,
                visibility: "local".into(),
                source_thread_id: item.thread_id.clone(),
                source_item_id: item.id.clone(),
                item,
            }));
        for (i, row) in self.visible.iter_mut().enumerate() {
            row.position = i as u64;
        }
    }

    pub fn active_run(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|r| r.is_active())
    }

    /// Queued user messages in the order they'll run (`deriveThreadQueueWorkflowState`):
    /// queue position, else run order. Each with the text of its message.
    pub fn queued_runs(&self) -> Vec<(&Run, String)> {
        let mut runs: Vec<&Run> = self.runs.iter().filter(|r| r.status == "queued").collect();
        runs.sort_by_key(|r| (r.queue_position.unwrap_or(r.ordinal), r.ordinal));
        runs.into_iter()
            .map(|run| {
                // A guest's queued message shows the guest's words, as the web's does.
                let message = self
                    .messages
                    .iter()
                    .find(|m| Some(&m.id) == run.user_message_id.as_ref());
                let from_message = message.map(|m| {
                    m.context
                        .as_ref()
                        .and_then(|c| c["guest"]["text"].as_str())
                        .map_or_else(|| m.text.clone(), str::to_owned)
                });
                let text = from_message
                    .or_else(|| {
                        self.turn_items
                            .iter()
                            .find(|i| {
                                i.kind == "user_message"
                                    && (i.message_id.is_some()
                                        && i.message_id == run.user_message_id
                                        || i.run_id.as_deref() == Some(run.id.as_str()))
                            })
                            .and_then(|i| i.text.clone())
                    })
                    .unwrap_or_else(|| "Queued message".into());
                (run, text)
            })
            .collect()
    }

    /// Stopping the thread held its queue; it waits for `queue.resume`.
    pub fn queue_held(&self) -> bool {
        self.runs
            .iter()
            .any(|r| r.status == "queued" && r.queue_held == Some(true))
    }

    /// Nothing is running, so a queued message can start a turn of its own.
    pub fn can_send_now(&self) -> bool {
        self.active_run().is_none()
            && self.thread.archived_at.is_none()
            && self.thread.deleted_at.is_none()
    }

    pub fn interruptible_run(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|r| r.is_interruptible())
    }

    /// Whether a steer can land now, as the orchestrator decides it: the active run is
    /// `running` with a running provider turn on its active attempt, and isn't compacting.
    pub fn can_steer(&self) -> bool {
        let Some(run) = self.active_run().filter(|r| r.status == "running") else {
            return false;
        };
        let turn = self.provider_turns.iter().any(|t| {
            t.status == "running"
                && t.run_attempt_id.is_some()
                && t.run_attempt_id == run.active_attempt_id
        });
        let compacting = self
            .turn_items
            .iter()
            .any(|i| i.kind == "compaction" && matches!(i.status.as_str(), "running" | "pending"));
        turn && !compacting
    }

    pub fn is_working(&self) -> bool {
        self.active_run().is_some() || self.thread.is_working()
    }

    /// Turns that captured a checkpoint, newest first: what a diff can be taken over.
    pub fn diff_turns(&self) -> Vec<&Checkpoint> {
        let mut turns: Vec<_> = self
            .checkpoints
            .iter()
            .filter(|c| c.app_run_ordinal.is_some() && c.run_id.is_some())
            .collect();
        turns.sort_by(|a, b| {
            b.app_run_ordinal
                .cmp(&a.app_run_ordinal)
                .then_with(|| b.captured_at.cmp(&a.captured_at))
        });
        turns
    }

    /// The pending request a turn item asks about, if it's still open.
    pub fn pending_request(&self, request_id: &str) -> Option<&RuntimeRequest> {
        self.runtime_requests
            .iter()
            .find(|r| r.id == request_id && r.status == "pending")
    }
}

fn upsert<T>(rows: &mut Vec<T>, item: T, id: impl Fn(&T) -> &String) {
    match rows.iter().position(|r| id(r) == id(&item)) {
        Some(i) => rows[i] = item,
        None => rows.push(item),
    }
}

fn upsert_rev<T>(rows: &mut Vec<T>, item: T, id: impl Fn(&T) -> &String) {
    match rows.iter().rposition(|r| id(r) == id(&item)) {
        Some(i) => rows[i] = item,
        None => rows.push(item),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, ordinal: u64, run: &str, text: &str) -> TurnItem {
        TurnItem {
            id: id.into(),
            thread_id: "t".into(),
            run_id: Some(run.into()),
            kind: "assistant_message".into(),
            ordinal,
            text: Some(text.into()),
            ..Default::default()
        }
    }

    fn thread() -> ThreadState {
        ThreadState {
            thread: ThreadShell {
                id: "t".into(),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn streamed_text_replaces_in_place_and_inserts_by_ordinal() {
        let mut s = thread();
        assert!(s.apply(1, "t", ThreadEvent::TurnItem(item("b", 2, "r", "Hel"))));
        assert!(s.apply(2, "t", ThreadEvent::TurnItem(item("b", 2, "r", "Hello"))));
        assert!(s.apply(3, "t", ThreadEvent::TurnItem(item("a", 1, "r", "Q"))));
        let order: Vec<_> = s
            .visible
            .iter()
            .map(|r| (r.source_item_id.as_str(), r.position))
            .collect();
        assert_eq!(order, [("a", 0), ("b", 1)]);
        assert_eq!(s.visible[1].item.text.as_deref(), Some("Hello"));
    }

    #[test]
    fn steering_needs_a_running_provider_turn_on_the_active_attempt() {
        let mut s = thread();
        let run = |status: &str| Run {
            id: "r".into(),
            status: status.into(),
            active_attempt_id: Some("a".into()),
            ..Default::default()
        };
        let turn = |status: &str| ProviderTurn {
            id: "p".into(),
            run_attempt_id: Some("a".into()),
            status: status.into(),
        };
        s.apply(1, "t", ThreadEvent::Run(run("starting")));
        assert!(!s.can_steer());
        s.apply(2, "t", ThreadEvent::Run(run("running")));
        assert!(!s.can_steer(), "no provider turn yet");
        s.apply(3, "t", ThreadEvent::ProviderTurn(turn("running")));
        assert!(s.can_steer());
        s.apply(4, "t", ThreadEvent::ProviderTurn(turn("completed")));
        assert!(
            !s.can_steer(),
            "the run is still finishing, but the agent stopped listening"
        );
    }

    #[test]
    fn stale_events_are_dropped_and_rolled_back_runs_hide_items() {
        let mut s = thread();
        s.apply(5, "t", ThreadEvent::TurnItem(item("a", 1, "r", "x")));
        assert!(!s.apply(5, "t", ThreadEvent::TurnItem(item("a", 1, "r", "stale"))));
        assert!(!s.apply(6, "other", ThreadEvent::TurnItem(item("z", 1, "r", "x"))));
        s.apply(
            7,
            "t",
            ThreadEvent::Run(Run {
                id: "r".into(),
                status: "rolled_back".into(),
                ..Default::default()
            }),
        );
        assert!(s.visible.is_empty());
        s.apply(
            8,
            "t",
            ThreadEvent::Run(Run {
                id: "r".into(),
                status: "completed".into(),
                ..Default::default()
            }),
        );
        assert_eq!(s.visible.len(), 1);
    }

    #[test]
    fn repository_identity_enrichment_keeps_threads() {
        let mut s = ShellState::default();
        let t = ThreadShell {
            id: "a".into(),
            project_id: "p".into(),
            ..Default::default()
        };
        s.apply(ShellStreamItem::Snapshot {
            snapshot: ShellSnapshot {
                snapshot_sequence: 4,
                threads: vec![t],
                older_settled: None,
                projects: vec![],
            },
            resolved_repository_identity_roots: None,
        });
        // The server follows the first snapshot with a thread-less enrichment.
        s.apply(ShellStreamItem::Snapshot {
            snapshot: ShellSnapshot {
                snapshot_sequence: 4,
                threads: vec![],
                older_settled: None,
                projects: vec![],
            },
            resolved_repository_identity_roots: Some(vec!["/repo".into()]),
        });
        assert_eq!(s.threads.len(), 1);
    }

    #[test]
    fn shell_archive_location_removes_and_old_sequences_drop() {
        let mut s = ShellState::default();
        let t = |id: &str| ThreadShell {
            id: id.into(),
            project_id: "p".into(),
            ..Default::default()
        };
        s.apply(ShellStreamItem::Snapshot {
            snapshot: ShellSnapshot {
                snapshot_sequence: 10,
                threads: vec![t("a")],
                older_settled: None,
                projects: vec![],
            },
            resolved_repository_identity_roots: None,
        });
        assert!(!s.apply(ShellStreamItem::ThreadUpdated {
            sequence: 9,
            location: "active".into(),
            thread: t("b")
        }));
        assert!(s.apply(ShellStreamItem::ThreadUpdated {
            sequence: 11,
            location: "archive".into(),
            thread: t("a")
        }));
        assert!(s.threads.is_empty());
    }

    #[test]
    fn settled_threads_leave_their_project_for_the_shelf() {
        let mut s = ShellState::default();
        let t = |id: &str, settled: Option<&str>, at: Option<&str>| ThreadShell {
            id: id.into(),
            project_id: "p".into(),
            settled_override: settled.map(Into::into),
            settled_at: at.map(Into::into),
            updated_at: "2026-11-01T00:00:00Z".into(),
            ..Default::default()
        };
        s.threads = vec![
            t("open", None, None),
            t("kept-active", Some("active"), Some("2026-11-05T00:00:00Z")),
            t("older", Some("settled"), Some("2026-11-02T00:00:00Z")),
            t("newer", Some("settled"), Some("2026-11-04T00:00:00Z")),
        ];
        let ids = |rows: Vec<&ThreadShell>| rows.iter().map(|t| t.id.clone()).collect::<Vec<_>>();
        assert_eq!(ids(s.project_threads("p")), ["open", "kept-active"]);
        assert_eq!(ids(s.settled_threads()), ["newer", "older"]);
    }

    #[test]
    fn queued_runs_follow_queue_position_and_hold() {
        let mut s = thread();
        let run = |id: &str, ordinal: u64, position: Option<u64>, held: bool| Run {
            id: id.into(),
            status: "queued".into(),
            ordinal,
            queue_position: position,
            queue_held: Some(held),
            user_message_id: Some(format!("m-{id}")),
            ..Default::default()
        };
        s.runs = vec![
            Run {
                id: "done".into(),
                status: "completed".into(),
                ordinal: 1,
                ..Default::default()
            },
            run("b", 3, Some(1), false),
            run("a", 2, Some(2), false),
            run("c", 4, None, false),
        ];
        s.turn_items.push(TurnItem {
            kind: "user_message".into(),
            message_id: Some("m-a".into()),
            text: Some("first queued".into()),
            ..Default::default()
        });
        // A queued message without a turn item yet reads from the conversation.
        s.messages.push(crate::protocol::Message {
            id: "m-c".into(),
            text: "from the conversation".into(),
            ..Default::default()
        });
        let order: Vec<_> = s
            .queued_runs()
            .iter()
            .map(|(r, t)| (r.id.clone(), t.clone()))
            .collect();
        assert_eq!(
            order,
            [
                ("b".into(), "Queued message".into()),
                ("a".into(), "first queued".into()),
                ("c".into(), "from the conversation".into()),
            ]
        );
        assert!(!s.queue_held() && s.can_send_now());
        s.runs[3].queue_held = Some(true);
        assert!(s.queue_held());
    }

    #[test]
    fn older_settled_threads_page_in_below_the_cut() {
        let settled = |id: &str, at: &str| ThreadShell {
            id: id.into(),
            project_id: "p".into(),
            settled_override: Some("settled".into()),
            settled_at: Some(at.into()),
            ..Default::default()
        };
        let mut s = ShellState::default();
        s.apply(ShellStreamItem::Snapshot {
            snapshot: ShellSnapshot {
                snapshot_sequence: 1,
                // One kept above the cut, one kept below it (pinned, say).
                threads: vec![
                    settled("new", "2026-11-07T10:00:00Z"),
                    settled("kept", "2026-11-01T00:00:00Z"),
                ],
                projects: vec![],
                older_settled: Some(OlderSettledSnapshot {
                    before: "2026-11-05T00:00:00Z".into(),
                    counts: vec![OlderSettledCount {
                        project_id: "p".into(),
                        count: 60,
                    }],
                }),
            },
            resolved_repository_identity_roots: None,
        });
        // The kept row below the cut is among the server's 60, so it isn't counted twice.
        assert_eq!(s.settled_count(), 61);
        assert_eq!(
            s.older_cursor(),
            Some(serde_json::json!({ "settledAt": "2026-11-05T00:00:00Z" }))
        );
        let page: Vec<_> = (0..OLDER_SETTLED_PAGE)
            .map(|i| {
                settled(
                    &format!("o{i:02}"),
                    &format!("2026-11-04T00:{:02}:00Z", 59 - i),
                )
            })
            .chain(std::iter::once(settled("kept", "2026-11-01T00:00:00Z")))
            .collect();
        s.add_older_page("2026-11-05T00:00:00Z", page);
        // A page of 51 rows: more may follow, after the last row read.
        assert_eq!(
            s.older_cursor(),
            Some(serde_json::json!({ "settledAt": "2026-11-01T00:00:00Z", "threadId": "kept" }))
        );
        // The live shell's own row wins over its paged copy.
        assert_eq!(s.older_rows().len(), OLDER_SETTLED_PAGE);
        s.add_older_page(
            "2026-11-05T00:00:00Z",
            vec![settled("last", "2026-10-01T00:00:00Z")],
        );
        assert_eq!((s.older_cursor(), s.older_frontier()), (None, None));
    }
}
