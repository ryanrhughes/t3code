//! Messages queued behind a running turn (`QueuedRunsControl.tsx`): send one now (a turn of its
//! own when nothing runs, else steered into the running one), edit it in the composer, remove
//! it, and resume a queue that Stop held.
use rune::{El, col, row, text};
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

    /// Sends a queued message now: it starts a turn when nothing runs, else steers into the
    /// running one. A message that starts a fresh context can only start a turn.
    pub fn send_queued_now(&mut self, run_id: &str) -> bool {
        let Some(t) = self.thread.as_ref() else {
            return false;
        };
        let fresh = t
            .runs
            .iter()
            .any(|r| r.id == run_id && r.fresh_context == Some(true));
        if t.can_send_now() {
            self.queue_command("queued-run.start", json!({ "runId": run_id }));
            true
        } else if t.can_steer() && !fresh {
            let target = t.active_run().map(|r| r.id.clone());
            self.queue_command(
                "queued-message.promote-to-steer",
                json!({ "queuedRunId": run_id, "targetRunId": target }),
            );
            true
        } else {
            false
        }
    }

    /// Ctrl/Cmd+Shift+Return (the web's `thread.steerQueuedMessage`): sends the next one.
    pub fn send_next_queued(&mut self) -> bool {
        let next = self
            .thread
            .as_ref()
            .and_then(|t| t.queued_runs().first().map(|(r, _)| r.id.clone()));
        next.is_some_and(|id| self.send_queued_now(&id))
    }

    /// Moves a queued message into the composer; the next send rewrites it in place.
    pub fn edit_queued(&mut self, run_id: &str) {
        let text = self.thread.as_ref().and_then(|t| {
            t.queued_runs()
                .into_iter()
                .find(|(r, _)| r.id == run_id)
                .map(|(_, text)| text)
        });
        let Some(text) = text else { return };
        self.composer = focused(&text);
        self.composer.set_selection(text.len(), text.len());
        self.editing_queued = Some(run_id.to_owned());
    }

    /// Alt+Up on an empty composer (the web's `thread.editQueuedMessage`): edits the last one.
    pub fn edit_last_queued(&mut self) -> bool {
        let last = self
            .thread
            .as_ref()
            .and_then(|t| t.queued_runs().last().map(|(r, _)| r.id.clone()));
        match last {
            Some(id) if self.composer.text.trim().is_empty() => {
                self.edit_queued(&id);
                true
            }
            _ => false,
        }
    }

    pub fn cancel_queued_edit(&mut self) {
        if self.editing_queued.take().is_some() {
            self.composer = focused("");
        }
    }

    /// Sends the composer as the edited text of the queued message being edited.
    pub fn save_queued_edit(&mut self, run_id: String, text: String) {
        self.queue_command("queued-run.edit", json!({ "runId": run_id, "text": text }));
        self.editing_queued = None;
        self.composer = focused("");
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
    let can_send = t.can_send_now() || t.can_steer();
    let send_label = if t.can_send_now() {
        "Send now"
    } else {
        "Steer"
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
    for (i, (run, message)) in queued.iter().enumerate() {
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
            if can_send && !(fresh && !t.can_send_now()) {
                let id = id.clone();
                actions = actions.child(
                    Button::new(send_label)
                        .variant(ButtonVariant::Ghost)
                        .control_size(ControlSize::Small)
                        .on_press(move |s: &mut App| {
                            s.send_queued_now(&id);
                        })
                        .build(k)
                        .key(format!("queued-send-{i}")),
                );
            }
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
