//! One environment connection: a background thread that keeps the socket up and subscribed,
//! and hands decoded events to the UI through a sink. JSON decoding happens here, off the UI
//! thread; reducers run on the UI thread.
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::pairing::{self, Credential, Unauthorized};
use crate::protocol::{ServerConfigEvent, ShellStreamItem, ThreadStreamItem};
use crate::rpc::{self, Rpc, StreamMsg};

#[derive(Clone, Debug)]
pub enum Status {
    Connecting,
    Connected,
    /// Lost the connection; retrying after `retry_in`.
    Reconnecting {
        reason: String,
        retry_in: Duration,
    },
}

#[derive(Debug)]
pub enum ClientEvent {
    Status(Status),
    Config(ServerConfigEvent),
    Shell(ShellStreamItem),
    Thread {
        thread_id: String,
        item: ThreadStreamItem,
    },
    ThreadLaunched {
        thread_id: String,
    },
    /// A command the user sent failed; show it, don't retry it.
    CommandFailed(String),
    /// The saved credential was refused. The session stops; pair again.
    Unauthorized(String),
}

pub type Sink = Arc<dyn Fn(ClientEvent) + Send + Sync>;

enum SessionCmd {
    WatchThread(Option<String>),
    Dispatch(Value),
    Launch(Value),
    Shutdown,
}

/// Controls a running session. Dropping it shuts the session down.
pub struct SessionHandle {
    tx: mpsc::UnboundedSender<SessionCmd>,
}

impl SessionHandle {
    /// Follow one thread's timeline (or none). Replaces the previous subscription.
    pub fn watch_thread(&self, id: Option<String>) {
        let _ = self.tx.send(SessionCmd::WatchThread(id));
    }

    /// `orchestration.dispatchCommand` with a command built in `protocol`.
    pub fn dispatch(&self, command: Value) {
        let _ = self.tx.send(SessionCmd::Dispatch(command));
    }

    /// `orchestration.launchThread`; success arrives as `ClientEvent::ThreadLaunched`.
    pub fn launch(&self, input: Value) {
        let _ = self.tx.send(SessionCmd::Launch(input));
    }
}

impl Drop for SessionHandle {
    fn drop(&mut self) {
        let _ = self.tx.send(SessionCmd::Shutdown);
    }
}

pub fn spawn(cred: Credential, sink: Sink) -> SessionHandle {
    let (tx, rx) = mpsc::unbounded_channel();
    std::thread::Builder::new()
        .name("t3-session".into())
        .spawn(move || {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("tokio runtime");
            rt.block_on(run(cred, sink, rx));
        })
        .expect("session thread");
    SessionHandle { tx }
}

/// Pairs with a link and saves the credential. Blocking; call it off the UI thread.
pub fn pair_blocking(link: &str) -> anyhow::Result<Credential> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let link = pairing::parse_pairing_link(link)?;
        let cred = pairing::pair(&link).await?;
        pairing::save_credential(&cred)?;
        Ok(cred)
    })
}

async fn run(cred: Credential, sink: Sink, mut cmds: mpsc::UnboundedReceiver<SessionCmd>) {
    let mut watched: Option<String> = None;
    let mut backoff = Duration::from_millis(500);
    loop {
        sink(ClientEvent::Status(Status::Connecting));
        let reason = match connected(&cred, &sink, &mut cmds, &mut watched).await {
            Ok(Exit::Shutdown) => return,
            Ok(Exit::Lost(reason)) => {
                backoff = Duration::from_millis(500);
                reason
            }
            Err(e) if e.is::<Unauthorized>() => {
                sink(ClientEvent::Unauthorized(e.to_string()));
                return;
            }
            Err(e) => e.to_string(),
        };
        sink(ClientEvent::Status(Status::Reconnecting {
            reason,
            retry_in: backoff,
        }));
        // Keep taking commands while waiting so the UI's view of the session stays current.
        let wait = tokio::time::sleep(backoff);
        tokio::pin!(wait);
        loop {
            tokio::select! {
                _ = &mut wait => break,
                cmd = cmds.recv() => match cmd {
                    None | Some(SessionCmd::Shutdown) => return,
                    Some(SessionCmd::WatchThread(id)) => watched = id,
                    Some(SessionCmd::Dispatch(_) | SessionCmd::Launch(_)) => sink(ClientEvent::CommandFailed("Not connected to the server".into())),
                },
            }
        }
        backoff = (backoff * 2).min(Duration::from_secs(15));
    }
}

enum Exit {
    Shutdown,
    Lost(String),
}

