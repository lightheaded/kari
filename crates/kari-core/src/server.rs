//! The server: the always-on host every node dials and every client asks.
//!
//! A node cannot always be reached, but it can always reach out. The server is
//! the fixed point that makes that useful: it accepts a link from each node
//! (see `link`), keeps the roster, and answers questions about all of them at
//! once. It runs no Claude Code, holds no login and reads no transcript, so it
//! is the one part of kari with nothing on the machine it needs.
//!
//! This is the first half. It proves the chain end to end — a node's own API,
//! reached over its outbound link, merged across nodes — and it is deliberately
//! read only. The hub API that replaces the desktop's in-process hub, and the
//! enrolment that gives nodes and clients separate tokens, come next.

use crate::hooks::TOKEN_HEADER;
use crate::hub::Hub;
use crate::hubapi::HubApi;
use crate::link::{self, LinkRegistry, LinkedNode, Presence, RegistryLinks};
use crate::model::*;
use crate::Engine;
use axum::{
    extract::{ws::WebSocketUpgrade, Path, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
    Json, Router,
};
use serde::Serialize;
use std::net::SocketAddr;
use std::sync::Arc;
use tracing::{info, warn};

/// The API version a client can expect from this server. Bumped when a route
/// changes shape, the same way the node API's is.
pub const SERVER_API_VERSION: u32 = 1;

#[derive(Clone)]
pub struct ServerState {
    pub registry: Arc<LinkRegistry>,
    pub token: Arc<String>,
    /// The hub, when this server holds one. `None` keeps the read-only server
    /// of the first half working: a roster and the nodes' boards, no columns
    /// and no actions.
    pub hub: Option<Arc<Hub>>,
}

/// What `/kari/health` says. Enough for a client to refuse a server it cannot
/// speak to, and for a probe to tell a live server from an open port.
#[derive(Debug, Serialize)]
pub struct ServerIdentity {
    pub ok: bool,
    pub app: &'static str,
    pub version: String,
    pub api_version: u32,
    pub link_protocol: u32,
    /// Nodes linked right now. A quick answer to "is anything connected?".
    pub nodes_online: usize,
}

/// One node's board as the server fetched it, or why it could not.
#[derive(Debug, Serialize)]
pub struct NodeBoard {
    pub node_id: String,
    pub node_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub board: Option<BoardView>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Every node and every board the server could get, in one answer.
#[derive(Debug, Serialize)]
pub struct ServerBoard {
    pub nodes: Vec<LinkedNode>,
    pub boards: Vec<NodeBoard>,
    pub generated_at: chrono::DateTime<chrono::Utc>,
}

struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

fn authorized(st: &ServerState, headers: &HeaderMap) -> bool {
    headers
        .get(TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == st.token.as_str())
}

async fn require_token(State(st): State<ServerState>, req: Request, next: Next) -> Response {
    if !authorized(&st, req.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(req).await
}

async fn health(State(st): State<ServerState>) -> Json<ServerIdentity> {
    Json(ServerIdentity {
        ok: true,
        app: "kari-server",
        version: crate::version().into(),
        api_version: SERVER_API_VERSION,
        link_protocol: link::PROTOCOL_VERSION,
        nodes_online: st.registry.open_links().len(),
    })
}

/// A node dialling in. The token was checked by the middleware, so by the time
/// the socket is upgraded the caller has already proved it belongs here.
async fn link_upgrade(State(st): State<ServerState>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| link::serve_link(socket, st.registry))
}

async fn nodes(State(st): State<ServerState>) -> Json<Vec<LinkedNode>> {
    Json(st.registry.nodes())
}

/// The board of one node, fetched over its link.
async fn node_board(
    State(st): State<ServerState>,
    Path(id): Path<String>,
) -> Result<Json<BoardView>, ApiError> {
    let Some(link) = st.registry.get(&id) else {
        return Err(ApiError(
            StatusCode::NOT_FOUND,
            format!("no node {id} is linked"),
        ));
    };
    match link.call::<BoardView>("GET", "/kari/v1/board", None).await {
        Ok(b) => Ok(Json(b)),
        Err(e) => Err(ApiError(StatusCode::BAD_GATEWAY, e.to_string())),
    }
}

/// Every linked node's board, fetched at once.
///
/// One node that is slow or broken must not cost the others their place on the
/// board, so each answer is kept beside its node and a failure becomes an
/// `error` on that row rather than an error for the whole call.
async fn board(State(st): State<ServerState>) -> Json<ServerBoard> {
    let links = st.registry.open_links();
    let fetches = links.into_iter().map(|l| async move {
        let node_id = l.node_id().to_string();
        let node_name = l.identity().node_name.clone();
        match l.call::<BoardView>("GET", "/kari/v1/board", None).await {
            Ok(b) => NodeBoard {
                node_id,
                node_name,
                board: Some(b),
                error: None,
            },
            Err(e) => {
                warn!("board from {node_name} failed: {e}");
                NodeBoard {
                    node_id,
                    node_name,
                    board: None,
                    error: Some(e.to_string()),
                }
            }
        }
    });
    let mut boards = futures_util::future::join_all(fetches).await;
    boards.sort_by(|a, b| a.node_name.cmp(&b.node_name));
    Json(ServerBoard {
        nodes: st.registry.nodes(),
        boards,
        generated_at: chrono::Utc::now(),
    })
}

// --------------------------------------------------------------- the hub API
//
// The routes the desktop app and the phone call instead of running a hub of
// their own. One per `HubApi` method, node-scoped in the path where the method
// takes a node, so a client is a thin translation and holds no board logic.

/// The hub, or an answer saying this server does not have one.
fn hub(st: &ServerState) -> Result<&Arc<Hub>, ApiError> {
    st.hub.as_ref().ok_or_else(|| {
        ApiError(
            StatusCode::NOT_IMPLEMENTED,
            "this server keeps no hub: it serves the roster and the nodes' \
             boards only"
                .into(),
        )
    })
}

fn failed(e: anyhow::Error) -> ApiError {
    ApiError(StatusCode::BAD_GATEWAY, e.to_string())
}

async fn hub_board(State(st): State<ServerState>) -> Result<Json<HubBoard>, ApiError> {
    let h = Arc::clone(hub(&st)?);
    Ok(Json(blocking(move || Ok(h.board())).await?))
}

async fn hub_nodes(State(st): State<ServerState>) -> Result<Json<Vec<NodeStatus>>, ApiError> {
    let h = Arc::clone(hub(&st)?);
    Ok(Json(blocking(move || Ok(h.nodes())).await?))
}

async fn hub_refresh(State(st): State<ServerState>) -> Result<StatusCode, ApiError> {
    let h = Arc::clone(hub(&st)?);
    blocking(move || {
        h.refresh_all();
        Ok(())
    })
    .await?;
    Ok(StatusCode::ACCEPTED)
}

async fn hub_columns(State(st): State<ServerState>) -> Result<Json<Vec<Column>>, ApiError> {
    let h = Arc::clone(hub(&st)?);
    Ok(Json(blocking(move || Ok(h.columns())).await?))
}

async fn hub_set_columns(
    State(st): State<ServerState>,
    Json(cols): Json<Vec<Column>>,
) -> Result<StatusCode, ApiError> {
    let h = Arc::clone(hub(&st)?);
    blocking(move || h.set_columns(cols)).await?;
    Ok(StatusCode::NO_CONTENT)
}

/// Every hub call blocks — it can end up waiting on a node — and this is an
/// async server, so none of them may run on a runtime worker. `LinkedClient`
/// blocks a thread on the runtime to reach a node, and doing that from a
/// worker would deadlock it.
async fn blocking<T, F>(f: F) -> Result<T, ApiError>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(v),
        Ok(Err(e)) => Err(failed(e)),
        Err(e) => Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

/// The whole server API. Health is open; everything else needs the token.
pub fn router(registry: Arc<LinkRegistry>, token: String, hub: Option<Arc<Hub>>) -> Router {
    let st = ServerState {
        registry,
        token: Arc::new(token),
        hub,
    };
    let v1 = Router::new()
        .route(
            link::LINK_PATH.trim_start_matches("/kari/v1"),
            get(link_upgrade),
        )
        .route("/nodes", get(nodes))
        .route("/nodes/{id}/board", get(node_board))
        .route("/board", get(board))
        .route("/hub/board", get(hub_board))
        .route("/hub/nodes", get(hub_nodes))
        .route("/hub/refresh", post(hub_refresh))
        .route("/hub/columns", get(hub_columns).put(hub_set_columns));
    let guarded = Router::new()
        .nest("/kari/v1", v1)
        .route_layer(middleware::from_fn_with_state(st.clone(), require_token));
    Router::new()
        .route("/kari/health", get(health))
        // The root answers the same thing. Somebody who opens the server's URL
        // in a browser, or a health probe that knows nothing about kari, should
        // learn what this is rather than meet a 404.
        .route("/", get(health))
        .merge(guarded)
        .with_state(st)
}

/// Build the hub this server serves, and keep it in step with the links.
///
/// The store is the server's own: it holds the columns, and the last board of
/// every node so an absent node is shown dimmed rather than dropped. The engine
/// is a store only — a server runs no Claude Code and is not a node.
pub fn hub_over_links(store: Arc<Engine>, registry: Arc<LinkRegistry>) -> Arc<Hub> {
    let links = Arc::new(RegistryLinks::new(
        Arc::clone(&registry),
        tokio::runtime::Handle::current(),
    ));
    let hub = Hub::over_links(store, links);

    // A node that dials in takes its place on the board, and one that goes
    // away keeps its row. Watching presence rather than polling means the
    // board changes at the moment the socket does.
    let h = Arc::clone(&hub);
    let mut rx = registry.subscribe_presence();
    std::thread::Builder::new()
        .name("kari-server-presence".into())
        .spawn(move || {
            loop {
                match rx.blocking_recv() {
                    Ok(Presence {
                        node_id,
                        node_name,
                        online: true,
                    }) => {
                        info!("node {node_name} joined the board");
                        h.link_arrived(&node_id, &node_name);
                    }
                    Ok(Presence {
                        node_id, node_name, ..
                    }) => {
                        info!("node {node_name} left the board");
                        h.link_lost(&node_id);
                    }
                    // Dropped presence changes would leave the board wrong in
                    // a way no later event corrects, so re-read the links.
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!("missed {n} link changes; re-reading the roster");
                        for l in registry.open_links() {
                            h.link_arrived(l.node_id(), &l.identity().node_name);
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                }
            }
        })
        .expect("spawn");
    hub
}

/// True for an address it is safe to bind without saying so out loud: loopback,
/// or one of the private ranges. The server carries no TLS and one token, so
/// the private network is what keeps it honest.
pub fn is_safe_bind(addr: &SocketAddr) -> bool {
    addr.ip().is_loopback() || addr.ip().is_unspecified() || crate::net::is_private(&addr.ip())
}

/// Bind and serve until the process ends.
///
/// A public address is refused unless `allow_public` is set, and that flag is
/// not a supported deployment: it exists so the refusal can be overridden by
/// someone who has read why it is there.
pub async fn serve(
    registry: Arc<LinkRegistry>,
    addr: SocketAddr,
    token: String,
    allow_public: bool,
    hub: Option<Arc<Hub>>,
) -> anyhow::Result<()> {
    if !is_safe_bind(&addr) && !allow_public {
        anyhow::bail!(
            "{addr} is a public address. The server carries no TLS and one token, \
             so it belongs on a private network — put it behind a VPN, or pass \
             --allow-public if you know what you are doing"
        );
    }
    if addr.ip().is_unspecified() {
        info!("binding {addr}: every address on this host, private and public");
    }
    let app = router(registry, token, hub);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("kari-server {} listening on {addr}", crate::version());
    axum::serve(listener, app).await?;
    Ok(())
}
