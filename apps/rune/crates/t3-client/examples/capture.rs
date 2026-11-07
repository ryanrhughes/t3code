//! Records the first frame of each subscription the client uses, as wire JSON, for headless
//! scenes and tests: `cargo run -p t3-client --example capture -- out.json [thread-id]`.
//! Uses the saved credential (pair first with the `probe` example). Read-only: it subscribes and
//! leaves; it never dispatches a command.
use serde_json::{Value, json};
use t3_client::pairing;
use t3_client::rpc::{self, StreamMsg};

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args
        .first()
        .cloned()
        .unwrap_or_else(|| "capture.json".into());
    let cred = pairing::load_credential()
        .ok_or_else(|| anyhow::anyhow!("no saved credential; run the probe example first"))?;
    let ticket = pairing::ws_ticket(&cred).await?;
    let (rpc, actor) = rpc::connect(cred.ws_url(&ticket)?.as_str()).await?;
    tokio::spawn(actor);

    let first = |tag: &str, payload: Value, until: fn(&Value) -> bool| {
        let (_, mut rx) = rpc.stream(tag, payload);
        async move {
            let mut items = Vec::new();
            while let Some(StreamMsg::Chunk(values)) = rx.recv().await {
                let done = values.iter().any(until);
                items.extend(values);
                if done {
                    break;
                }
            }
            items
        }
    };
    let config = first(
        "subscribeServerConfig",
        json!({ "workspaceSnapshotsOnDemand": true, "environmentThemes": true }),
        |v| v["type"] == "environmentThemesUpdated",
    )
    .await;
    let shell = first("orchestration.subscribeShell", json!({}), |v| {
        v["kind"] == "snapshot"
    })
    .await;
    let threads = shell
        .iter()
        .find(|v| v["kind"] == "snapshot")
        .map(|v| v["snapshot"]["threads"].clone())
        .unwrap_or_default();
    let thread_id = args.get(1).cloned().or_else(|| {
        // The busiest settled thread makes the richest fixture.
        threads
            .as_array()?
            .iter()
            .filter(|t| t["lineage"]["relationshipToParent"].is_null())
            .max_by_key(|t| t["visibleItemCount"].as_u64().unwrap_or(0))?["id"]
            .as_str()
            .map(str::to_owned)
    });
    let thread = match &thread_id {
        Some(id) => {
            first(
                "orchestration.subscribeThread",
                json!({ "threadId": id }),
                |v| v["kind"] == "snapshot",
            )
            .await
        }
        None => vec![],
    };
    // The newest turn that changed files, as the diff panel opens it.
    let mut diff = Value::Null;
    if let (Some(id), Some(snapshot)) =
        (&thread_id, thread.iter().find(|v| v["kind"] == "snapshot"))
    {
        let turn = snapshot["projection"]["checkpoints"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|c| c["files"].as_array().is_some_and(|f| !f.is_empty()))
            .filter_map(|c| c["appRunOrdinal"].as_u64())
            .max();
        if let Some(to) = turn {
            let from = to - 1;
            let (tag, payload) = if from == 0 {
                (
                    "orchestration.getFullThreadDiff",
                    json!({ "threadId": id, "toTurnCount": to }),
                )
            } else {
                (
                    "orchestration.getTurnDiff",
                    json!({ "threadId": id, "fromTurnCount": from, "toTurnCount": to }),
                )
            };
            match rpc.call(tag, payload).await {
                Ok(v) => diff = json!({ "from": from, "to": to, "patch": v["diff"] }),
                Err(e) => println!("diff failed: {e}"),
            }
        }
    }
    std::fs::write(
        &out,
        serde_json::to_vec(
            &json!({ "config": config, "shell": shell, "thread": thread, "diff": diff }),
        )?,
    )?;
    println!(
        "wrote {out}: {} config, {} shell, {} thread items (thread {:?})",
        config.len(),
        shell.len(),
        thread.len(),
        thread_id
    );
    Ok(())
}
