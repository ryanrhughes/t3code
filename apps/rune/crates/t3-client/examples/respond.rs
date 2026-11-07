//! Answers a pending question on a sandbox thread, as the question card does:
//! `cargo run -p t3-client --example respond -- <thread> <request> '{"<question id>": "<label>"}'`.
//! Sandbox servers only. Uses the saved pairing.
use std::sync::Arc;
use std::time::Duration;

use t3_client::pairing;
use t3_client::protocol;
use t3_client::session::{self, ClientEvent};

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let [thread, request, answers] = args.as_slice() else {
        anyhow::bail!("usage: respond <thread> <request> <answers json>");
    };
    let answers: serde_json::Value = serde_json::from_str(answers)?;
    let cred = pairing::load_credential().ok_or_else(|| anyhow::anyhow!("no saved credential"))?;
    anyhow::ensure!(
        cred.http_base.contains("127.0.0.1:16482"),
        "refusing to answer on {}: sandbox only",
        cred.http_base
    );
    let sink: session::Sink = Arc::new(|ev| {
        if let ClientEvent::CommandFailed(e) = ev {
            eprintln!("failed: {e}");
        }
    });
    let handle = session::spawn(cred, sink);
    std::thread::sleep(Duration::from_secs(2));
    handle.dispatch(protocol::respond_answers(thread, request, answers));
    std::thread::sleep(Duration::from_secs(3));
    println!("answered {request}");
    Ok(())
}
