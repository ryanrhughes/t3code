//! Effect RPC over one WebSocket, JSON serialization (`RpcSerialization.layerJson`).
//!
//! Wire rules (`.repos/effect/packages/effect/src/rpc/RpcMessage.ts`, `RpcServer.ts`):
//! - Client → server: `Request {id, tag, payload, headers: []}`, `Ack {requestId}` once per
//!   received `Chunk` frame, `Interrupt {requestId}`, `Ping`. Ids are numbers echoed back
//!   unchanged and unique while in flight.
//! - Server → client: one message per frame — `Chunk {requestId, values}`, `Exit {requestId, exit}`,
//!   `Defect` (socket-level), `Pong`. A stream ends with `Exit Success value:null`.
//! - With `?frameParts=1` frames over ~48 KiB arrive split; every part but the last starts with
//!   U+001E. Any frame counts as liveness; 15 s of silence drops the socket.
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use tokio::sync::{mpsc, oneshot};
use tokio::time::{Instant, interval};
use tokio_tungstenite::tungstenite::Message;

const PING_EVERY: Duration = Duration::from_secs(5);
const LIVENESS: Duration = Duration::from_secs(15);
const PART_PREFIX: char = '\u{1e}';

#[derive(Clone, Debug)]
pub enum RpcError {
    /// The handler failed; `cause` is the encoded Cause (an array of reasons).
    Failure(Value),
    Interrupted,
    Disconnected,
}

impl RpcError {
    /// The most readable message in the cause: a typed error's `message`, else a defect's.
    pub fn message(&self) -> String {
        match self {
            RpcError::Interrupted => "Interrupted".into(),
            RpcError::Disconnected => "Disconnected from the server".into(),
            RpcError::Failure(cause) => {
                let reasons = cause.as_array().cloned().unwrap_or_default();
                for r in &reasons {
                    let inner = r.get("error").or_else(|| r.get("defect"));
                    if let Some(m) = inner
                        .and_then(|e| e.get("message").or_else(|| e.get("detail")))
                        .and_then(Value::as_str)
                    {
                        return m.to_owned();
                    }
                    if let Some(s) = inner.and_then(Value::as_str) {
                        return s.to_owned();
                    }
                }
                "The server rejected the request".into()
            }
        }
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message())
    }
}

impl std::error::Error for RpcError {}

#[derive(Debug)]
pub enum StreamMsg {
    Chunk(Vec<Value>),
    End(Result<(), RpcError>),
}

enum Cmd {
    Call {
        id: u64,
        tag: String,
        payload: Value,
        reply: oneshot::Sender<Result<Value, RpcError>>,
    },
    Stream {
        id: u64,
        tag: String,
        payload: Value,
        sink: mpsc::UnboundedSender<StreamMsg>,
    },
    Interrupt {
        id: u64,
    },
}

/// A handle to a live connection. Cheap to clone; every method is non-blocking.
#[derive(Clone)]
pub struct Rpc {
    tx: mpsc::UnboundedSender<Cmd>,
    next: Arc<AtomicU64>,
}

impl Rpc {
    pub async fn call(&self, tag: &str, payload: Value) -> Result<Value, RpcError> {
        let (reply, rx) = oneshot::channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.tx
            .send(Cmd::Call {
                id,
                tag: tag.into(),
                payload,
                reply,
            })
            .map_err(|_| RpcError::Disconnected)?;
        rx.await.unwrap_or(Err(RpcError::Disconnected))
    }

    /// Starts a streaming RPC. Chunks are acked as they're forwarded; the receiver ends with
    /// `StreamMsg::End`.
    pub fn stream(&self, tag: &str, payload: Value) -> (u64, mpsc::UnboundedReceiver<StreamMsg>) {
        let (sink, rx) = mpsc::unbounded_channel();
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        if let Err(mpsc::error::SendError(Cmd::Stream { sink, .. })) = self.tx.send(Cmd::Stream {
            id,
            tag: tag.into(),
            payload,
            sink,
        }) {
            let _ = sink.send(StreamMsg::End(Err(RpcError::Disconnected)));
        }
        (id, rx)
    }

    pub fn interrupt(&self, id: u64) {
        let _ = self.tx.send(Cmd::Interrupt { id });
    }

    pub fn is_closed(&self) -> bool {
        self.tx.is_closed()
    }
}

enum Pending {
    Call(oneshot::Sender<Result<Value, RpcError>>),
    Stream(mpsc::UnboundedSender<StreamMsg>),
}

