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
}

impl ShellState {
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
    pub fn project_threads(&self, project_id: &str) -> Vec<&ThreadShell> {
        let mut rows: Vec<_> = self
            .threads
            .iter()
            .filter(|t| t.project_id == project_id && t.is_sidebar_row())
            .collect();
        rows.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
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
    pub runtime_requests: Vec<RuntimeRequest>,
    /// Local turn items, by id.
    pub turn_items: Vec<TurnItem>,
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
            runtime_requests: p.runtime_requests,
            turn_items: p.turn_items,
            visible: p.visible_turn_items,
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

    pub fn interruptible_run(&self) -> Option<&Run> {
        self.runs.iter().rev().find(|r| r.is_interruptible())
    }

    pub fn is_working(&self) -> bool {
        self.active_run().is_some() || self.thread.is_working()
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
            }),
        );
        assert!(s.visible.is_empty());
        s.apply(
            8,
            "t",
            ThreadEvent::Run(Run {
                id: "r".into(),
                status: "completed".into(),
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
                projects: vec![],
            },
            resolved_repository_identity_roots: None,
        });
        // The server follows the first snapshot with a thread-less enrichment.
        s.apply(ShellStreamItem::Snapshot {
            snapshot: ShellSnapshot {
                snapshot_sequence: 4,
                threads: vec![],
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
}
