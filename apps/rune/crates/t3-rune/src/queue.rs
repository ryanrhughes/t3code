//! Messages queued behind a running turn (`QueuedRunsControl.tsx`): send one now (a turn of its
//! own when nothing runs, else steered into the running one), edit it in the composer, remove
//! it, and resume a queue that Stop held.
use rune::{El, Tx, col, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};
use serde_json::json;
use t3_client::protocol::thread_command;

use crate::theme::Look;
use crate::{App, focused};

impl App {
    fn queue_command(&mut self, kind: &str, extra: serde_json::Value) {
        if let (Some(thread), Some(session)) = (self.selected.clone(), self.session()) {
            session.dispatch(thread_command(kind, &thread, extra));
        }
    }

    /// Sends a queued message now (`sendQueuedMessage`): a turn of its own when nothing runs,
    /// else into the running turn, which a running `/compact` gives way to.
    pub fn send_queued_now(&mut self, run_id: &str) -> bool {
        let Some(t) = self.thread.as_ref() else {
            return false;
        };
        let fresh = t
            .runs
            .iter()
            .any(|r| r.id == run_id && r.fresh_context == Some(true));
        if !t.can_send_queued(fresh) {
            return false;
        }
        if t.can_send_now() {
            self.queue_command("queued-run.start", json!({ "runId": run_id }));
        } else if let Some(target) = t.active_run().map(|r| r.id.clone()) {
            self.queue_command(
                "queued-message.promote-to-steer",
                json!({ "queuedRunId": run_id, "targetRunId": target }),
            );
        }
        true
    }

    /// Ctrl/Cmd+Shift+Return (the web's `thread.steerQueuedMessage`): sends the next one.
    pub fn send_next_queued(&mut self) -> bool {
        let next = self
            .thread
            .as_ref()
            .and_then(|t| t.queued_runs().first().map(|q| q.run.id.clone()));
        next.is_some_and(|id| self.send_queued_now(&id))
    }

    /// Moves a queued message into the composer; the next send rewrites it in place. The draft
    /// that was there waits and comes back after.
    pub fn edit_queued(&mut self, run_id: &str) {
        let text = self.thread.as_ref().and_then(|t| {
            t.queued_runs()
                .into_iter()
                .find(|q| q.run.id == run_id && q.editable)
                .map(|q| q.text)
        });
        let Some(text) = text else { return };
        let mut edit = focused(&text);
        edit.set_selection(text.len(), text.len());
        let draft = std::mem::replace(&mut self.composer, edit);
        if self.editing_queued.is_none() {
            self.stashed_draft = Some(draft);
        }
        self.editing_queued = Some(run_id.to_owned());
    }

    /// Alt+Up on an empty composer (the web's `thread.editQueuedMessage`): edits the last one
    /// the user can edit.
    pub fn edit_last_queued(&mut self) -> bool {
        let last = self.thread.as_ref().and_then(|t| {
            t.queued_runs()
                .iter()
                .rev()
                .find(|q| q.editable)
                .map(|q| q.run.id.clone())
        });
        match last {
            Some(id) if self.composer.text.trim().is_empty() => {
                self.edit_queued(&id);
                true
            }
            _ => false,
        }
    }

    fn restore_draft(&mut self) {
        self.composer = self.stashed_draft.take().unwrap_or_else(|| focused(""));
        self.composer.focused = true;
    }

    pub fn cancel_queued_edit(&mut self) {
        if self.editing_queued.take().is_some() {
            self.restore_draft();
        }
    }

    /// Saves the composer as the queued message's new text, with any files attached meanwhile
    /// (and the ones it had). The edit stays open until the server takes it, so a refusal (the
    /// message ran meanwhile, say) loses nothing.
    pub fn save_queued_edit(&mut self, run_id: String, text: String, tx: &mut Tx) {
        let files = match self.ready_attachments() {
            Ok(files) => files,
            Err(why) => return self.toast(why, tx),
        };
        let (Some(thread), Some(env_id)) = (
            self.selected.clone(),
            self.env().map(|e| e.cred.environment_id.clone()),
        ) else {
            return;
        };
        let mut extra = json!({ "runId": run_id, "text": text });
        if !files.is_empty() {
            // `attachments` replaces the message's; absent keeps them.
            let mut all = self
                .thread
                .as_ref()
                .and_then(|t| t.queued_runs().into_iter().find(|q| q.run.id == run_id))
                .map(|q| q.attachments)
                .unwrap_or_default();
            all.extend(files);
            extra["attachments"] = all.into();
        }
        let command = thread_command("queued-run.edit", &thread, extra);
        let saved = run_id.clone();
        self.dispatch_in_order(
            env_id.clone(),
            vec![command],
            Some(Box::new(move |s: &mut App, _| {
                if s.editing_queued.as_deref() == Some(saved.as_str()) {
                    s.editing_queued = None;
                    s.restore_draft();
                    s.attachments.retain(|a| a.env_id != env_id);
                }
            })),
        );
    }

