//! The slice of `packages/contracts` the Rune client reads and writes, as lenient serde types.
//!
//! Invariants, from the Effect Schema JSON encoding:
//! - Branded ids are plain strings; `DateTimeUtc` is an ISO string.
//! - `Schema.optional` keys are absent (never `null`); `Schema.NullOr` keys are present and may be `null`.
//! - Unknown fields are ignored and unknown union tags land on an `Unknown` variant, so a newer
//!   server never fails a whole frame.
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

// ---------------------------------------------------------------------------------------------
// Server config (`subscribeServerConfig`, packages/contracts/src/server.ts)

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct EnvironmentDescriptor {
    pub environment_id: String,
    pub label: String,
    pub server_version: String,
    pub orchestration_protocol_version: Option<u32>,
    pub capabilities: Value,
}

impl EnvironmentDescriptor {
    pub fn capability(&self, name: &str) -> bool {
        self.capabilities
            .get(name)
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerProviderModel {
    pub slug: String,
    pub name: String,
    pub short_name: Option<String>,
    pub is_custom: bool,
    pub is_default: Option<bool>,
    pub is_legacy: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerProvider {
    pub instance_id: String,
    pub driver: String,
    pub display_name: Option<String>,
    pub enabled: bool,
    pub installed: bool,
    pub status: String,
    pub availability: Option<String>,
    pub accent_color: Option<String>,
    pub models: Vec<ServerProviderModel>,
    pub slash_commands: Vec<SlashCommand>,
    pub skills: Vec<Skill>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct SlashCommand {
    pub name: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Skill {
    pub name: String,
    pub description: Option<String>,
    pub short_description: Option<String>,
    pub enabled: bool,
}

impl ServerProvider {
    pub fn usable(&self) -> bool {
        self.enabled
            && self.availability.as_deref() != Some("unavailable")
            && self.status != "disabled"
    }

    pub fn label(&self) -> &str {
        self.display_name.as_deref().unwrap_or(&self.instance_id)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ServerConfig {
    pub environment: EnvironmentDescriptor,
    pub providers: Vec<ServerProvider>,
    /// `ServerSettings` is large; read the few keys the client needs through accessors.
    pub settings: Value,
}

impl ServerConfig {
    pub fn default_model_selection(&self) -> Option<ModelSelection> {
        serde_json::from_value(self.settings.get("defaultModelSelection")?.clone()).ok()
    }

    pub fn default_runtime_mode(&self) -> String {
        self.settings
            .get("defaultRuntimeMode")
            .and_then(Value::as_str)
            .unwrap_or("full-access")
            .to_owned()
    }

    /// `t3 theme set <id>` writes these; clients adopt each `(theme, setAt)` generation once.
    pub fn default_theme(&self) -> Option<(String, String)> {
        // The server keeps an empty `defaultTheme` when none was ever set.
        let theme = self
            .settings
            .get("defaultTheme")?
            .as_str()
            .filter(|t| !t.trim().is_empty())?
            .to_owned();
        let at = self
            .settings
            .get("defaultThemeSetAt")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_owned();
        Some((theme, at))
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ServerConfigEvent {
    Snapshot {
        config: ServerConfig,
    },
    ProviderStatuses {
        payload: ProvidersPayload,
    },
    SettingsUpdated {
        payload: SettingsPayload,
    },
    /// Raw `EnvironmentTheme` objects; `t3_theme::environment_theme` validates each.
    EnvironmentThemesUpdated {
        payload: ThemesPayload,
    },
    #[serde(other)]
    Unknown,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ProvidersPayload {
    pub providers: Vec<ServerProvider>,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SettingsPayload {
    pub settings: Value,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ThemesPayload {
    pub themes: Vec<Value>,
}

// ---------------------------------------------------------------------------------------------
// Models (packages/contracts/src/modelSelection.ts)

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelSelection {
    #[serde(alias = "provider")]
    pub instance_id: String,
    pub model: String,
    /// `[{id, value}]` on the wire; decode also accepts the legacy object form, so keep it opaque.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub options: Option<Value>,
}

// ---------------------------------------------------------------------------------------------
// Shell (`orchestration.subscribeShell`, orchestrationV2.ts ~2083-2273)

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectShell {
    pub id: String,
    pub title: String,
    pub workspace_root: String,
    pub default_model_selection: Option<ModelSelection>,
    pub pinned_at: Option<String>,
    pub archived_at: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Lineage {
    pub parent_thread_id: Option<String>,
    pub relationship_to_parent: Option<String>,
    pub side_chat: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct LatestMessage {
    pub role: String,
    pub text: String,
}

/// `OrchestrationV2ThreadShell`; the thread projection's `AppThread` is a superset, so this
/// struct decodes both.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadShell {
    pub id: String,
    pub project_id: String,
    pub title: String,
    pub provider_instance_id: String,
    pub model_selection: ModelSelection,
    pub runtime_mode: String,
    pub interaction_mode: String,
    pub branch: Option<String>,
    pub worktree_path: Option<String>,
    pub lineage: Lineage,
    /// `"idle"` or a run status (`preparing|queued|starting|running|waiting|completed|…`).
    pub status: String,
    pub active_run_id: Option<String>,
    /// When the current run began; the UI shows "Working for 1m 4s" from it.
    pub activity_run_started_at: Option<String>,
    pub pending_runtime_request: Option<Value>,
    pub latest_visible_message: Option<LatestMessage>,
    pub created_at: String,
    pub updated_at: String,
    pub archived_at: Option<String>,
    pub settled_at: Option<String>,
    /// `"settled"` puts the thread on the sidebar's Settled shelf; `"active"` keeps it out.
    pub settled_override: Option<String>,
    pub deleted_at: Option<String>,
    pub pinned_at: Option<String>,
}

impl ThreadShell {
    /// Subagent children and side chats live under their parent, not in the sidebar.
    pub fn is_sidebar_row(&self) -> bool {
        self.deleted_at.is_none()
            && self.lineage.relationship_to_parent.as_deref() != Some("subagent")
            && self.lineage.side_chat != Some(true)
    }

    pub fn is_settled(&self) -> bool {
        self.settled_override.as_deref() == Some("settled")
    }

    pub fn is_working(&self) -> bool {
        matches!(
            self.status.as_str(),
            "preparing" | "queued" | "starting" | "running"
        )
    }

    pub fn needs_attention(&self) -> bool {
        self.pending_runtime_request
            .as_ref()
            .is_some_and(|v| !v.is_null())
            || self.status == "waiting"
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ShellSnapshot {
    pub snapshot_sequence: u64,
    pub threads: Vec<ThreadShell>,
    pub projects: Vec<ProjectShell>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind")]
pub enum ShellStreamItem {
    #[serde(rename = "synchronized")]
    Synchronized,
    #[serde(rename = "snapshot", rename_all = "camelCase")]
    Snapshot {
        snapshot: ShellSnapshot,
        resolved_repository_identity_roots: Option<Vec<String>>,
    },
    #[serde(rename = "project.updated")]
    ProjectUpdated {
        sequence: u64,
        project: ProjectShell,
    },
    #[serde(rename = "project.removed", rename_all = "camelCase")]
    ProjectRemoved { sequence: u64, project_id: String },
    #[serde(rename = "thread.updated")]
    ThreadUpdated {
        sequence: u64,
        location: String,
        thread: ThreadShell,
    },
    #[serde(rename = "thread.removed", rename_all = "camelCase")]
    ThreadRemoved {
        sequence: u64,
        location: String,
        thread_id: String,
    },
    #[serde(other)]
    Unknown,
}

// ---------------------------------------------------------------------------------------------
// Thread (`orchestration.subscribeThread`, orchestrationV2.ts ~1505-2047, ~3907-3930)

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ApprovalOption {
    pub decision: String,
    pub label: String,
    pub warning: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct QuestionOption {
    pub label: String,
    pub description: String,
    pub value: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct UserInputQuestion {
    pub id: String,
    pub header: String,
    pub question: String,
    pub options: Vec<QuestionOption>,
    pub multi_select: Option<bool>,
    pub allow_custom_answer: Option<bool>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct PlanStep {
    pub id: String,
    pub text: String,
    pub status: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Failure {
    pub class: String,
    pub message: String,
}

/// One `OrchestrationV2TurnItem`, flattened: `kind` is the wire `type`, and each variant's fields
/// are optional here. Only the fields the client renders are listed.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TurnItem {
    pub id: String,
    pub thread_id: String,
    pub run_id: Option<String>,
    #[serde(rename = "type")]
    pub kind: String,
    pub ordinal: u64,
    pub status: String,
    pub title: Option<String>,
    pub started_at: Option<String>,
    pub completed_at: Option<String>,
    pub updated_at: String,
    pub tool_name: Option<String>,

    // user_message / assistant_message / reasoning
    pub text: Option<String>,
    pub streaming: Option<bool>,
    pub input_intent: Option<String>,
    // proposed_plan / todo_list
    pub markdown: Option<String>,
    pub steps: Option<Vec<PlanStep>>,
    pub explanation: Option<String>,
    // command_execution (String) / dynamic_tool (unknown)
    pub input: Option<Value>,
    pub output: Option<Value>,
    pub output_omitted: Option<bool>,
    pub exit_code: Option<i64>,
    // file_change
    pub file_name: Option<String>,
    pub additions: Option<u64>,
    pub deletions: Option<u64>,
    // file_search / web_search
    pub pattern: Option<String>,
    pub patterns: Option<Vec<String>>,
    // approval_request / user_input_request
    pub request_id: Option<String>,
    pub request_kind: Option<String>,
    pub prompt: Option<String>,
    pub reason: Option<String>,
    pub options: Option<Vec<ApprovalOption>>,
    pub questions: Option<Vec<UserInputQuestion>>,
    // error / notices
    pub failure: Option<Failure>,
    pub message: Option<String>,
    // checkpoint
    pub checkpoint_id: Option<String>,
    // subagent / compaction
    pub progress: Option<String>,
    pub result: Option<String>,
    pub summary: Option<String>,
}

impl TurnItem {
    pub fn is_streaming(&self) -> bool {
        self.streaming == Some(true)
    }

    pub fn input_text(&self) -> Option<&str> {
        self.input.as_ref()?.as_str()
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProjectedTurnItem {
    pub position: u64,
    /// `local | inherited | synthetic`.
    pub visibility: String,
    pub source_thread_id: String,
    pub source_item_id: String,
    pub item: TurnItem,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Run {
    pub id: String,
    pub status: String,
    pub active_attempt_id: Option<String>,
}

impl Run {
    pub fn is_interruptible(&self) -> bool {
        matches!(self.status.as_str(), "preparing" | "starting" | "running")
    }

    pub fn is_active(&self) -> bool {
        self.is_interruptible() || self.status == "waiting"
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct RuntimeRequest {
    pub id: String,
    pub kind: String,
    /// `pending | resolved | expired | cancelled`.
    pub status: String,
    pub response_capability: Value,
}

impl RuntimeRequest {
    pub fn supports_feedback(&self) -> bool {
        self.response_capability
            .get("supportsFeedback")
            .and_then(Value::as_bool)
            .unwrap_or(false)
    }
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadProjection {
    pub thread: ThreadShell,
    pub runs: Vec<Run>,
    pub provider_turns: Vec<ProviderTurn>,
    pub checkpoints: Vec<Checkpoint>,
    pub runtime_requests: Vec<RuntimeRequest>,
    pub turn_items: Vec<TurnItem>,
    pub visible_turn_items: Vec<ProjectedTurnItem>,
}

/// A turn's snapshot of the workspace. Those with `app_run_ordinal` are the turns a diff spans:
/// turn N's changes are checkpoint N−1 to N (`deriveThreadCheckpointSummaries`).
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Checkpoint {
    pub id: String,
    pub run_id: Option<String>,
    pub app_run_ordinal: Option<u64>,
    /// `ready | missing | error | stale`.
    pub status: String,
    pub files: Vec<CheckpointFile>,
    pub captured_at: String,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct CheckpointFile {
    pub path: String,
    pub kind: String,
    pub additions: u64,
    pub deletions: u64,
}

/// One native provider turn; a run can only be steered while its active attempt has a running one.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderTurn {
    pub id: String,
    pub run_attempt_id: Option<String>,
    /// `pending | running | completed | interrupted | failed | cancelled`.
    pub status: String,
}

/// The thread events the client applies, decoded off the UI thread. Everything else only
/// advances the resume cursor.
#[derive(Clone, Debug)]
pub enum ThreadEvent {
    Thread(ThreadShell),
    Deleted,
    Run(Run),
    ProviderTurn(ProviderTurn),
    Checkpoint(Checkpoint),
    RuntimeRequest(RuntimeRequest),
    TurnItem(TurnItem),
    Other,
}

impl ThreadEvent {
    pub fn decode(kind: &str, payload: Value) -> ThreadEvent {
        fn decode<T: serde::de::DeserializeOwned>(p: Value) -> Option<T> {
            serde_json::from_value(p).ok()
        }
        match kind {
            "thread.deleted" => ThreadEvent::Deleted,
            "thread.pull-request-link-updated" => ThreadEvent::Other,
            k if k.starts_with("thread.") => {
                decode(payload).map_or(ThreadEvent::Other, ThreadEvent::Thread)
            }
            "run.created" | "run.updated" => {
                decode(payload).map_or(ThreadEvent::Other, ThreadEvent::Run)
            }
            "checkpoint.captured" => {
                decode(payload).map_or(ThreadEvent::Other, ThreadEvent::Checkpoint)
            }
            "provider-turn.updated" => {
                decode(payload).map_or(ThreadEvent::Other, ThreadEvent::ProviderTurn)
            }
            "runtime-request.updated" => {
                decode(payload).map_or(ThreadEvent::Other, ThreadEvent::RuntimeRequest)
            }
            "turn-item.updated" => {
                decode(payload).map_or(ThreadEvent::Other, ThreadEvent::TurnItem)
            }
            _ => ThreadEvent::Other,
        }
    }
}

#[derive(Clone, Debug)]
pub enum ThreadStreamItem {
    Snapshot {
        sequence: u64,
        projection: Box<ThreadProjection>,
    },
    Synchronized,
    Event {
        sequence: u64,
        thread_id: String,
        event: ThreadEvent,
    },
    Unknown,
}

impl ThreadStreamItem {
    pub fn decode(raw: Value) -> ThreadStreamItem {
        let Value::Object(mut obj) = raw else {
            return ThreadStreamItem::Unknown;
        };
        let seq = |o: &serde_json::Map<String, Value>, k: &str| {
            o.get(k).and_then(Value::as_u64).unwrap_or(0)
        };
        match obj.get("kind").and_then(Value::as_str) {
            Some("synchronized") => ThreadStreamItem::Synchronized,
            Some("snapshot") => {
                let sequence = seq(&obj, "snapshotSequence");
                match obj
                    .remove("projection")
                    .map(serde_json::from_value::<ThreadProjection>)
                {
                    Some(Ok(p)) => ThreadStreamItem::Snapshot {
                        sequence,
                        projection: Box::new(p),
                    },
                    _ => ThreadStreamItem::Unknown,
                }
            }
            Some("event") => {
                let sequence = seq(&obj, "sequence");
                let Some(Value::Object(mut ev)) = obj.remove("event") else {
                    return ThreadStreamItem::Unknown;
                };
                let kind = ev
                    .get("type")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let thread_id = ev
                    .get("threadId")
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_owned();
                let payload = ev.remove("payload").unwrap_or(Value::Null);
                ThreadStreamItem::Event {
                    sequence,
                    thread_id,
                    event: ThreadEvent::decode(&kind, payload),
                }
            }
            _ => ThreadStreamItem::Unknown,
        }
    }
}

// ---------------------------------------------------------------------------------------------
// Commands (`orchestration.dispatchCommand` / `orchestration.launchThread`)

pub const CREATION_SOURCE: &str = "web";

pub fn new_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// How a message lands while a run may be active (`ComposerDispatchMode` in client-runtime).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DispatchMode {
    /// No run active: start a turn.
    Auto,
    /// Redirect the active run with this message.
    Steer,
    /// Wait for the active run to finish, then start a turn.
    Queue,
    /// Stop the active run and start over with this message.
    Restart,
}

impl DispatchMode {
    /// `resolveComposerDispatchMode`: the `followUpBehavior` setting (steer unless
    /// `queue_by_default`) while running; the alternate modifier (Ctrl/Cmd) picks the other. Waiting approvals force steer, since a send redirects the agent.
    /// A run that can't take steering yet (still starting, or compacting) gets the message queued:
    /// the server rejects a steer then.
    pub fn resolve(
        running: bool,
        alternate: bool,
        approvals_waiting: bool,
        can_steer: bool,
        queue_by_default: bool,
    ) -> DispatchMode {
        if !running {
            return DispatchMode::Auto;
        }
        if !can_steer {
            return DispatchMode::Queue;
        }
        if !approvals_waiting && alternate != queue_by_default {
            DispatchMode::Queue
        } else {
            DispatchMode::Steer
        }
    }
}

pub fn message_dispatch(
    thread_id: &str,
    text: &str,
    model: Option<&ModelSelection>,
    mode: DispatchMode,
) -> Value {
    let mut cmd = json!({
        "type": "message.dispatch",
        "commandId": new_id(),
        "createdBy": "user",
        "creationSource": CREATION_SOURCE,
        "threadId": thread_id,
        "messageId": new_id(),
        "text": text,
        "attachments": [],
        "dispatchMode": { "type": if mode == DispatchMode::Queue { "queue_after_active" } else { "start_immediately" } },
    });
    let intent = match mode {
        DispatchMode::Auto => Some("auto"),
        DispatchMode::Steer => Some("steer"),
        DispatchMode::Restart => Some("restart"),
        DispatchMode::Queue => None,
    };
    if let Some(intent) = intent {
        cmd["deliveryIntent"] = intent.into();
    }
    if let Some(m) = model {
        cmd["modelSelection"] = serde_json::to_value(m).unwrap_or(Value::Null);
    }
    cmd
}

pub fn launch_thread(
    project_id: &str,
    text: &str,
    model: &ModelSelection,
    runtime_mode: &str,
) -> Value {
    let title: String = text
        .lines()
        .next()
        .unwrap_or("New thread")
        .chars()
        .take(80)
        .collect();
    json!({
        "commandId": new_id(),
        "creationSource": CREATION_SOURCE,
        "threadId": new_id(),
        "projectId": project_id,
        "title": if title.trim().is_empty() { "New thread".to_owned() } else { title },
        "generateTitle": true,
        "modelSelection": model,
        "runtimeMode": runtime_mode,
        "interactionMode": "default",
        "workspaceStrategy": { "type": "root" },
        "initialMessage": { "messageId": new_id(), "text": text, "attachments": [] },
    })
}

/// Stop: `thread.stop` when the server has the capability, else interrupt the newest active run.
pub fn stop(thread_id: &str, thread_stop: bool, active_run_id: Option<&str>) -> Option<Value> {
    if thread_stop {
        return Some(
            json!({ "type": "thread.stop", "commandId": new_id(), "threadId": thread_id }),
        );
    }
    let run_id = active_run_id?;
    Some(
        json!({ "type": "run.interrupt", "commandId": new_id(), "threadId": thread_id, "runId": run_id, "holdQueue": true }),
    )
}

pub fn respond_decision(thread_id: &str, request_id: &str, decision: &str) -> Value {
    json!({
        "type": "runtime-request.respond",
        "commandId": new_id(),
        "threadId": thread_id,
        "requestId": request_id,
        "decision": decision,
    })
}

pub fn respond_answers(thread_id: &str, request_id: &str, answers: Value) -> Value {
    json!({
        "type": "runtime-request.respond",
        "commandId": new_id(),
        "threadId": thread_id,
        "requestId": request_id,
        "answers": answers,
    })
}

pub fn set_model(thread_id: &str, model: &ModelSelection) -> Value {
    json!({ "type": "thread.model-selection.set", "commandId": new_id(), "threadId": thread_id, "modelSelection": model })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn unknown_shell_items_and_fields_do_not_fail() {
        let item: ShellStreamItem =
            serde_json::from_value(json!({ "kind": "thread.family-updated", "x": 1 })).unwrap();
        assert!(matches!(item, ShellStreamItem::Unknown));
        let item: ShellStreamItem = serde_json::from_value(json!({
            "kind": "thread.updated", "sequence": 7, "location": "active",
            "thread": { "id": "t1", "projectId": "p1", "title": "Hi", "status": "running", "brandNew": true }
        }))
        .unwrap();
        let ShellStreamItem::ThreadUpdated {
            sequence, thread, ..
        } = item
        else {
            panic!()
        };
        assert_eq!(
            (sequence, thread.id.as_str(), thread.is_working()),
            (7, "t1", true)
        );
    }

    #[test]
    fn thread_events_decode_by_type() {
        let item = ThreadStreamItem::decode(json!({
            "kind": "event", "sequence": 12,
            "event": { "id": "e", "threadId": "t1", "type": "turn-item.updated", "occurredAt": "x",
                "payload": { "id": "i1", "threadId": "t1", "type": "assistant_message", "ordinal": 3,
                    "status": "running", "text": "Hel", "streaming": true, "messageId": "m" } }
        }));
        let ThreadStreamItem::Event {
            sequence: 12,
            event: ThreadEvent::TurnItem(item),
            ..
        } = item
        else {
            panic!()
        };
        assert_eq!(
            (
                item.kind.as_str(),
                item.text.as_deref(),
                item.is_streaming()
            ),
            ("assistant_message", Some("Hel"), true)
        );
    }

    #[test]
    fn follow_ups_steer_by_default_and_queue_with_the_modifier() {
        use DispatchMode::*;
        assert_eq!(DispatchMode::resolve(false, true, false, true, false), Auto);
        assert_eq!(
            DispatchMode::resolve(true, false, false, true, false),
            Steer
        );
        assert_eq!(DispatchMode::resolve(true, true, false, true, false), Queue);
        // With follow-ups set to queue, the modifier steers instead.
        assert_eq!(DispatchMode::resolve(true, false, false, true, true), Queue);
        assert_eq!(DispatchMode::resolve(true, true, false, true, true), Steer);
        // A send redirects an agent waiting on approval, so it can't queue behind it.
        assert_eq!(DispatchMode::resolve(true, true, true, true, false), Steer);
        assert_eq!(DispatchMode::resolve(true, false, true, true, true), Steer);
        // The server rejects a steer while the run starts or compacts.
        assert_eq!(
            DispatchMode::resolve(true, false, false, false, false),
            Queue
        );
        let queued = message_dispatch("t", "x", None, Queue);
        assert_eq!(
            (
                queued.get("deliveryIntent"),
                &queued["dispatchMode"]["type"]
            ),
            (None, &json!("queue_after_active"))
        );
    }

    #[test]
    fn an_empty_server_default_theme_is_no_default() {
        let config = |settings| ServerConfig {
            settings,
            ..Default::default()
        };
        assert_eq!(
            config(json!({ "defaultTheme": "", "defaultThemeSetAt": "" })).default_theme(),
            None
        );
        assert_eq!(
            config(
                json!({ "defaultTheme": "omarchy", "defaultThemeSetAt": "2026-11-07T00:00:00Z" })
            )
            .default_theme(),
            Some(("omarchy".into(), "2026-11-07T00:00:00Z".into()))
        );
    }

    #[test]
    fn model_selection_accepts_legacy_provider_key() {
        let m: ModelSelection =
            serde_json::from_value(json!({ "provider": "codex", "model": "gpt" })).unwrap();
        assert_eq!(m.instance_id, "codex");
        assert_eq!(
            serde_json::to_value(&m).unwrap(),
            json!({ "instanceId": "codex", "model": "gpt" })
        );
    }
}
