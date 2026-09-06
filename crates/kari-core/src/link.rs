//! The link: one outbound WebSocket from a node to a server.
//!
//! Peer to peer needs every machine to reach every other one. A laptop that
//! sleeps or roams cannot be reached, so it is offline to every client at once
//! while being perfectly able to reach out itself. The link inverts that: the
//! node dials the server and holds one socket, and the server calls the node's
//! API back down it.
//!
//! What travels is the node API unchanged. A `Req` frame is dispatched into the
//! very same `api::router()` the node serves on loopback, as a plain tower
//! service with no listener in front of it, and the response goes back as a
//! `Res`. So there is one API with two transports, and a route added to the
//! router is reachable over both without another line of code.
//!
//! Events are the exception. A request and a response cannot carry a stream, so
//! the node pushes `Evt` frames as its engine emits, and the server turns each
//! into what it would have made from a message on `/kari/v1/events`.
//!
//! Liveness is the WebSocket's own ping and pong, not a frame of ours: the
//! server pings on an interval and the node's stack answers without waking any
//! of this code.

use crate::hooks::TOKEN_HEADER;
use crate::model::NodeIdentity;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::{broadcast, mpsc, oneshot};
use tracing::{debug, info, warn};

/// Where a node dials and a server listens.
pub const LINK_PATH: &str = "/kari/v1/link";

/// The framing version. A node and a server that disagree do not link, because
/// neither can know what the other left out.
pub const PROTOCOL_VERSION: u32 = 1;

/// How long a call over the link waits for its response.
pub const CALL_TIMEOUT: Duration = Duration::from_secs(30);

/// The largest response body the server accepts from a node. A board of a few
/// hundred cards is far under this; the cap is here so a broken node cannot
/// make the server allocate without bound.
pub const MAX_BODY: usize = 8 * 1024 * 1024;

/// One frame on the link. JSON, one object per WebSocket text message.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum Frame {
    /// First frame the node sends. Registers it, so the server never has to ask
    /// who called before it can call back.
    Hello {
        protocol: u32,
        identity: Box<NodeIdentity>,
    },
    /// Server to node: one call on the node API.
    Req {
        id: u64,
        method: String,
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        body: Option<Value>,
    },
    /// Node to server: the answer to one `Req`.
    Res { id: u64, status: u16, body: Value },
    /// Node to server: an engine event, in the shape `/kari/v1/events` sends.
    Evt { event: String, data: Value },
}

impl Frame {
    fn encode(&self) -> anyhow::Result<String> {
        Ok(serde_json::to_string(self)?)
    }

    fn decode(s: &str) -> anyhow::Result<Frame> {
        Ok(serde_json::from_str(s)?)
    }
}

/// The answer to one call: the node's status and its JSON body.
pub type CallResult = (u16, Value);

/// Calls waiting for their `Res`, by frame id. The pump takes a sender out and
/// hands the answer over; a dropped sender is how a caller learns the socket
/// closed underneath it.
type Pending = Arc<Mutex<HashMap<u64, oneshot::Sender<CallResult>>>>;

/// An event a node pushed up its link.
#[derive(Debug, Clone)]
pub struct LinkEvent {
    pub node_id: String,
    pub event: String,
    pub data: Value,
}

/// The server's side of one node's link: call the node, and watch it close.
///
/// Cloning is cheap and every clone talks to the same socket.
#[derive(Clone)]
pub struct LinkHandle {
    identity: Arc<NodeIdentity>,
    out: mpsc::UnboundedSender<Frame>,
    pending: Pending,
    next_id: Arc<AtomicU64>,
    open: Arc<AtomicBool>,
}

impl LinkHandle {
    pub fn identity(&self) -> &NodeIdentity {
        &self.identity
    }