pub type Socket =
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>;

/// Opens `url` and returns a handle plus the actor future, which resolves with the reason the
/// connection ended. Every pending call and stream fails with `Disconnected` when it does.
pub async fn connect(
    url: &str,
) -> anyhow::Result<(
    Rpc,
    impl std::future::Future<Output = String> + Send + use<>,
)> {
    let mut config = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default();
    config.max_message_size = Some(256 << 20);
    config.max_frame_size = Some(64 << 20);
    let (ws, _) = tokio_tungstenite::connect_async_with_config(url, Some(config), false)
        .await
        .map_err(describe_connect_error)?;
    let (tx, rx) = mpsc::unbounded_channel();
    let rpc = Rpc {
        tx,
        next: Arc::new(AtomicU64::new(1)),
    };
    Ok((rpc, run(ws, rx)))
}

fn describe_connect_error(e: tokio_tungstenite::tungstenite::Error) -> anyhow::Error {
    use tokio_tungstenite::tungstenite::Error;
    match e {
        Error::Http(res) => {
            let body = res
                .body()
                .as_ref()
                .map(|b| String::from_utf8_lossy(b).into_owned())
                .unwrap_or_default();
            let msg = serde_json::from_str::<Value>(&body)
                .ok()
                .and_then(|v| {
                    v.get("message")
                        .or_else(|| v.get("reason"))
                        .and_then(Value::as_str)
                        .map(str::to_owned)
                })
                .unwrap_or_else(|| res.status().to_string());
            if res.status() == 401 {
                return crate::pairing::Unauthorized(msg).into();
            }
            anyhow::anyhow!("The server refused the connection: {msg}")
        }
        other => anyhow::anyhow!("Couldn't open the connection: {other}"),
    }
}

async fn run(ws: Socket, mut rx: mpsc::UnboundedReceiver<Cmd>) -> String {
    let (mut write, mut read) = ws.split();
    let mut pending: HashMap<u64, Pending> = HashMap::new();
    let mut ping = interval(PING_EVERY);
    let mut last_frame = Instant::now();
    let mut parts = String::new();
    let send = |v: Value| Message::text(v.to_string());

    let reason = loop {
        tokio::select! {
            cmd = rx.recv() => {
                let Some(cmd) = cmd else { break "closed by the client".to_owned() };
                let frame = match cmd {
                    Cmd::Call { id, tag, payload, reply } => {
                        pending.insert(id, Pending::Call(reply));
                        json!({ "_tag": "Request", "id": id, "tag": tag, "payload": payload, "headers": [] })
                    }
                    Cmd::Stream { id, tag, payload, sink } => {
                        pending.insert(id, Pending::Stream(sink));
                        json!({ "_tag": "Request", "id": id, "tag": tag, "payload": payload, "headers": [] })
                    }
                    Cmd::Interrupt { id } => {
                        if !pending.contains_key(&id) { continue; }
                        json!({ "_tag": "Interrupt", "requestId": id })
                    }
                };
                if let Err(e) = write.send(send(frame)).await { break format!("send failed: {e}"); }
            }
            msg = read.next() => {
                let text = match msg {
                    None => break "the server closed the connection".to_owned(),
                    Some(Err(e)) => break format!("connection error: {e}"),
                    Some(Ok(Message::Text(t))) => t,
                    Some(Ok(Message::Close(f))) => break f.map_or("the server closed the connection".to_owned(), |f| format!("closed: {}", f.reason)),
                    Some(Ok(_)) => { last_frame = Instant::now(); continue; }
                };
                last_frame = Instant::now();
                let text = text.as_str();
                if let Some(part) = text.strip_prefix(PART_PREFIX) { parts.push_str(part); continue; }
                let whole = if parts.is_empty() { serde_json::from_str::<Value>(text) } else {
                    parts.push_str(text);
                    let v = serde_json::from_str::<Value>(&parts);
                    parts.clear();
                    v
                };
                let Ok(value) = whole else { continue };
                // The shared parser accepts arrays; the server sends one message per frame.
                let messages = match value { Value::Array(a) => a, v => vec![v] };
                for m in messages {
                    if let Some(ack) = handle(&mut pending, m) {
                        if let Err(e) = write.send(send(ack)).await { return finish(pending, format!("send failed: {e}")); }
                    }
                }
            }
            _ = ping.tick() => {
                if last_frame.elapsed() > LIVENESS { break "the server stopped answering".to_owned(); }
                if let Err(e) = write.send(send(json!({ "_tag": "Ping" }))).await { break format!("send failed: {e}"); }
            }
        }
    };
    finish(pending, reason)
}

