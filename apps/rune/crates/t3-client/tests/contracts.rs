//! The client's hand-built wire values against the types generated from `packages/contracts`:
//! when upstream changes a command or stream item, these fail instead of the server rejecting it.
use serde_json::{Value, json};
use t3_client::protocol::{self, DispatchMode, ModelSelection};
use t3_contracts as c;

fn decodes<T: serde::de::DeserializeOwned>(what: &str, v: Value) {
    if let Err(e) = serde_path_to_error::deserialize::<_, T>(v) {
        panic!(
            "{what} no longer matches the contract at `{}`: {}",
            e.path(),
            e.inner()
        );
    }
}

#[test]
fn every_command_the_client_sends_matches_the_contract() {
    let model = ModelSelection {
        instance_id: "codex".into(),
        model: "gpt-6-astra".into(),
        options: None,
    };
    type Dispatch = c::OrchestrationDispatchCommandPayload;
    decodes::<Dispatch>(
        "message.dispatch",
        protocol::message_dispatch("t1", "hi", Some(&model), DispatchMode::Auto),
    );
    decodes::<Dispatch>(
        "message.dispatch (steer)",
        protocol::message_dispatch("t1", "hi", None, DispatchMode::Steer),
    );
    for mode in [DispatchMode::Queue, DispatchMode::Restart] {
        decodes::<Dispatch>(
            "message.dispatch (queue, restart)",
            protocol::message_dispatch("t1", "hi", None, mode),
        );
    }
    decodes::<Dispatch>("thread.stop", protocol::stop("t1", true, None).unwrap());
    decodes::<Dispatch>(
        "run.interrupt",
        protocol::stop("t1", false, Some("r1")).unwrap(),
    );
    decodes::<Dispatch>(
        "runtime-request.respond",
        protocol::respond_decision("t1", "q1", "accept"),
    );
    decodes::<Dispatch>(
        "runtime-request.respond (answers)",
        protocol::respond_answers("t1", "q1", json!({ "a": "b" })),
    );
    decodes::<Dispatch>(
        "thread.model-selection.set",
        protocol::set_model("t1", &model),
    );
    decodes::<c::OrchestrationLaunchThreadPayload>(
        "launchThread",
        protocol::launch_thread("p1", "hi", &model, "full-access"),
    );
    let files = [
        json!({ "type": "image", "id": "a1", "name": "shot.png", "mimeType": "image/png", "sizeBytes": 10 }),
        json!({ "type": "file", "id": "a2", "name": "notes.md", "mimeType": "text/markdown", "sizeBytes": 3 }),
    ];
    let mut send = protocol::message_dispatch("t1", "look", None, DispatchMode::Auto);
    protocol::attach(&mut send, &files);
    decodes::<Dispatch>("message.dispatch (attachments)", send);
    let mut launch = protocol::launch_thread("p1", "look", &model, "full-access");
    protocol::attach(&mut launch, &files);
    decodes::<c::OrchestrationLaunchThreadPayload>("launchThread (attachments)", launch);
    let tuned = model.with_option("reasoningEffort", json!("high"));
    decodes::<Dispatch>(
        "message.dispatch (model options)",
        protocol::message_dispatch("t1", "hi", Some(&tuned), DispatchMode::Auto),
    );
    decodes::<c::AttachmentsCreateUploadUrlPayload>(
        "attachments.createUploadUrl",
        json!({ "type": "image", "name": "shot.png", "mimeType": "image/png", "sizeBytes": 10 }),
    );
    let mut implement = protocol::message_dispatch(
        "t1",
        "PLEASE IMPLEMENT THIS PLAN:\nx",
        None,
        DispatchMode::Auto,
    );
    protocol::with_plan(&mut implement, "t1", "plan-1");
    decodes::<Dispatch>("message.dispatch (implement a plan)", implement);
    let mut planned = protocol::launch_thread("p1", "plan it", &model, "full-access");
    planned["interactionMode"] = json!("plan");
    decodes::<c::OrchestrationLaunchThreadPayload>("launchThread (plan mode)", planned);
    let mut worktree = protocol::launch_thread("p1", "work", &model, "full-access");
    worktree["workspaceStrategy"] = json!({ "type": "worktree", "baseRef": "main" });
    decodes::<c::OrchestrationLaunchThreadPayload>("launchThread (new worktree)", worktree);
    decodes::<c::VcsListRefsPayload>(
        "vcs.listRefs",
        json!({ "cwd": "/repo", "refKind": "local", "limit": 100 }),
    );
    // The sidebar's thread actions.
    for (kind, extra) in [
        ("thread.pin", json!({})),
        ("thread.unpin", json!({})),
        ("thread.settle", json!({})),
        ("thread.unsettle", json!({ "reason": "user" })),
        (
            "thread.snooze",
            json!({ "snoozedUntil": "2026-11-07T18:00:00.000Z" }),
        ),
        ("thread.unsnooze", json!({ "reason": "user" })),
        ("thread.mark-unread", json!({})),
        ("thread.archive", json!({})),
        ("thread.unarchive", json!({})),
        ("thread.metadata.update", json!({ "title": "New title" })),
        (
            "thread.visit",
            json!({ "visitedAt": "2026-11-07T18:00:00.000Z", "opened": true }),
        ),
        (
            "thread.interaction-mode.set",
            json!({ "interactionMode": "plan" }),
        ),
        ("queued-run.start", json!({ "runId": "r2" })),
        ("queued-run.cancel", json!({ "runId": "r2" })),
        (
            "queued-run.edit",
            json!({ "runId": "r2", "text": "reworded" }),
        ),
        (
            "queued-message.promote-to-steer",
            json!({ "queuedRunId": "r2", "targetRunId": "r1" }),
        ),
        ("queue.resume", json!({})),
    ] {
        decodes::<Dispatch>(kind, protocol::thread_command(kind, "t1", extra));
    }
}