    pub fn node_id(&self) -> &str {
        &self.identity.node_id
    }

    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    /// Call one route on the node and wait for its answer.
    ///
    /// `path` is the whole path as the node's router knows it, such as
    /// `/kari/v1/board`. The status comes back with the body, so a caller can
    /// tell "the node said no" from "the node did not answer".
    pub async fn request(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> anyhow::Result<CallResult> {
        if !self.is_open() {
            anyhow::bail!("link to {} is closed", self.node_id());
        }
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = oneshot::channel();
        self.pending.lock().expect("pending").insert(id, tx);
        let sent = self.out.send(Frame::Req {
            id,
            method: method.to_string(),
            path: path.to_string(),
            body,
        });
        if sent.is_err() {
            self.pending.lock().expect("pending").remove(&id);
            anyhow::bail!("link to {} is closed", self.node_id());
        }
        match tokio::time::timeout(CALL_TIMEOUT, rx).await {
            Ok(Ok(v)) => Ok(v),
            // The pump dropped the sender: the socket closed under the call.
            Ok(Err(_)) => anyhow::bail!("link to {} closed during the call", self.node_id()),
            Err(_) => {
                self.pending.lock().expect("pending").remove(&id);
                anyhow::bail!(
                    "{} did not answer {method} {path} in {}s",
                    self.node_id(),
                    CALL_TIMEOUT.as_secs()
                )
            }
        }
    }

    /// The same call, decoded, with a non-2xx status turned into an error that
    /// carries whatever the node put in `error`.
    pub async fn call<T: serde::de::DeserializeOwned>(
        &self,
        method: &str,
        path: &str,
        body: Option<Value>,
    ) -> anyhow::Result<T> {
        let (status, body) = self.request(method, path, body).await?;
        if !(200..300).contains(&status) {
            let msg = body
                .get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
                .unwrap_or_else(|| body.to_string());
            anyhow::bail!("{status}: {msg}");
        }
        Ok(serde_json::from_value(body)?)
    }

    /// Stop the link. The pump ends and every waiting call fails.
    pub fn close(&self) {
        self.open.store(false, Ordering::SeqCst);
        self.pending.lock().expect("pending").clear();
    }
}

// ------------------------------------------------------------------ registry

/// A node as the server knows it: what it said about itself, and when it was
/// last connected. A node that goes away keeps its row, so the board can show
/// it dimmed with a time rather than dropping it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LinkedNode {
    pub node_id: String,
    pub node_name: String,
    pub platform: String,
    pub version: String,
    pub api_version: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account: Option<crate::account::AccountIdentity>,
    pub online: bool,
    pub connected_at: chrono::DateTime<chrono::Utc>,
    pub last_seen: chrono::DateTime<chrono::Utc>,
}

/// Every node linked to this server, and the ones that were.
pub struct LinkRegistry {
    links: Mutex<HashMap<String, LinkHandle>>,
    seen: Mutex<HashMap<String, LinkedNode>>,
    events: broadcast::Sender<LinkEvent>,
}

impl Default for LinkRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl LinkRegistry {
    pub fn new() -> LinkRegistry {
        let (events, _) = broadcast::channel(256);
        LinkRegistry {
            links: Mutex::new(HashMap::new()),
            seen: Mutex::new(HashMap::new()),
            events,
        }
    }

    pub fn subscribe(&self) -> broadcast::Receiver<LinkEvent> {
        self.events.subscribe()
    }

    /// The open link to one node, if it is connected now.
    pub fn get(&self, node_id: &str) -> Option<LinkHandle> {
        self.links
            .lock()
            .expect("links")
            .get(node_id)
            .filter(|l| l.is_open())
            .cloned()
    }

    /// Every open link, in a stable order.
    pub fn open_links(&self) -> Vec<LinkHandle> {
        let mut v: Vec<LinkHandle> = self
            .links
            .lock()
            .expect("links")
            .values()
            .filter(|l| l.is_open())
            .cloned()
            .collect();
        v.sort_by(|a, b| a.node_id().cmp(b.node_id()));
        v
    }

    /// Every node this server has seen, connected or not, newest name first.
    pub fn nodes(&self) -> Vec<LinkedNode> {
        let mut v: Vec<LinkedNode> = self.seen.lock().expect("seen").values().cloned().collect();
        v.sort_by(|a, b| {
            b.online
                .cmp(&a.online)
                .then(a.node_name.cmp(&b.node_name))
                .then(a.node_id.cmp(&b.node_id))
        });
        v
    }

    fn register(&self, h: LinkHandle) -> Option<LinkHandle> {
        let id = h.node_id().to_string();
        let now = chrono::Utc::now();
        let i = h.identity();
        let row = LinkedNode {
            node_id: id.clone(),
            node_name: i.node_name.clone(),
            platform: i.platform.clone(),
            version: i.version.clone(),
            api_version: i.api_version,
            account: i.account.clone(),
            online: true,
            connected_at: now,
            last_seen: now,
        };
        self.seen.lock().expect("seen").insert(id.clone(), row);
        // A node that reconnects before the server noticed the old socket died
        // replaces it. The old handle is returned so the caller can close it.
        self.links.lock().expect("links").insert(id, h)
    }

