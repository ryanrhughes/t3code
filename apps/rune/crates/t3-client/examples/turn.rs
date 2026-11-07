//! Drives a real turn on a dev server's free `Replay (dev)` provider through the same session and
//! protocol code the UI uses: launch a thread, steer and queue follow-ups while it runs, accept
//! approvals, answer questions, then print how the timeline landed.
//! `cargo run -p t3-client --example turn -- [showcase|approval|question|multi-turn]`.
//! Sandbox servers only: it starts turns. Uses the saved pairing.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use t3_client::pairing;
use t3_client::protocol::{self, DispatchMode, ModelSelection, ThreadStreamItem};
use t3_client::session::{self, ClientEvent};
use t3_client::store::{ShellState, ThreadState};

#[derive(Default)]
struct Seen {
    shell: ShellState,
    thread: Option<ThreadState>,
    launched: Option<String>,
    failures: Vec<String>,
}

fn main() -> anyhow::Result<()> {
    let scenario = std::env::args().nth(1).unwrap_or_else(|| "showcase".into());
    let cred = pairing::load_credential().ok_or_else(|| anyhow::anyhow!("no saved credential"))?;
    anyhow::ensure!(
        cred.http_base.contains("127.0.0.1:16482")
            || std::env::var_os("T3_TURN_ANY_SERVER").is_some(),
        "refusing to start turns on {}: sandbox only",
        cred.http_base
    );
    let seen = Arc::new(Mutex::new(Seen::default()));
    let s2 = seen.clone();
    let sink: session::Sink = Arc::new(move |ev| {
        let mut s = s2.lock().unwrap();
        match ev {
            ClientEvent::Shell(item) => drop(s.shell.apply(item)),
            ClientEvent::ThreadLaunched { thread_id } => s.launched = Some(thread_id),
            ClientEvent::CommandFailed(e) => s.failures.push(e),
            ClientEvent::Thread { item, .. } => match item {
                ThreadStreamItem::Snapshot {
                    sequence,
                    projection,
                } => s.thread = Some(ThreadState::from_snapshot(sequence, *projection)),
                ThreadStreamItem::Event {
                    sequence,
                    thread_id,
                    event,
                } => {
                    if let Some(t) = s.thread.as_mut() {
                        t.apply(sequence, &thread_id, event);
                    }
                }
                _ => {}
            },
            _ => {}
        }
    });
    let handle = session::spawn(cred, sink);
    let wait = |what: &str, secs: u64, done: &dyn Fn(&Seen) -> bool| -> anyhow::Result<()> {
        let deadline = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < deadline {
            if done(&seen.lock().unwrap()) {
                return Ok(());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        anyhow::bail!("timed out waiting for {what}")
    };

    wait("the shell", 15, &|s| s.shell.loaded)?;
    let project = seen
        .lock()
        .unwrap()
        .shell
        .sidebar_projects()
        .first()
        .map(|p| p.id.clone())
        .ok_or_else(|| anyhow::anyhow!("no project"))?;
    let model = ModelSelection {
        instance_id: "devReplay".into(),
        model: scenario.clone(),
        options: None,
    };
    handle.launch(protocol::launch_thread(
        &project,
        "go",
        &model,
        "approval-required",
    ));
    wait("the launch", 20, &|s| {
        s.launched.is_some() || !s.failures.is_empty()
    })?;
    let id = seen
        .lock()
        .unwrap()
        .launched
        .clone()
        .ok_or_else(|| anyhow::anyhow!("launch failed: {:?}", seen.lock().unwrap().failures))?;
    println!("launched {id} ({scenario})");
    handle.watch_thread(Some(id.clone()));
    wait("the run to start", 20, &|s| {
        s.thread.as_ref().is_some_and(|t| t.active_run().is_some())
    })?;
    let mode = |alternate: bool| {
        let s = seen.lock().unwrap();
        let t = s.thread.as_ref().unwrap();
        DispatchMode::resolve(t.active_run().is_some(), alternate, false, t.can_steer())
    };
    // Sent the instant the run appears, a steer can't land yet; the client queues it instead.
    println!("send while starting -> {:?}", mode(false));
    wait("the run to be steerable", 30, &|s| {
        s.thread.as_ref().is_some_and(|t| t.can_steer())
    })?;
    // While it works: steer once, queue once, as Return and Ctrl+Return do.
    let (steer, queue) = (mode(false), mode(true));
    println!("send while running -> {steer:?}, with Ctrl -> {queue:?}");
    handle.dispatch(protocol::message_dispatch(
        &id,
        "steer: keep going",
        None,
        steer,
    ));
    handle.dispatch(protocol::message_dispatch(
        &id,
        "queued: then summarize",
        None,
        queue,
    ));

    let deadline = Instant::now() + Duration::from_secs(90);
    let mut answered = std::collections::HashSet::new();
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        let s = seen.lock().unwrap();
        let Some(t) = s.thread.as_ref() else { continue };
        for r in t.visible.iter().map(|r| &r.item) {
            let Some(req) = r
                .request_id
                .as_deref()
                .filter(|id| t.pending_request(id).is_some() && !answered.contains(*id))
            else {
                continue;
            };
            match r.kind.as_str() {
                "approval_request" => {
                    println!(
                        "approving {req} ({})",
                        r.request_kind.as_deref().unwrap_or("?")
                    );
                    handle.dispatch(protocol::respond_decision(&id, req, "accept"));
                }
                "user_input_request" => {
                    let answers: serde_json::Map<_, _> = r
                        .questions
                        .iter()
                        .flatten()
                        .map(|q| {
                            let o = &q.options[0];
                            (
                                q.id.clone(),
                                json!(o.value.clone().unwrap_or_else(|| o.label.clone())),
                            )
                        })
                        .collect();
                    println!("answering {req}: {answers:?}");
                    handle.dispatch(protocol::respond_answers(
                        &id,
                        req,
                        serde_json::Value::Object(answers),
                    ));
                }
                _ => continue,
            }
            answered.insert(req.to_owned());
        }
        let queued_left = t.visible.iter().any(|r| {
            r.item.input_intent.as_deref() == Some("queued_turn")
                && !matches!(
                    r.item.status.as_str(),
                    "completed" | "failed" | "cancelled" | "interrupted"
                )
        });
        if !t.is_working() && !queued_left && t.runs.len() >= 2 {
            break;
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    let s = seen.lock().unwrap();
    let t = s.thread.as_ref().unwrap();
    println!(
        "--- {} runs: {:?}",
        t.runs.len(),
        t.runs.iter().map(|r| r.status.as_str()).collect::<Vec<_>>()
    );
    for r in &t.visible {
        let i = &r.item;
        let detail = i
            .text
            .as_deref()
            .or(i.input_text())
            .or(i.prompt.as_deref())
            .unwrap_or("")
            .lines()
            .next()
            .unwrap_or("")
            .chars()
            .take(60)
            .collect::<String>();
        println!(
            "  {:<20} {:<10} {:<20} {detail}",
            i.kind,
            i.status,
            i.input_intent.as_deref().unwrap_or("")
        );
    }
    if !s.failures.is_empty() {
        println!("failures: {:?}", s.failures);
    }
    Ok(())
}
