//! Plan mode: the agent proposes a plan instead of changing anything, then the user implements
//! it (here or in a new thread) or keeps refining it, as the web's composer does
//! (`useProposedPlanSend.ts`, `proposedPlan.ts`).
use rune::{El, Tx, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};
use serde_json::json;
use t3_client::protocol::{self, PLAN_PROMPT_PREFIX, plan_title, thread_command};

use crate::theme::Look;
use crate::{App, HANDLE, focused};

/// A proposed plan waiting on the user.
pub struct ReadyPlan {
    pub id: String,
    pub markdown: String,
    pub title: Option<String>,
}

impl App {
    /// Whether the next send plans instead of acting: the open thread's mode, else the draft's.
    pub fn plan_mode(&self) -> bool {
        match self.selected.as_deref() {
            Some(id) => self
                .shell()
                .thread(id)
                .is_some_and(|t| t.interaction_mode == "plan"),
            None => self.draft_plan,
        }
    }

    pub fn toggle_plan_mode(&mut self) {
        let next = if self.plan_mode() { "default" } else { "plan" };
        match (self.selected.clone(), self.session()) {
            (Some(id), Some(session)) => session.dispatch(thread_command(
                "thread.interaction-mode.set",
                &id,
                json!({ "interactionMode": next }),
            )),
            (None, _) => self.draft_plan = next == "plan",
            _ => {}
        }
    }

    /// The open thread's newest proposed plan, while the server says it's still actionable.
    pub fn ready_plan(&self) -> Option<ReadyPlan> {
        let id = self.selected.as_deref()?;
        if self.shell().thread(id)?.has_actionable_proposed_plan != Some(true) {
            return None;
        }
        let item = self
            .thread
            .as_ref()?
            .turn_items
            .iter()
            .filter(|i| i.kind == "proposed_plan" && i.plan_id.is_some())
            .max_by_key(|i| i.ordinal)?;
        let markdown = item.markdown.clone().unwrap_or_default();
        Some(ReadyPlan {
            id: item.plan_id.clone()?,
            title: plan_title(&markdown),
            markdown,
        })
    }

    /// Sends commands one after another, each once the server accepted the one before; `then`
    /// runs after the last. Plain dispatches race, and these depend on their order.
    pub fn dispatch_in_order(
        &mut self,
        env_id: String,
        mut commands: Vec<serde_json::Value>,
        then: Option<Box<dyn FnOnce(&mut App, &mut Tx) + Send>>,
    ) {
        let (Some(i), Some(handle)) = (self.env_index(&env_id), HANDLE.get().cloned()) else {
            return;
        };
        let Some(session) = self.envs[i].session.clone() else {
            return;
        };
        if commands.is_empty() {
            return;
        }
        let first = commands.remove(0);
        session.call("orchestration.dispatchCommand", first, move |result| {
            handle.update(move |s: &mut App, tx| match result {
                Err(e) => s.toast(e, tx),
                Ok(_) if commands.is_empty() => {
                    if let Some(then) = then {
                        then(s, tx)
                    }
                }
                Ok(_) => s.dispatch_in_order(env_id, commands, then),
            });
        });
    }

    /// Implements the ready plan in its own thread: out of plan mode first, then the plan as
    /// the message, in that order.
    pub fn implement_plan(&mut self, tx: &mut Tx) {
        let (Some(plan), Some(thread), Some(env_id)) = (
            self.ready_plan(),
            self.selected.clone(),
            self.env().map(|e| e.cred.environment_id.clone()),
        ) else {
            return;
        };
        let mut commands = Vec::new();
        if self.plan_mode() {
            commands.push(thread_command(
                "thread.interaction-mode.set",
                &thread,
                json!({ "interactionMode": "default" }),
            ));
        }
        let text = format!("{PLAN_PROMPT_PREFIX}{}", plan.markdown.trim());
        let mode = protocol::DispatchMode::resolve(
            self.thread
                .as_ref()
                .is_some_and(|t| t.active_run().is_some()),
            false,
            false,
            self.thread.as_ref().is_some_and(|t| t.can_steer()),
            self.prefs.follow_up == "queue",
        );
        let mut cmd = protocol::message_dispatch(&thread, &text, self.model.as_ref(), mode);
        protocol::with_plan(&mut cmd, &thread, &plan.id);
        commands.push(cmd);
        self.dispatch_in_order(env_id, commands, None);
        self.composer = focused("");
        let _ = tx;
    }