    fn deregister(&self, node_id: &str, handle: &LinkHandle) {
        let mut links = self.links.lock().expect("links");
        // Only drop the map entry when it is still this link. A node that
        // reconnected before this cleanup ran has a newer handle in the map,
        // and that one is live: neither its entry nor its row may be touched,
        // or a reconnect would leave the board showing the node as offline.
        let was_ours = links
            .get(node_id)
            .is_some_and(|cur| Arc::ptr_eq(&cur.open, &handle.open));
        if was_ours {
            links.remove(node_id);
        }
        drop(links);
        if !was_ours {
            return;
        }
        if let Some(row) = self.seen.lock().expect("seen").get_mut(node_id) {
            row.online = false;
            row.last_seen = chrono::Utc::now();
        }
    }

    fn touch(&self, node_id: &str) {
        if let Some(row) = self.seen.lock().expect("seen").get_mut(node_id) {
            row.last_seen = chrono::Utc::now();
        }
    }
}

// ------------------------------------------------------------- server side

/// Serve one accepted WebSocket: register the node, then pump frames until the
/// socket ends. Runs until the link dies, so a caller spawns it per connection.
pub async fn serve_link(socket: axum::extract::ws::WebSocket, reg: Arc<LinkRegistry>) {
    use axum::extract::ws::Message;
    use futures_util::{SinkExt, StreamExt};

    let (mut sink, mut stream) = socket.split();

    // A node says who it is before anything else, so the server never has to
    // call back to learn the name of the caller.
    let identity = match tokio::time::timeout(Duration::from_secs(10), stream.next()).await {
        Ok(Some(Ok(Message::Text(t)))) => match Frame::decode(&t) {
            Ok(Frame::Hello { protocol, identity }) => {
                if protocol != PROTOCOL_VERSION {
                    warn!(
                        "node {} speaks link protocol {protocol}, this server speaks {PROTOCOL_VERSION}",
                        identity.node_id
                    );
                    let _ = sink.send(Message::Close(None)).await;
                    return;
                }
                *identity
            }
            Ok(_) => {
                warn!("first frame on a link was not a hello");
                let _ = sink.send(Message::Close(None)).await;
                return;
            }
            Err(e) => {
                warn!("undecodable hello: {e}");
                return;
            }
        },
        _ => {
            warn!("a link opened but sent no hello");
            return;
        }
    };

    let node_id = identity.node_id.clone();
    let node_name = identity.node_name.clone();
    if node_id.is_empty() {
        warn!("a link said hello with no node id");
        return;
    }

    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();
    let handle = LinkHandle {
        identity: Arc::new(identity),
        out: out_tx,
        pending: Arc::new(Mutex::new(HashMap::new())),
        next_id: Arc::new(AtomicU64::new(1)),
        open: Arc::new(AtomicBool::new(true)),
    };
    if let Some(old) = reg.register(handle.clone()) {
        info!("node {node_name} reconnected; dropping the previous link");
        old.close();
    }
    info!("node {node_name} ({node_id}) linked");

    // The writer owns the sink: every frame and every keepalive goes through
    // here, so nothing else needs a lock on it.
    let writer = tokio::spawn(async move {
        let mut ping = tokio::time::interval(Duration::from_secs(20));
        ping.tick().await; // the first tick is immediate
        loop {
            tokio::select! {
                frame = out_rx.recv() => {
                    let Some(frame) = frame else { break };
                    let Ok(text) = frame.encode() else { continue };
                    if sink.send(Message::Text(text.into())).await.is_err() {
                        break;
                    }
                }
                _ = ping.tick() => {
                    if sink.send(Message::Ping(Default::default())).await.is_err() {
                        break;
                    }
                }
            }
        }
        let _ = sink.send(Message::Close(None)).await;
    });

    while let Some(msg) = stream.next().await {
        let Ok(msg) = msg else { break };
        match msg {
            Message::Text(t) => {
                reg.touch(&node_id);
                match Frame::decode(&t) {
                    Ok(Frame::Res { id, status, body }) => {
                        let waiting = handle.pending.lock().expect("pending").remove(&id);
                        if let Some(tx) = waiting {
                            let _ = tx.send((status, body));
                        } else {
                            debug!("late or unknown response {id} from {node_name}");
                        }
                    }
                    Ok(Frame::Evt { event, data }) => {
                        let _ = reg.events.send(LinkEvent {
                            node_id: node_id.clone(),
                            event,
                            data,
                        });
                    }
                    Ok(Frame::Hello { .. }) => debug!("{node_name} said hello twice"),
                    Ok(Frame::Req { id, .. }) => {
                        // Nodes do not call servers. Answer so the node's own
                        // pending call does not hang for its timeout.
                        let _ = handle.out.send(Frame::Res {
                            id,
                            status: 501,
                            body: serde_json::json!({ "error": "a server takes no requests" }),
                        });
                    }
                    Err(e) => warn!("undecodable frame from {node_name}: {e}"),
                }
            }
            Message::Pong(_) | Message::Ping(_) => reg.touch(&node_id),
            Message::Close(_) => break,
            Message::Binary(_) => debug!("binary frame from {node_name} ignored"),
        }
    }

    handle.close();
    writer.abort();
    reg.deregister(&node_id, &handle);
    info!("node {node_name} ({node_id}) unlinked");
}