    /// From `poll`: a queued message being edited that ran or was removed meanwhile can't take
    /// the edit; its text stays in the composer as an ordinary draft.
    pub fn check_queued_edit(&mut self, tx: &mut Tx) {
        let (Some(run), Some(t)) = (self.editing_queued.as_deref(), self.thread.as_ref()) else {
            return;
        };
        if !t.synchronized || t.queued_runs().iter().any(|q| q.run.id == run) {
            return;
        }
        self.editing_queued = None;
        if let Some(draft) = self
            .stashed_draft
            .take()
            .filter(|d| !d.text.trim().is_empty())
        {
            let text = format!("{}\n\n{}", self.composer.text, draft.text);
            self.composer = focused(&text);
        }
        self.toast(
            "That message left the queue before the edit was saved; your edit is in the composer",
            tx,
        );
    }

    pub fn remove_queued(&mut self, run_id: &str) {
        if self.editing_queued.as_deref() == Some(run_id) {
            self.cancel_queued_edit();
        }
        self.queue_command("queued-run.cancel", json!({ "runId": run_id }));
    }

    pub fn resume_queue(&mut self) {
        self.queue_command("queue.resume", json!({}));
    }
}

/// The queue above the composer: a header (with Resume when Stop held it) and one row each.
pub fn panel(s: &App, look: &Look) -> Option<El> {
    let t = s.thread.as_ref()?;
    let queued = t.queued_runs();
    if queued.is_empty() {
        return None;
    }
    let k = &look.kit;
    let held = t.queue_held();
    let send_help = if t.can_send_now() {
        "Start a turn with this message"
    } else if t.promote_skips_compaction() {
        "Skip compaction and send this message"
    } else {
        "Send into the current turn"
    };
    let mut header = row().w_full().gap(8.0).center_y().child(
        text(if held {
            format!("{} queued · paused after Stop", queued.len())
        } else {
            format!("{} queued", queued.len())
        })
        .font(k.ty.small_medium)
        .color(look.muted)
        .grow(1.0),
    );
    if held {
        header = header.child(
            Button::new("Resume")
                .variant(ButtonVariant::Contrast)
                .control_size(ControlSize::Small)
                .on_press(|s: &mut App| s.resume_queue())
                .build(k)
                .key("queue-resume"),
        );
    }
    let mut list = col().w_full().gap(4.0).child(header);
    for (i, q) in queued.iter().enumerate() {
        let (run, message) = (q.run, &q.text);
        let editing = s.editing_queued.as_deref() == Some(run.id.as_str());
        let fresh = run.fresh_context == Some(true);
        let mut actions = row().gap(2.0).center_y();
        let id = run.id.clone();
        if editing {
            actions = actions.child(
                text("Editing in the composer")
                    .font(k.ty.small)
                    .color(look.muted),
            );
        } else {
            if t.can_send_queued(fresh) {
                let id = id.clone();
                actions = actions.child(
                    Button::new("Send now")
                        .help(send_help)
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| {
                            s.send_queued_now(&id);
                        })
                        .build(k)
                        .key(format!("queued-send-{i}")),
                );
            }
            // A guest's message can be removed, not rewritten.
            if q.editable {
                let edit_id = id.clone();
                actions = actions.child(
                    Button::new("Edit")
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| s.edit_queued(&edit_id))
                        .build(k)
                        .key(format!("queued-edit-{i}")),
                );
            }
        }
        actions = actions.child(
            Button::new("Remove")
                .variant(ButtonVariant::GhostFaint)
                .control_size(ControlSize::Small)
                .on_press(move |s: &mut App| s.remove_queued(&id))
                .build(k)
                .key(format!("queued-remove-{i}")),
        );
        list = list.child(
            row()
                .key(format!("queued-{}", run.id))
                .w_full()
                .gap(8.0)
                .px(10.0)
                .py(6.0)
                .center_y()
                .radius(k.radius.md)
                .border(1.0, if editing { look.accent } else { look.border })
                .child(
                    text(message.lines().next().unwrap_or("").to_owned())
                        .font(k.ty.ui)
                        .color(look.text)
                        .truncate()
                        .grow(1.0)
                        .shrink(),
                )
                .child(actions),
        );
    }
    Some(list)
}