    /// Implements the ready plan in a new thread that works where this one does (its branch and
    /// worktree), linked to the plan with `sourcePlanRef`, as the web's
    /// `useImplementPlanInNewThread` does: `thread.create`, then the plan as its first message.
    pub fn implement_plan_in_new_thread(&mut self, tx: &mut Tx) {
        let (Some(plan), Some(source), Some(env_id)) = (
            self.ready_plan(),
            self.selected
                .as_deref()
                .and_then(|id| self.shell().thread(id))
                .cloned(),
            self.env().map(|e| e.cred.environment_id.clone()),
        ) else {
            return;
        };
        let Some(model) = self.effective_model() else {
            self.toast("No provider is ready on this server", tx);
            return;
        };
        let runtime = self
            .config()
            .map_or("full-access".to_owned(), |c| c.default_runtime_mode());
        let title: String = match &plan.title {
            Some(t) => format!("Implement {t}"),
            None => "Implement plan".into(),
        }
        .chars()
        .take(80)
        .collect();
        let id = protocol::new_id();
        let create = json!({
            "type": "thread.create",
            "commandId": protocol::new_id(),
            "createdBy": "user",
            "creationSource": protocol::CREATION_SOURCE,
            "threadId": id,
            "projectId": source.project_id,
            "title": title,
            "modelSelection": model,
            "runtimeMode": runtime,
            "interactionMode": "default",
            "branch": source.branch,
            "worktreePath": source.worktree_path,
        });
        let text = format!("{PLAN_PROMPT_PREFIX}{}", plan.markdown.trim());
        let mut send =
            protocol::message_dispatch(&id, &text, Some(&model), protocol::DispatchMode::Auto);
        protocol::with_plan(&mut send, &source.id, &plan.id);
        let (open_env, open_id) = (env_id.clone(), id.clone());
        self.dispatch_in_order(
            env_id,
            vec![create, send],
            Some(Box::new(move |s: &mut App, _| {
                if let Some(env) = s.env_index(&open_env) {
                    s.open_thread_in(env, open_id);
                }
            })),
        );
    }
}

/// "Plan ready · title" above the composer, with its two ways forward.
pub fn banner(s: &App, look: &Look) -> Option<El> {
    let plan = s.ready_plan()?;
    let k = &look.kit;
    let mut words = row()
        .gap(6.0)
        .center_y()
        .shrink()
        .min_w(0.0)
        .child(text("Plan ready").font(k.ty.small_medium).color(look.muted));
    if let Some(title) = plan.title {
        words = words.child(
            text(title)
                .font(k.ty.small)
                .color(look.text)
                .truncate()
                .shrink(),
        );
    }
    Some(
        row()
            .key("plan-banner")
            .w_full()
            .gap(8.0)
            .px(4.0)
            .center_y()
            .child(words.grow(1.0))
            .child(
                Button::new("Implement in new thread")
                    .variant(ButtonVariant::Ghost)
                    .control_size(ControlSize::Small)
                    .on_press_tx(|s: &mut App, tx| s.implement_plan_in_new_thread(tx))
                    .build(k)
                    .key("plan-new-thread"),
            )
            .child(
                Button::new("Implement")
                    .variant(ButtonVariant::Contrast)
                    .control_size(ControlSize::Small)
                    .on_press_tx(|s: &mut App, tx| s.implement_plan(tx))
                    .build(k)
                    .key("plan-implement"),
            ),
    )
}