// --------------------------------------------------------------- node side

/// Choose rustls' crypto provider, once per process.
///
/// rustls 0.23 will not guess. With neither or both of its `ring` and
/// `aws-lc-rs` features enabled it panics at the first handshake, inside the
/// task doing the connecting — so a node pointed at an `https://` server dies
/// on a background thread while the log still says it is linking. Which of the
/// two features ends up on is a property of the whole dependency graph, not of
/// anything kari declares, so kari names the provider itself and stops
/// depending on the answer.
///
/// Idempotent: a second call, or a provider installed by the host application,
/// is fine and the error is deliberately dropped.
pub fn ensure_crypto_provider() {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = rustls::crypto::ring::default_provider().install_default();
    });
}

/// The WebSocket URL of a server's link endpoint, from whatever base URL the
/// user configured. `http` and `https` are accepted and translated, because
/// that is what a person types.
pub fn link_url(server: &str) -> String {
    let s = server.trim().trim_end_matches('/');
    let s = if let Some(rest) = s.strip_prefix("https://") {
        format!("wss://{rest}")
    } else if let Some(rest) = s.strip_prefix("http://") {
        format!("ws://{rest}")
    } else if s.starts_with("ws://") || s.starts_with("wss://") {
        s.to_string()
    } else {
        format!("ws://{s}")
    };
    format!("{s}{LINK_PATH}")
}

/// Run one call against the node's own router, with no listener in front of it.
///
/// This is the whole reason the link carries so little of its own: the router
/// is the same object `kari-node serve` binds to a port, so every route reaches
/// the link for free and the two transports cannot drift apart.
async fn dispatch(
    router: axum::Router,
    local_token: &str,
    method: &str,
    path: &str,
    body: Option<Value>,
) -> (u16, Value) {
    use tower::ServiceExt;

    // The event stream never ends, so it would hold a request slot forever.
    // Events reach the server as `Evt` pushes instead.
    if path.starts_with("/kari/v1/events") {
        return (
            400,
            serde_json::json!({ "error": "events are pushed on the link, not requested" }),
        );
    }

    let mut b = axum::http::Request::builder()
        .method(method)
        .uri(path)
        .header(TOKEN_HEADER, local_token);
    let req = match body {
        Some(v) => {
            b = b.header("content-type", "application/json");
            match serde_json::to_vec(&v) {
                Ok(bytes) => b.body(axum::body::Body::from(bytes)),
                Err(e) => return (400, serde_json::json!({ "error": e.to_string() })),
            }
        }
        None => b.body(axum::body::Body::empty()),
    };
    let req = match req {
        Ok(r) => r,
        Err(e) => return (400, serde_json::json!({ "error": e.to_string() })),
    };

    let resp = match router.oneshot(req).await {
        Ok(r) => r,
        Err(e) => return (500, serde_json::json!({ "error": e.to_string() })),
    };
    let status = resp.status().as_u16();
    let bytes = match axum::body::to_bytes(resp.into_body(), MAX_BODY).await {
        Ok(b) => b,
        Err(e) => return (500, serde_json::json!({ "error": e.to_string() })),
    };
    if bytes.is_empty() {
        return (status, Value::Null);
    }
    match serde_json::from_slice(&bytes) {
        Ok(v) => (status, v),
        // Not every route answers JSON. Hand the text back rather than lose it.
        Err(_) => (
            status,
            Value::String(String::from_utf8_lossy(&bytes).into()),
        ),
    }
}