fn finish(pending: HashMap<u64, Pending>, reason: String) -> String {
    for (_, p) in pending {
        match p {
            Pending::Call(reply) => drop(reply.send(Err(RpcError::Disconnected))),
            Pending::Stream(sink) => drop(sink.send(StreamMsg::End(Err(RpcError::Disconnected)))),
        }
    }
    reason
}

/// Routes one server message; returns the Ack to send for a chunk.
fn handle(pending: &mut HashMap<u64, Pending>, m: Value) -> Option<Value> {
    let tag = m.get("_tag").and_then(Value::as_str)?;
    let id = m.get("requestId").and_then(Value::as_u64);
    match tag {
        "Chunk" => {
            let id = id?;
            let values = match m.get("values") {
                Some(Value::Array(v)) => v.clone(),
                _ => vec![],
            };
            match pending.get(&id) {
                Some(Pending::Stream(sink)) => {
                    if sink.send(StreamMsg::Chunk(values)).is_err() {
                        // The consumer went away: stop the stream rather than ack it forever.
                        pending.remove(&id);
                        return Some(json!({ "_tag": "Interrupt", "requestId": id }));
                    }
                    Some(json!({ "_tag": "Ack", "requestId": id }))
                }
                _ => None,
            }
        }
        "Exit" => {
            let exit = m.get("exit")?;
            let result = match exit.get("_tag").and_then(Value::as_str) {
                Some("Success") => Ok(exit.get("value").cloned().unwrap_or(Value::Null)),
                _ => {
                    let cause = exit.get("cause").cloned().unwrap_or(Value::Null);
                    let interrupted = cause.as_array().is_some_and(|c| {
                        !c.is_empty()
                            && c.iter()
                                .all(|r| r.get("_tag").and_then(Value::as_str) == Some("Interrupt"))
                    });
                    Err(if interrupted {
                        RpcError::Interrupted
                    } else {
                        RpcError::Failure(cause)
                    })
                }
            };
            match pending.remove(&id?)? {
                Pending::Call(reply) => drop(reply.send(result)),
                Pending::Stream(sink) => drop(sink.send(StreamMsg::End(result.map(|_| ())))),
            }
            None
        }
        // A socket-level defect fails everything in flight, as the Effect client does.
        "Defect" => {
            let defect = m.get("defect").cloned().unwrap_or(Value::Null);
            for (_, p) in pending.drain() {
                let err = RpcError::Failure(json!([{ "_tag": "Die", "defect": defect }]));
                match p {
                    Pending::Call(reply) => drop(reply.send(Err(err))),
                    Pending::Stream(sink) => drop(sink.send(StreamMsg::End(Err(err)))),
                }
            }
            None
        }
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn chunks_ack_and_exits_resolve() {
        let mut pending = HashMap::new();
        let (sink, mut rx) = mpsc::unbounded_channel();
        pending.insert(4, Pending::Stream(sink));
        let ack = handle(
            &mut pending,
            json!({ "_tag": "Chunk", "requestId": 4, "values": [1, 2] }),
        );
        assert_eq!(ack, Some(json!({ "_tag": "Ack", "requestId": 4 })));
        assert!(matches!(rx.try_recv(), Ok(StreamMsg::Chunk(v)) if v.len() == 2));
        handle(
            &mut pending,
            json!({ "_tag": "Exit", "requestId": 4, "exit": { "_tag": "Success", "value": null } }),
        );
        assert!(matches!(rx.try_recv(), Ok(StreamMsg::End(Ok(())))));
        assert!(pending.is_empty());
    }

    #[test]
    fn failures_carry_the_typed_error_message() {
        let mut pending = HashMap::new();
        let (reply, mut rx) = oneshot::channel();
        pending.insert(1, Pending::Call(reply));
        handle(
            &mut pending,
            json!({ "_tag": "Exit", "requestId": 1, "exit": { "_tag": "Failure", "cause": [
            { "_tag": "Fail", "error": { "_tag": "OrchestrationV2DispatchCommandError", "message": "Thread is busy" } } ] } }),
        );
        let err = rx.try_recv().unwrap().unwrap_err();
        assert_eq!(err.message(), "Thread is busy");
    }
}
