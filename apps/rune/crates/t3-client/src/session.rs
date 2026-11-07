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

/// Runs on the session thread with a unary RPC's result; error text is user-facing.
pub type Reply = Box<dyn FnOnce(Result<Value, String>) + Send>;
/// Runs on the session thread with each streamed item, then once with `None` (ended) or the
/// reason it failed. A stream that ends because the connection dropped also reports here.
pub type OnItem = Box<dyn FnMut(Option<Result<Value, String>>) + Send>;

enum SessionCmd {
    WatchThread(Option<String>),
    Dispatch(Value),
    Launch(Value),
    Call(String, Value, Reply),
    Subscribe(u64, String, Value, OnItem),
    Unsubscribe(u64),
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

    /// Any unary RPC. `reply` runs on the session thread; post to the UI from it.
    pub fn call(
        &self,
        tag: &str,
        payload: Value,
        reply: impl FnOnce(Result<Value, String>) + Send + 'static,
    ) {
        if let Err(mpsc::error::SendError(SessionCmd::Call(_, _, reply))) = self
            .tx
            .send(SessionCmd::Call(tag.into(), payload, Box::new(reply)))
        {
            reply(Err("Not connected to the server".into()));
        }
    }

    /// Any streaming RPC, kept open until `unsubscribe(key)`. `key` is the caller's, unique
    /// among its open subscriptions; reusing one replaces the earlier subscription.
    pub fn subscribe(
        &self,
        key: u64,
        tag: &str,
        payload: Value,
        on_item: impl FnMut(Option<Result<Value, String>>) + Send + 'static,
    ) {
        let _ = self.tx.send(SessionCmd::Subscribe(
            key,
            tag.into(),
            payload,
            Box::new(on_item),
        ));
    }

    pub fn unsubscribe(&self, key: u64) {
        let _ = self.tx.send(SessionCmd::Unsubscribe(key));
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

/// Connects to the server running on this machine for this user and saves the credential.
/// Blocking; call it off the UI thread.
pub fn pair_local_blocking(home: &std::path::Path) -> anyhow::Result<Credential> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()?;
    rt.block_on(async {
        let cred = pairing::pair_local(home).await?;
        pairing::save_credential(&cred)?;
        Ok(cred)
    })
}

/// Subscriptions the UI holds open, by its key; reopened on every connect.
type Subs = std::collections::HashMap<u64, (String, Value, Arc<std::sync::Mutex<OnItem>>)>;

async fn run(cred: Credential, sink: Sink, mut cmds: mpsc::UnboundedReceiver<SessionCmd>) {
    let mut watched: Option<String> = None;
    let mut subs = Subs::new();
    let mut backoff = Duration::from_millis(500);
    loop {
        sink(ClientEvent::Status(Status::Connecting));
        let reason = match connected(&cred, &sink, &mut cmds, &mut watched, &mut subs).await {
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
                    Some(SessionCmd::Call(_, _, reply)) => reply(Err("Not connected to the server".into())),
                    Some(SessionCmd::Subscribe(key, tag, payload, on_item)) => {
                        subs.insert(key, (tag, payload, Arc::new(std::sync::Mutex::new(on_item))));
                    }
                    Some(SessionCmd::Unsubscribe(key)) => {
                        subs.remove(&key);
                    }
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
    subs: &mut Subs,
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
    let mut live: std::collections::HashMap<u64, tokio::sync::oneshot::Sender<()>> = subs
        .iter()
        .map(|(key, (tag, payload, on_item))| {
            (*key, open_sub(&rpc, tag, payload.clone(), on_item.clone()))
        })
        .collect();

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
                Some(SessionCmd::Call(tag, payload, reply)) => {
                    let rpc = rpc.clone();
                    tokio::spawn(async move { reply(rpc.call(&tag, payload).await.map_err(|e| e.message())) });
                }
                Some(SessionCmd::Subscribe(key, tag, payload, on_item)) => {
                    let on_item = Arc::new(std::sync::Mutex::new(on_item));
                    // Dropping the old cancel sender ends a replaced subscription.
                    live.insert(key, open_sub(&rpc, &tag, payload.clone(), on_item.clone()));
                    subs.insert(key, (tag, payload, on_item));
                }
                Some(SessionCmd::Unsubscribe(key)) => {
                    subs.remove(&key);
                    live.remove(&key);
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

/// Forwards one subscription to its callback until the stream ends or the returned sender is
/// dropped (then the server is told to stop). A dropped connection ends it quietly; the session
/// reopens it on the next connect.
fn open_sub(
    rpc: &Rpc,
    tag: &str,
    payload: Value,
    on_item: Arc<std::sync::Mutex<OnItem>>,
) -> tokio::sync::oneshot::Sender<()> {
    let (cancel, mut cancelled) = tokio::sync::oneshot::channel::<()>();
    let (id, mut rx) = rpc.stream(tag, payload);
    let rpc = rpc.clone();
    tokio::spawn(async move {
        let emit = |item: Option<Result<Value, String>>| {
            if let Ok(mut f) = on_item.lock() {
                f(item);
            }
        };
        loop {
            tokio::select! {
                _ = &mut cancelled => {
                    rpc.interrupt(id);
                    return;
                }
                m = rx.recv() => match m {
                    Some(StreamMsg::Chunk(values)) => values.into_iter().for_each(|v| emit(Some(Ok(v)))),
                    Some(StreamMsg::End(Ok(()))) => return emit(None),
                    Some(StreamMsg::End(Err(rpc::RpcError::Disconnected))) | None => return,
                    Some(StreamMsg::End(Err(e))) => return emit(Some(Err(e.message()))),
                },
            }
        }
    });
    cancel
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
