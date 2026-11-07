//! Pairs with a server and prints what the session streams, for checking the protocol by hand:
//! `cargo run -p t3-client --example probe -- '<pairing link>' [seconds]`
//! With no link it reuses the saved credential.
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use t3_client::protocol::{ServerConfigEvent, ThreadStreamItem};
use t3_client::session::{self, ClientEvent};
use t3_client::store::{ShellState, ThreadState};
use t3_client::{pairing, protocol};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let cred = match args.first().filter(|a| a.contains("://")) {
        Some(link) => session::pair_blocking(link)?,
        None => pairing::load_credential()
            .ok_or_else(|| anyhow::anyhow!("no saved credential; pass a pairing link"))?,
    };
    let secs: u64 = args.iter().find_map(|a| a.parse().ok()).unwrap_or(8);
    println!("paired with {} ({})", cred.label, cred.http_base);

    let shell = Arc::new(Mutex::new(ShellState::default()));
    let thread: Arc<Mutex<Option<ThreadState>>> = Arc::default();
    let started = Instant::now();
    let (s2, t2) = (shell.clone(), thread.clone());
    let sink: session::Sink = Arc::new(move |ev| match ev {
        ClientEvent::Status(s) => println!("[{:>5}ms] status {s:?}", started.elapsed().as_millis()),
        ClientEvent::Config(ServerConfigEvent::Snapshot { config }) => println!(
            "[{:>5}ms] config: env={} providers={:?}",
            started.elapsed().as_millis(),
            config.environment.label,
            config
                .providers
                .iter()
                .map(|p| format!("{}({} models, {})", p.instance_id, p.models.len(), p.status))
                .collect::<Vec<_>>()
        ),
        ClientEvent::Config(ServerConfigEvent::EnvironmentThemesUpdated { payload }) => {
            println!(
                "[{:>5}ms] themes: {:?}",
                started.elapsed().as_millis(),
                payload
                    .themes
                    .iter()
                    .map(|t| t["id"].clone())
                    .collect::<Vec<_>>()
            )
        }
        ClientEvent::Config(_) => {}
        ClientEvent::Shell(item) => {
            let mut s = s2.lock().unwrap();
            let was = s.loaded;
            s.apply(item);
            if s.loaded && !was {
                println!(
                    "[{:>5}ms] shell: {} projects, {} threads (seq {})",
                    started.elapsed().as_millis(),
                    s.projects.len(),
                    s.threads.len(),
                    s.sequence
                );
            }
        }
        ClientEvent::Thread { item, .. } => {
            let mut t = t2.lock().unwrap();
            match item {
                ThreadStreamItem::Snapshot {
                    sequence,
                    projection,
                } => {
                    let st = ThreadState::from_snapshot(sequence, *projection);
                    let kinds: std::collections::BTreeMap<_, usize> =
                        st.visible.iter().fold(Default::default(), |mut m, r| {
                            *m.entry(r.item.kind.clone()).or_default() += 1;
                            m
                        });
                    println!(
                        "[{:>5}ms] thread {:?}: {} visible items {kinds:?}",
                        started.elapsed().as_millis(),
                        st.thread.title,
                        st.visible.len()
                    );
                    *t = Some(st);
                }
                ThreadStreamItem::Event {
                    sequence,
                    thread_id,
                    event,
                } => {
                    if let Some(st) = t.as_mut() {
                        st.apply(sequence, &thread_id, event);
                    }
                }
                ThreadStreamItem::Synchronized => println!(
                    "[{:>5}ms] thread synchronized",
                    started.elapsed().as_millis()
                ),
                ThreadStreamItem::Unknown => {}
            }
        }
        other => println!("[{:>5}ms] {other:?}", started.elapsed().as_millis()),
    });
    let handle = session::spawn(cred, sink);

    // Once the shell lands, open the most recently updated thread.
    let deadline = Instant::now() + Duration::from_secs(secs);
    let mut watched = false;
    while Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(50));
        if !watched {
            let s = shell.lock().unwrap();
            if let Some(t) = s
                .threads
                .iter()
                .filter(|t| t.is_sidebar_row())
                .max_by(|a, b| a.updated_at.cmp(&b.updated_at))
            {
                println!("watching {:?} ({})", t.title, protocol::CREATION_SOURCE);
                handle.watch_thread(Some(t.id.clone()));
                watched = true;
            }
        }
    }
    Ok(())
}