/// Hold a link to `server`, reconnecting for as long as this future is polled.
///
/// Never returns. A node that cannot reach its server keeps serving loopback,
/// so nothing about the sessions or the jobs on this host depends on the link.
pub async fn run(
    engine: Arc<crate::Engine>,
    server: String,
    server_token: String,
    local_token: String,
) {
    ensure_crypto_provider();
    let url = link_url(&server);
    let router = crate::api::router(Arc::clone(&engine), local_token.clone());
    let mut backoff = 1u64;
    loop {
        let started = std::time::Instant::now();
        match connect_once(&engine, &router, &url, &server_token, &local_token).await {
            Ok(()) => info!("link to {server} closed"),
            Err(e) => warn!("link to {server} failed: {e}"),
        }
        // A link that held for a while starts its backoff over, so a server
        // that restarts nightly is not met with a minute of silence.
        if started.elapsed() > Duration::from_secs(60) {
            backoff = 1;
        }
        tokio::time::sleep(Duration::from_secs(backoff)).await;
        backoff = (backoff * 2).min(60);
    }
}

async fn connect_once(
    engine: &Arc<crate::Engine>,
    router: &axum::Router,
    url: &str,
    server_token: &str,
    local_token: &str,
) -> anyhow::Result<()> {
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    let mut req = url.into_client_request()?;
    req.headers_mut().insert(
        TOKEN_HEADER,
        server_token
            .parse()
            .map_err(|_| anyhow::anyhow!("the server token is not a valid header value"))?,
    );
    let (ws, _) = tokio_tungstenite::connect_async(req).await?;
    let (mut sink, mut stream) = ws.split();

    sink.send(Message::Text(
        Frame::Hello {
            protocol: PROTOCOL_VERSION,
            identity: Box::new(engine.identity()),
        }
        .encode()?
        .into(),
    ))
    .await?;
    info!("linked to the server at {url}");

    // One writer, many producers: the event forwarder and every dispatched
    // request hand their frames here rather than share the sink.
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();
    let writer = tokio::spawn(async move {
        while let Some(frame) = out_rx.recv().await {
            let Ok(text) = frame.encode() else { continue };
            if sink.send(Message::Text(text.into())).await.is_err() {
                break;
            }
        }
    });

    // The engine's events, in the shape `/kari/v1/events` would have sent.
    let mut rx = engine.subscribe();
    let ev_tx = out_tx.clone();
    let events = tokio::spawn(async move {
        loop {
            let frame = match rx.recv().await {
                Ok(crate::Event::BoardChanged) => Frame::Evt {
                    event: "board_changed".into(),
                    data: serde_json::json!({}),
                },
                Ok(crate::Event::LeaseChanged) => Frame::Evt {
                    event: "lease_changed".into(),
                    data: serde_json::json!({}),
                },
                Ok(crate::Event::Notice {
                    title,
                    body,
                    card_id,
                }) => Frame::Evt {
                    event: "notice".into(),
                    data: serde_json::json!({ "title": title, "body": body, "card_id": card_id }),
                },
                // A slow reader lost events; the server refetches the board.
                Err(broadcast::error::RecvError::Lagged(_)) => Frame::Evt {
                    event: "board_changed".into(),
                    data: serde_json::json!({}),
                },
                Err(broadcast::error::RecvError::Closed) => break,
            };
            if ev_tx.send(frame).is_err() {
                break;
            }
        }
    });

    let result = loop {
        let Some(msg) = stream.next().await else {
            break Ok(());
        };
        let msg = match msg {
            Ok(m) => m,
            Err(e) => break Err(anyhow::Error::from(e)),
        };
        match msg {
            Message::Text(t) => match Frame::decode(&t) {
                Ok(Frame::Req {
                    id,
                    method,
                    path,
                    body,
                }) => {
                    // Concurrently, so one slow call does not stall the socket.
                    let router = router.clone();
                    let token = local_token.to_string();
                    let tx = out_tx.clone();
                    tokio::spawn(async move {
                        let (status, body) = dispatch(router, &token, &method, &path, body).await;
                        let _ = tx.send(Frame::Res { id, status, body });
                    });
                }
                Ok(Frame::Res { .. }) | Ok(Frame::Evt { .. }) | Ok(Frame::Hello { .. }) => {
                    debug!("a server sent a frame only a node sends")
                }
                Err(e) => warn!("undecodable frame from the server: {e}"),
            },
            Message::Close(_) => break Ok(()),
            _ => {}
        }
    };

    events.abort();
    writer.abort();
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures_util::{SinkExt, StreamExt};
    use tokio_tungstenite::tungstenite::client::IntoClientRequest;
    use tokio_tungstenite::tungstenite::Message;

    fn identity(id: &str, name: &str) -> NodeIdentity {
        NodeIdentity {
            ok: true,
            app: "kari".into(),
            version: "0.6.0".into(),
            api_version: 1,
            node_id: id.into(),
            node_name: name.into(),
            platform: "linux".into(),
            addresses: vec![],
            account: None,
        }
    }

    #[test]
    fn frames_round_trip() {
        let cases = vec![
            Frame::Req {
                id: 7,
                method: "POST".into(),
                path: "/kari/v1/cards".into(),
                body: Some(serde_json::json!({ "title": "x" })),
            },
            Frame::Res {
                id: 7,
                status: 200,
                body: serde_json::json!({ "ok": true }),
            },
            Frame::Evt {
                event: "board_changed".into(),
                data: serde_json::json!({}),
            },
            Frame::Hello {
                protocol: PROTOCOL_VERSION,
                identity: Box::new(identity("n1", "one")),
            },
        ];
        for c in cases {
            let wire = c.encode().expect("encode");
            let back = Frame::decode(&wire).expect("decode");
            assert_eq!(wire, back.encode().expect("re-encode"));
        }
    }

    /// A `Req` with no body must not carry a null one: an older node would take
    /// `"body": null` as a body and set a content type for it.
    #[test]
    fn a_request_without_a_body_omits_the_field() {
        let wire = Frame::Req {
            id: 1,
            method: "GET".into(),
            path: "/kari/v1/board".into(),
            body: None,
        }
        .encode()
        .expect("encode");
        assert!(!wire.contains("body"), "{wire}");
    }

    #[test]
    fn undecodable_frames_are_errors_not_panics() {
        assert!(Frame::decode("{}").is_err());
        assert!(Frame::decode("not json").is_err());
        assert!(Frame::decode(r#"{"t":"nope"}"#).is_err());
    }

    /// The panic this prevents happens inside the connecting task, at the first
    /// handshake, on a machine that has a TLS server to talk to — so no test of
    /// the plaintext path catches it. Assert the provider directly instead.
    #[test]
    fn a_crypto_provider_is_installed_for_wss() {
        ensure_crypto_provider();
        assert!(
            rustls::crypto::CryptoProvider::get_default().is_some(),
            "no rustls provider: a node pointed at an https:// server would panic"
        );
        ensure_crypto_provider(); // idempotent
    }

    #[test]
    fn link_urls_take_what_a_person_types() {
        for (input, want) in [
            ("http://host:47312", "ws://host:47312/kari/v1/link"),
            ("https://host", "wss://host/kari/v1/link"),
            ("host:47312", "ws://host:47312/kari/v1/link"),
            ("ws://host:47312/", "ws://host:47312/kari/v1/link"),
            ("  http://host:47312/  ", "ws://host:47312/kari/v1/link"),
        ] {
            assert_eq!(link_url(input), want, "for {input}");
        }
    }

    /// Start the server router on a loopback port and hand back its address.
    async fn server(reg: Arc<LinkRegistry>) -> std::net::SocketAddr {
        let app = crate::server::router(reg, "test-token".into());
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let addr = listener.local_addr().expect("addr");
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        addr
    }

    /// A node that dials in, says hello, and answers every request with the
    /// same canned body. Enough to prove the framing and the correlation
    /// without an engine behind it.
    async fn fake_node(
        addr: std::net::SocketAddr,
        id: &str,
        name: &str,
        answer: Value,
    ) -> tokio::task::JoinHandle<()> {
        let url = link_url(&format!("http://{addr}"));
        let mut req = url.into_client_request().expect("request");
        req.headers_mut()
            .insert(TOKEN_HEADER, "test-token".parse().expect("header"));
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .expect("connect");
        let (mut sink, mut stream) = ws.split();
        sink.send(Message::Text(
            Frame::Hello {
                protocol: PROTOCOL_VERSION,
                identity: Box::new(identity(id, name)),
            }
            .encode()
            .expect("encode")
            .into(),
        ))
        .await
        .expect("hello");
        tokio::spawn(async move {
            while let Some(Ok(msg)) = stream.next().await {
                if let Message::Text(t) = msg {
                    if let Ok(Frame::Req { id, .. }) = Frame::decode(&t) {
                        let res = Frame::Res {
                            id,
                            status: 200,
                            body: answer.clone(),
                        }
                        .encode()
                        .expect("encode");
                        if sink.send(Message::Text(res.into())).await.is_err() {
                            break;
                        }
                    }
                }
            }
        })
    }

    /// Wait for the registry to hold an open link to `id`.
    async fn wait_for(reg: &LinkRegistry, id: &str) -> LinkHandle {
        for _ in 0..100 {
            if let Some(h) = reg.get(id) {
                return h;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        panic!("node {id} never linked");
    }

    #[tokio::test]
    async fn a_node_that_dials_in_is_registered_and_can_be_called() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let _node = fake_node(addr, "n1", "one", serde_json::json!({ "cards": [] })).await;

        let link = wait_for(&reg, "n1").await;
        assert_eq!(link.identity().node_name, "one");
        assert!(link.is_open());

        // The server calls the node over the socket the node opened.
        let (status, body) = link
            .request("GET", "/kari/v1/board", None)
            .await
            .expect("board");
        assert_eq!(status, 200);
        assert_eq!(body, serde_json::json!({ "cards": [] }));

        // And the roster names it.
        let nodes = reg.nodes();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].node_id, "n1");
        assert!(nodes[0].online);
    }

    /// Two calls in flight at once must not take each other's answers.
    #[tokio::test]
    async fn concurrent_calls_keep_their_own_answers() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let _node = fake_node(addr, "n1", "one", serde_json::json!({ "ok": true })).await;
        let link = wait_for(&reg, "n1").await;

        let calls = (0..25).map(|_| link.request("GET", "/kari/v1/board", None));
        for r in futures_util::future::join_all(calls).await {
            let (status, body) = r.expect("call");
            assert_eq!(status, 200);
            assert_eq!(body, serde_json::json!({ "ok": true }));
        }
    }

    #[tokio::test]
    async fn a_closed_link_fails_its_calls_and_goes_offline() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let node = fake_node(addr, "n1", "one", serde_json::json!({})).await;
        let link = wait_for(&reg, "n1").await;

        node.abort();
        for _ in 0..100 {
            if !reg.get("n1").is_some_and(|h| h.is_open()) {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert!(reg.get("n1").is_none(), "the link should be gone");
        assert!(link.request("GET", "/kari/v1/board", None).await.is_err());

        // The node keeps its row, so a board can show it dimmed.
        let nodes = reg.nodes();
        assert_eq!(nodes.len(), 1);
        assert!(!nodes[0].online);
    }

    #[tokio::test]
    async fn a_link_with_the_wrong_protocol_is_refused() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let url = link_url(&format!("http://{addr}"));
        let mut req = url.into_client_request().expect("request");
        req.headers_mut()
            .insert(TOKEN_HEADER, "test-token".parse().expect("header"));
        let (ws, _) = tokio_tungstenite::connect_async(req)
            .await
            .expect("connect");
        let (mut sink, _stream) = ws.split();
        sink.send(Message::Text(
            Frame::Hello {
                protocol: PROTOCOL_VERSION + 1,
                identity: Box::new(identity("n1", "one")),
            }
            .encode()
            .expect("encode")
            .into(),
        ))
        .await
        .expect("hello");

        tokio::time::sleep(Duration::from_millis(200)).await;
        assert!(reg.get("n1").is_none());
        assert!(reg.nodes().is_empty());
    }

    /// The root and the health route say the same thing, so a probe that knows
    /// nothing about kari does not read a 404 as a dead service.
    #[tokio::test]
    async fn the_root_answers_like_health() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let http = reqwest::Client::new();
        let mut seen = vec![];
        for path in ["/", "/kari/health"] {
            let r = http
                .get(format!("http://{addr}{path}"))
                .send()
                .await
                .expect("get");
            assert_eq!(r.status().as_u16(), 200, "for {path}");
            seen.push(r.json::<Value>().await.expect("json"));
        }
        assert_eq!(seen[0]["app"], "kari-server");
        assert_eq!(seen[0]["app"], seen[1]["app"]);
    }

    #[tokio::test]
    async fn a_link_without_the_token_is_refused() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let url = link_url(&format!("http://{addr}"));
        let req = url.into_client_request().expect("request");
        assert!(
            tokio_tungstenite::connect_async(req).await.is_err(),
            "an untokened link must not upgrade"
        );
    }

    #[tokio::test]
    async fn a_reconnect_replaces_the_previous_link() {
        let reg = Arc::new(LinkRegistry::new());
        let addr = server(Arc::clone(&reg)).await;
        let first = fake_node(addr, "n1", "one", serde_json::json!({ "n": 1 })).await;
        let old = wait_for(&reg, "n1").await;

        let _second = fake_node(addr, "n1", "one", serde_json::json!({ "n": 2 })).await;
        // The newer socket wins, and the roster still holds one row.
        for _ in 0..100 {
            if reg
                .get("n1")
                .and_then(|h| (!Arc::ptr_eq(&h.open, &old.open)).then_some(()))
                .is_some()
            {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let now = reg.get("n1").expect("linked");
        assert!(!Arc::ptr_eq(&now.open, &old.open), "still the old link");
        assert_eq!(reg.nodes().len(), 1);
        let (_, body) = now
            .request("GET", "/kari/v1/board", None)
            .await
            .expect("board");
        assert_eq!(body, serde_json::json!({ "n": 2 }));

        // The old socket now closes. Its cleanup must not touch the live link
        // that replaced it, or a reconnect would read as an offline node.
        first.abort();
        tokio::time::sleep(Duration::from_millis(300)).await;
        let nodes = reg.nodes();
        assert_eq!(nodes.len(), 1);
        assert!(nodes[0].online, "the reconnected node was marked offline");
        assert!(reg.get("n1").is_some(), "the live link was dropped");
        let (_, body) = reg
            .get("n1")
            .expect("linked")
            .request("GET", "/kari/v1/board", None)
            .await
            .expect("board after the old socket closed");
        assert_eq!(body, serde_json::json!({ "n": 2 }));
    }

    // ---- the node's side: one router, two transports ----

    fn test_router() -> axum::Router {
        use axum::routing::{get, post};
        axum::Router::new()
            .route(
                "/kari/v1/board",
                get(|| async { axum::Json(serde_json::json!({ "cards": [] })) }),
            )
            .route(
                "/kari/v1/cards",
                post(|body: axum::Json<Value>| async move { axum::Json(body.0) }),
            )
            .route(
                "/kari/v1/boom",
                get(|| async {
                    (
                        axum::http::StatusCode::BAD_REQUEST,
                        axum::Json(serde_json::json!({ "error": "no" })),
                    )
                }),
            )
    }

    #[tokio::test]
    async fn dispatch_runs_the_router_with_no_listener() {
        let (status, body) = dispatch(test_router(), "tok", "GET", "/kari/v1/board", None).await;
        assert_eq!(status, 200);
        assert_eq!(body, serde_json::json!({ "cards": [] }));
    }

    #[tokio::test]
    async fn dispatch_carries_a_body_both_ways() {
        let sent = serde_json::json!({ "title": "a card" });
        let (status, body) = dispatch(
            test_router(),
            "tok",
            "POST",
            "/kari/v1/cards",
            Some(sent.clone()),
        )
        .await;
        assert_eq!(status, 200);
        assert_eq!(body, sent);
    }

    #[tokio::test]
    async fn dispatch_keeps_the_status_of_a_refusal() {
        let (status, body) = dispatch(test_router(), "tok", "GET", "/kari/v1/boom", None).await;
        assert_eq!(status, 400);
        assert_eq!(body, serde_json::json!({ "error": "no" }));
    }

    #[tokio::test]
    async fn dispatch_answers_an_unknown_route_rather_than_hanging() {
        let (status, _) = dispatch(test_router(), "tok", "GET", "/kari/v1/nope", None).await;
        assert_eq!(status, 404);
    }

    /// The event stream never ends, so requesting it would hold a call open
    /// until its timeout. Events come up the link as pushes instead.
    #[tokio::test]
    async fn dispatch_refuses_the_event_stream() {
        let (status, body) = dispatch(test_router(), "tok", "GET", "/kari/v1/events", None).await;
        assert_eq!(status, 400);
        assert!(body["error"].as_str().expect("error").contains("pushed"));
    }
}
