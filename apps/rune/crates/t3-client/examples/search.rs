//! Runs the composer's `@` file search, `projects.searchEntries`, against the first project:
//! `cargo run -p t3-client --example search -- <query>`. Read-only; uses the saved pairing.
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde_json::json;
use t3_client::composer::file_link;
use t3_client::pairing;
use t3_client::session::{self, ClientEvent};
use t3_client::store::ShellState;

fn main() -> anyhow::Result<()> {
    let query = std::env::args().nth(1).unwrap_or_default();
    let cred = pairing::load_credential().ok_or_else(|| anyhow::anyhow!("no saved credential"))?;
    let shell = Arc::new(Mutex::new(ShellState::default()));
    let s2 = shell.clone();
    let handle = session::spawn(
        cred,
        Arc::new(move |ev| {
            if let ClientEvent::Shell(item) = ev {
                s2.lock().unwrap().apply(item);
            }
        }),
    );
    for _ in 0..100 {
        if shell.lock().unwrap().loaded {
            break;
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    let root = shell
        .lock()
        .unwrap()
        .sidebar_projects()
        .iter()
        .map(|p| p.workspace_root.clone())
        .find(|r| std::path::Path::new(r).is_dir());
    let root = root.ok_or_else(|| anyhow::anyhow!("no project with a workspace on disk"))?;
    println!("searching {root:?} for {query:?}");
    let (tx, rx) = mpsc::channel();
    handle.call(
        "projects.searchEntries",
        json!({ "cwd": root, "query": query, "limit": 8 }),
        move |r| drop(tx.send(r)),
    );
    let result = rx
        .recv_timeout(Duration::from_secs(15))?
        .map_err(anyhow::Error::msg)?;
    for e in result["entries"].as_array().into_iter().flatten() {
        let path = e["path"].as_str().unwrap_or("");
        println!(
            "  {:<9} {path:<50} -> {}",
            e["kind"].as_str().unwrap_or(""),
            file_link(path)
        );
    }
    Ok(())
}
