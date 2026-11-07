//! Plan mode: the agent proposes a plan instead of changing anything, then the user implements
//! it (here or in a new thread) or keeps refining it, as the web's composer does
//! (`useProposedPlanSend.ts`, `proposedPlan.ts`).
use rune::{El, Tx, row, text};
use rune_kit::{Button, ButtonVariant, ControlSize};
use serde_json::json;
use t3_client::protocol::{self, PLAN_PROMPT_PREFIX, plan_title, thread_command};

use crate::theme::Look;
use crate::{App, focused};

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

    /// Implements the ready plan in its own thread: out of plan mode, then the plan as the message.
    pub fn implement_plan(&mut self, tx: &mut Tx) {
        let (Some(plan), Some(thread), Some(session)) =
            (self.ready_plan(), self.selected.clone(), self.session())
        else {
            return;
        };
        if self.plan_mode() {
            session.dispatch(thread_command(
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
        session.dispatch(cmd);
        self.composer = focused("");
        let _ = tx;
    }

    /// Implements the ready plan in a fresh thread in the same project, leaving this one as is.
    pub fn implement_plan_in_new_thread(&mut self, tx: &mut Tx) {
        let (Some(plan), Some(session)) = (self.ready_plan(), self.session()) else {
            return;
        };
        let Some(project) = self
            .selected
            .as_deref()
            .and_then(|id| self.shell().thread(id))
            .map(|t| t.project_id.clone())
        else {
            return;
        };
        let Some(model) = self.effective_model() else {
            self.toast("No provider is ready on this server", tx);
            return;
        };
        let runtime = self
            .config()
            .map_or("full-access".to_owned(), |c| c.default_runtime_mode());
        let text = format!("{PLAN_PROMPT_PREFIX}{}", plan.markdown.trim());
        let mut input = protocol::launch_thread(&project, &text, &model, &runtime);
        input["title"] = match &plan.title {
            Some(t) => format!("Implement {t}"),
            None => "Implement plan".into(),
        }
        .chars()
        .take(80)
        .collect::<String>()
        .into();
        input["generateTitle"] = false.into();
        session.launch(input);
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
