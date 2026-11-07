//! Streams one command's output through `SessionHandle::subscribe`, the path the UI's expanded
//! command rows use: `cargo run -p t3-client --example output -- [thread-id]`. Read-only; uses
//! the saved pairing. Without a thread id it picks the newest thread that ran a command.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;
use t3_client::pairing;
use t3_client::protocol::ThreadStreamItem;
use t3_client::session::{self, ClientEvent};
use t3_client::store::{ShellState, ThreadState};
use t3_client::terminal::{TerminalOutput, plain_text};

fn main() -> anyhow::Result<()> {
    let cred = pairing::load_credential()
        .ok_or_else(|| anyhow::anyhow!("no saved credential; run the probe example first"))?;
    let wanted = std::env::args().nth(1);
    let shell = Arc::new(Mutex::new(ShellState::default()));
    let thread: Arc<Mutex<Option<ThreadState>>> = Arc::default();
    let (s2, t2) = (shell.clone(), thread.clone());
    let sink: session::Sink = Arc::new(move |ev| match ev {
        ClientEvent::Shell(item) => {
            s2.lock().unwrap().apply(item);
        }
        ClientEvent::Thread {
            item:
                ThreadStreamItem::Snapshot {
                    sequence,
                    projection,
                },
            ..
        } => {
            *t2.lock().unwrap() = Some(ThreadState::from_snapshot(sequence, *projection));
        }
        _ => {}
    });
    let handle = session::spawn(cred, sink);

    let deadline = Instant::now() + Duration::from_secs(20);
    let (mut watched, mut subscribed) = (false, false);
    let out = Arc::new(Mutex::new((TerminalOutput::default(), 0usize, false)));
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(100));
        if !watched {
            let s = shell.lock().unwrap();
            let id = wanted.clone().or_else(|| {
                s.threads
                    .iter()
                    .filter(|t| t.is_sidebar_row())
                    .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
                    .map(|t| t.id.clone())
            });
            if let Some(id) = id.filter(|_| s.loaded) {
                println!("watching {id}");
                handle.watch_thread(Some(id));
                watched = true;
            }
            continue;
        }
        if subscribed {
            if out.lock().unwrap().2 {
                break;
            }
            continue;
        }
        let t = thread.lock().unwrap();
        let Some(t) = t.as_ref() else { continue };
        let Some(cmd) = t
            .visible
            .iter()
            .rev()
            .map(|r| &r.item)
            .find(|i| i.kind == "command_execution")
        else {
            println!("no command in this thread");
            return Ok(());
        };
        println!(
            "command {:?}",
            cmd.input_text().unwrap_or("").lines().next().unwrap_or("")
        );
        let (o, id) = (out.clone(), cmd.id.clone());
        handle.subscribe(
            1,
            "orchestration.subscribeCommandOutput",
            json!({ "threadId": t.thread.id, "itemId": id }),
            move |frame| {
                let mut o = o.lock().unwrap();
                match frame {
                    Some(Ok(f)) => {
                        o.1 += 1;
                        println!(
                            "frame {}: kind={} running={} truncated={} {} chars",
                            o.1,
                            f["kind"],
                            f["running"],
                            f["truncated"],
                            f["text"].as_str().unwrap_or("").chars().count()
                        );
                        o.0.apply_frame(
                            f["kind"] == "replace",
                            f["text"].as_str().unwrap_or(""),
                            f["truncated"] == true,
                        );
                        if f["running"] == false {
                            o.2 = true;
                        }
                    }
                    Some(Err(e)) => {
                        println!("stream failed: {e}");
                        o.2 = true;
                    }
                    None => {
                        println!("stream ended");
                        o.2 = true;
                    }
                }
            },
        );
        subscribed = true;
    }
    handle.unsubscribe(1);
    let o = out.lock().unwrap();
    let plain = plain_text(&o.0.text);
    println!(
        "--- {} lines, truncated={}; last lines:",
        plain.lines().count(),
        o.0.truncated
    );
    for l in plain
        .lines()
        .rev()
        .take(6)
        .collect::<Vec<_>>()
        .into_iter()
        .rev()
    {
        println!("  {}", l.chars().take(110).collect::<String>());
    }
    Ok(())
}
