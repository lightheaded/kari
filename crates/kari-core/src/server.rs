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
use crate::link::{self, LinkRegistry, LinkedNode};
use crate::model::BoardView;
use axum::{
    extract::{ws::WebSocketUpgrade, Path, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::get,
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

/// The whole server API. Health is open; everything else needs the token.
pub fn router(registry: Arc<LinkRegistry>, token: String) -> Router {
    let st = ServerState {
        registry,
        token: Arc::new(token),
    };
    let v1 = Router::new()
        .route(
            link::LINK_PATH.trim_start_matches("/kari/v1"),
            get(link_upgrade),
        )
        .route("/nodes", get(nodes))
        .route("/nodes/{id}/board", get(node_board))
        .route("/board", get(board));
    let guarded = Router::new()
        .nest("/kari/v1", v1)
        .route_layer(middleware::from_fn_with_state(st.clone(), require_token));
    Router::new()
        .route("/kari/health", get(health))
        .merge(guarded)
        .with_state(st)
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
    let app = router(registry, token);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    info!("kari-server {} listening on {addr}", crate::version());
    axum::serve(listener, app).await?;
    Ok(())
}