#[test]
fn every_subscription_payload_matches_the_contract() {
    decodes::<c::SubscribeServerConfigPayload>(
        "subscribeServerConfig",
        json!({ "workspaceSnapshotsOnDemand": true, "environmentThemes": true }),
    );
    decodes::<c::OrchestrationSubscribeShellPayload>(
        "subscribeShell",
        json!({ "requestCompletionMarker": true }),
    );
    decodes::<c::OrchestrationSubscribeThreadPayload>(
        "subscribeThread",
        json!({ "threadId": "t1", "requestCompletionMarker": true }),
    );
    let item = json!({ "threadId": "t1", "itemId": "i1" });
    decodes::<c::OrchestrationSubscribeCommandOutputPayload>(
        "subscribeCommandOutput",
        item.clone(),
    );
    decodes::<c::OrchestrationGetTurnItemPayload>("getTurnItem", item);
    decodes::<c::OrchestrationGetTurnDiffPayload>(
        "getTurnDiff",
        json!({ "threadId": "t1", "fromTurnCount": 2, "toTurnCount": 3 }),
    );
    decodes::<c::OrchestrationGetFullThreadDiffPayload>(
        "getFullThreadDiff",
        json!({ "threadId": "t1", "toTurnCount": 3 }),
    );
    decodes::<c::ProjectsSearchEntriesPayload>(
        "projects.searchEntries",
        json!({ "cwd": "/repo", "query": "src/ma", "limit": 20 }),
    );
}

/// Real traffic, when a capture is at hand (`cargo run -p t3-client --example capture`). It holds
/// a user's threads, so it's never committed: `T3_WIRE_FIXTURE=/tmp/fixture.json cargo test`.
#[test]
fn captured_traffic_decodes_into_the_contract_types() {
    let Some(path) = std::env::var_os("T3_WIRE_FIXTURE") else {
        return;
    };
    let fixture: Value = serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap();
    let items = |k: &str| fixture[k].as_array().cloned().unwrap_or_default();
    for v in items("config") {
        decodes::<c::SubscribeServerConfigItem>("server config item", v);
    }
    for v in items("shell") {
        decodes::<c::OrchestrationSubscribeShellItem>("shell item", v);
    }
    for v in items("thread") {
        decodes::<c::OrchestrationSubscribeThreadItem>("thread item", v);
    }
}
