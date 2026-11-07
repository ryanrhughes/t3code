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