async fn connected(
    cred: &Credential,
    sink: &Sink,
    cmds: &mut mpsc::UnboundedReceiver<SessionCmd>,
    watched: &mut Option<String>,
) -> anyhow::Result<Exit> {
    let ticket = pairing::ws_ticket(cred).await?;
    let (rpc, actor) = rpc::connect(cred.ws_url(&ticket)?.as_str()).await?;
    let actor = tokio::spawn(actor);
    let ended = async move { actor.await.unwrap_or_else(|e| e.to_string()) };
    tokio::pin!(ended);

    // The config stream's first snapshot is the "welcome"; its environment must be the one we paired with.
    let (_, mut config) = rpc.stream(
        "subscribeServerConfig",
        json!({ "workspaceSnapshotsOnDemand": true, "environmentThemes": true }),
    );
    let first = tokio::select! {
        m = config.recv() => m,
        reason = &mut ended => return Ok(Exit::Lost(reason)),
    };
    match first {
        Some(StreamMsg::Chunk(values)) => {
            for v in values {
                if let Ok(ev) = serde_json::from_value::<ServerConfigEvent>(v) {
                    if let ServerConfigEvent::Snapshot { config } = &ev {
                        if config.environment.environment_id != cred.environment_id {
                            anyhow::bail!(
                                "This server is a different environment than the one you paired with"
                            );
                        }
                    }
                    sink(ClientEvent::Config(ev));
                }
            }
        }
        Some(StreamMsg::End(Err(e))) => return Err(e.into()),
        _ => return Ok(Exit::Lost("the config stream ended".into())),
    }
    sink(ClientEvent::Status(Status::Connected));

    // Without `afterSequence` the server sends a full snapshot; reconnects resync the same way.
    let (_, mut shell) = rpc.stream(
        "orchestration.subscribeShell",
        json!({ "requestCompletionMarker": true }),
    );
    let mut thread = watched.clone().map(|id| subscribe_thread(&rpc, id));

    loop {
        tokio::select! {
            reason = &mut ended => return Ok(Exit::Lost(reason)),
            m = config.recv() => match m {
                Some(StreamMsg::Chunk(values)) => for v in values {
                    if let Ok(ev) = serde_json::from_value::<ServerConfigEvent>(v) { sink(ClientEvent::Config(ev)); }
                },
                _ => return Ok(Exit::Lost("the config stream ended".into())),
            },
            m = shell.recv() => match m {
                Some(StreamMsg::Chunk(values)) => for v in values {
                    match serde_json::from_value::<ShellStreamItem>(v) {
                        Ok(item) => sink(ClientEvent::Shell(item)),
                        Err(e) => eprintln!("t3-rune: skipped a shell item: {e}"),
                    }
                },
                _ => return Ok(Exit::Lost("the shell stream ended".into())),
            },
            m = recv_thread(&mut thread) => {
                let (id, msg) = m;
                match msg {
                    Some(StreamMsg::Chunk(values)) => for v in values {
                        sink(ClientEvent::Thread { thread_id: id.clone(), item: ThreadStreamItem::decode(v) });
                    },
                    Some(StreamMsg::End(Err(e))) if !matches!(e, rpc::RpcError::Interrupted) => {
                        if matches!(e, rpc::RpcError::Disconnected) { return Ok(Exit::Lost("disconnected".into())); }
                        sink(ClientEvent::CommandFailed(format!("Couldn't open the thread: {}", e.message())));
                        thread = None;
                    }
                    _ => thread = None,
                }
            }
            cmd = cmds.recv() => match cmd {
                None | Some(SessionCmd::Shutdown) => return Ok(Exit::Shutdown),
                Some(SessionCmd::WatchThread(id)) => {
                    if id.as_ref() != thread.as_ref().map(|t| &t.0) {
                        if let Some((_, rid, _)) = thread.take() { rpc.interrupt(rid); }
                        thread = id.clone().map(|id| subscribe_thread(&rpc, id));
                    }
                    *watched = id;
                }
                Some(SessionCmd::Dispatch(command)) => {
                    let (rpc, sink) = (rpc.clone(), sink.clone());
                    tokio::spawn(async move {
                        if let Err(e) = rpc.call("orchestration.dispatchCommand", command).await {
                            sink(ClientEvent::CommandFailed(e.message()));
                        }
                    });
                }
                Some(SessionCmd::Launch(input)) => {
                    let (rpc, sink) = (rpc.clone(), sink.clone());
                    let thread_id = input.get("threadId").and_then(Value::as_str).unwrap_or_default().to_owned();
                    tokio::spawn(async move {
                        match rpc.call("orchestration.launchThread", input).await {
                            Ok(_) => sink(ClientEvent::ThreadLaunched { thread_id }),
                            Err(e) => sink(ClientEvent::CommandFailed(e.message())),
                        }
                    });
                }
            },
        }
    }
}

type ThreadSub = (String, u64, mpsc::UnboundedReceiver<StreamMsg>);

fn subscribe_thread(rpc: &Rpc, id: String) -> ThreadSub {
    let (rid, rx) = rpc.stream(
        "orchestration.subscribeThread",
        json!({ "threadId": id, "requestCompletionMarker": true }),
    );
    (id, rid, rx)
}

async fn recv_thread(sub: &mut Option<ThreadSub>) -> (String, Option<StreamMsg>) {
    match sub {
        Some((id, _, rx)) => (id.clone(), rx.recv().await),
        None => std::future::pending().await,
    }
}
