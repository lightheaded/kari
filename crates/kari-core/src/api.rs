//! The HTTP API of a kari node.
//!
//! One route per engine method, JSON in and out, under `/kari/v1/`. The hook
//! receiver (`/kari/hook`) and the health probe (`/kari/health`) live beside
//! it. The desktop app serves this router for its hooks, and `kari-node serve`
//! serves it as a headless node. Every route except health needs the token
//! from `~/.config/kari/hook-token` in the `x-kari-token` header.

use crate::model::*;
use crate::{hooks, Engine, Event};
use axum::{
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::{
        sse::{Event as SseEvent, KeepAlive, Sse},
        IntoResponse, Response,
    },
    routing::{get, post},
    Json, Router,
};
use futures_util::stream::{Stream, StreamExt};
use serde::Deserialize;
use std::future::IntoFuture;
use std::net::SocketAddr;
use std::sync::Arc;
use tokio_stream::wrappers::BroadcastStream;
use tracing::{info, warn};

#[derive(Clone)]
pub struct ApiState {
    pub engine: Arc<Engine>,
    pub token: Arc<String>,
}

/// An error the client can show: status plus one line.
pub struct ApiError(StatusCode, String);

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (self.0, Json(serde_json::json!({ "error": self.1 }))).into_response()
    }
}

fn bad(e: anyhow::Error) -> ApiError {
    ApiError(StatusCode::BAD_REQUEST, e.to_string())
}

type R<T> = Result<Json<T>, ApiError>;

fn authorized(st: &ApiState, headers: &HeaderMap) -> bool {
    headers
        .get(hooks::TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == st.token.as_str())
}

async fn require_token(State(st): State<ApiState>, req: Request, next: Next) -> Response {
    if !authorized(&st, req.headers()) {
        return StatusCode::UNAUTHORIZED.into_response();
    }
    next.run(req).await
}

/// Run an engine call off the async runtime. The engine takes locks and talks
/// to files, sockets and child processes.
async fn blocking<T, F>(f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce() -> anyhow::Result<T> + Send + 'static,
{
    match tokio::task::spawn_blocking(f).await {
        Ok(Ok(v)) => Ok(Json(v)),
        Ok(Err(e)) => Err(bad(e)),
        Err(e) => Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, e.to_string())),
    }
}

#[derive(Deserialize, Default)]
struct Limit {
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct MoveBody {
    column_id: String,
}

/// The new order of one column. `ranked` holds the cards the user placed, top
/// first. `unranked` holds the rest of that column, which sort automatically.
#[derive(Deserialize)]
struct ReorderBody {
    ranked: Vec<String>,
    #[serde(default)]
    unranked: Vec<String>,
}

#[derive(Deserialize, Default)]
struct StartBody {
    prompt: Option<String>,
}

#[derive(Deserialize, Default)]
struct AcceptBody {
    card_ids: Option<Vec<String>>,
}

#[derive(Deserialize)]
struct SnoozeBody {
    minutes: i64,
}

#[derive(Deserialize)]
struct ReleaseBody {
    hub_id: String,
}

fn hub_of(headers: &HeaderMap) -> Option<String> {
    headers
        .get(hooks::HUB_HEADER)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

// ---------------------------------------------------------------- handlers

async fn health(State(st): State<ApiState>) -> Json<NodeIdentity> {
    Json(st.engine.identity())
}

/// Take a hook payload. A permission request in Away mode is held: the
/// response waits for an answer from the API, or for the hold to run out,
/// and carries the decision the relay prints for Claude Code.
async fn hook(
    State(st): State<ApiState>,
    Json(payload): Json<serde_json::Value>,
) -> Json<serde_json::Value> {
    if let Err(e) = st.engine.ingest_hook(payload.clone()) {
        warn!("hook rejected: {e}");
        return Json(serde_json::json!({}));
    }
    let Some(rx) = st.engine.hold_permission(&payload) else {
        return Json(serde_json::json!({}));
    };
    let hold = st
        .engine
        .settings()
        .away_hold_secs
        .clamp(5, hooks::HELD_TIMEOUT_SECS - 30);
    let id = st
        .engine
        .pending_permissions()
        .into_iter()
        .max_by_key(|p| p.since)
        .map(|p| p.id);
    match tokio::time::timeout(std::time::Duration::from_secs(hold), rx).await {
        Ok(Ok(b)) if b == "allow" || b == "deny" => Json(hooks::decision_json(&b)),
        // Cancelled: the session moved on, or nobody answered in time.
        _ => {
            if let Some(id) = id {
                st.engine.drop_permission(&id);
            }
            Json(serde_json::json!({}))
        }
    }
}

async fn permissions(State(st): State<ApiState>) -> Json<Vec<PendingPermission>> {
    Json(st.engine.pending_permissions())
}

async fn answer_permission(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    Json(a): Json<PermissionAnswer>,
) -> R<()> {
    let e = st.engine;
    blocking(move || e.answer_permission(&id, &a.behavior)).await
}

async fn board(State(st): State<ApiState>) -> R<BoardView> {
    let e = st.engine;
    blocking(move || Ok(e.board())).await
}

async fn events(
    State(st): State<ApiState>,
) -> Sse<impl Stream<Item = Result<SseEvent, std::convert::Infallible>>> {
    let rx = st.engine.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|r| async move {
        match r {
            Ok(Event::BoardChanged) => {
                Some(Ok(SseEvent::default().event("board_changed").data("{}")))
            }
            Ok(Event::LeaseChanged) => {
                Some(Ok(SseEvent::default().event("lease_changed").data("{}")))
            }
            Ok(Event::Notice {
                title,
                body,
                card_id,
            }) => SseEvent::default()
                .event("notice")
                .json_data(serde_json::json!({ "title": title, "body": body, "card_id": card_id }))
                .ok()
                .map(Ok),
            // A slow reader lost events. The next board fetch catches up.
            Err(_) => Some(Ok(SseEvent::default().event("board_changed").data("{}"))),
        }
    });
    Sse::new(stream).keep_alive(KeepAlive::default())
}

async fn refresh(State(st): State<ApiState>) -> Json<serde_json::Value> {
    let e = st.engine;
    std::thread::spawn(move || e.refresh_all());
    Json(serde_json::json!({}))
}

async fn add_task(State(st): State<ApiState>, Json(t): Json<NewTask>) -> R<Card> {
    let e = st.engine;
    blocking(move || e.add_task(t)).await
}

async fn patch_card(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    Json(p): Json<CardPatch>,
) -> R<Card> {
    let e = st.engine;
    blocking(move || e.patch_card(&id, p)).await
}

async fn delete_card(State(st): State<ApiState>, Path(id): Path<String>) -> R<()> {
    let e = st.engine;
    blocking(move || e.delete_card(&id)).await
}

async fn reorder_cards(State(st): State<ApiState>, Json(b): Json<ReorderBody>) -> R<()> {
    let e = st.engine;
    blocking(move || e.reorder_cards(&b.ranked, &b.unranked)).await
}

async fn move_card(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    Json(b): Json<MoveBody>,
) -> R<()> {
    let e = st.engine;
    blocking(move || e.move_card(&id, &b.column_id)).await
}

async fn start_card(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<StartBody>>,
) -> R<String> {
    let e = st.engine;
    let prompt = body.and_then(|b| b.0.prompt);
    blocking(move || e.start_card(&id, prompt)).await
}

async fn stop_card(State(st): State<ApiState>, Path(id): Path<String>) -> R<()> {
    let e = st.engine;
    blocking(move || e.stop_card(&id)).await
}

async fn summarize_card(State(st): State<ApiState>, Path(id): Path<String>) -> R<Summary> {
    let e = st.engine;
    blocking(move || e.summarize_card(&id)).await
}

async fn jump(State(st): State<ApiState>, Path(id): Path<String>) -> R<JumpPlan> {
    let e = st.engine;
    blocking(move || e.jump_plan(&id)).await
}

async fn job_log(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    Query(q): Query<Limit>,
) -> R<Vec<JobLogEntry>> {
    let e = st.engine;
    blocking(move || Ok(e.job_log(&id, q.limit.unwrap_or(40)))).await
}

async fn get_columns(State(st): State<ApiState>) -> R<Vec<Column>> {
    let e = st.engine;
    blocking(move || Ok(e.columns())).await
}

/// Only the hub that holds this node's lease may push columns.
fn require_lease(e: &Engine, headers: &HeaderMap) -> Result<(), ApiError> {
    let hub = hub_of(headers);
    if e.lease_allows(hub.as_deref()) {
        return Ok(());
    }
    let holder = e
        .lease()
        .map(|l| l.hub_name)
        .unwrap_or_else(|| "another hub".into());
    Err(ApiError(
        StatusCode::CONFLICT,
        format!("not primary: {holder} holds the lease"),
    ))
}

async fn set_columns(
    State(st): State<ApiState>,
    headers: HeaderMap,
    Json(cols): Json<Vec<Column>>,
) -> R<()> {
    require_lease(&st.engine, &headers)?;
    let e = st.engine;
    blocking(move || e.set_columns(cols)).await
}

async fn reset_columns(State(st): State<ApiState>, headers: HeaderMap) -> R<()> {
    require_lease(&st.engine, &headers)?;
    let e = st.engine;
    blocking(move || e.reset_columns()).await
}

async fn get_lease(State(st): State<ApiState>) -> Json<Option<Lease>> {
    Json(st.engine.lease())
}

async fn claim_lease(State(st): State<ApiState>, Json(c): Json<LeaseClaim>) -> R<Lease> {
    let e = st.engine;
    match tokio::task::spawn_blocking(move || e.claim_lease(c)).await {
        Ok(Ok(l)) => Ok(Json(l)),
        // A live holder refused the claim. 409, so the hub knows it is not a bug.
        Ok(Err(err)) => Err(ApiError(StatusCode::CONFLICT, err.to_string())),
        Err(err) => Err(ApiError(StatusCode::INTERNAL_SERVER_ERROR, err.to_string())),
    }
}

async fn release_lease(State(st): State<ApiState>, Json(b): Json<ReleaseBody>) -> R<()> {
    let e = st.engine;
    blocking(move || e.release_lease(&b.hub_id)).await
}

async fn get_settings(State(st): State<ApiState>) -> Json<Settings> {
    Json(st.engine.settings())
}

async fn set_settings(State(st): State<ApiState>, Json(s): Json<Settings>) -> R<()> {
    let e = st.engine;
    blocking(move || e.set_settings(s)).await
}

async fn get_proposal(State(st): State<ApiState>) -> Json<Option<Proposal>> {
    Json(st.engine.proposal())
}

async fn propose_now(State(st): State<ApiState>) -> R<Proposal> {
    let e = st.engine;
    blocking(move || e.propose_now()).await
}

async fn proposal_history(State(st): State<ApiState>, Query(q): Query<Limit>) -> R<Vec<Proposal>> {
    let e = st.engine;
    blocking(move || Ok(e.proposal_history(q.limit.unwrap_or(20)))).await
}

async fn accept_proposal(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    body: Option<Json<AcceptBody>>,
) -> R<usize> {
    let e = st.engine;
    let ids = body.and_then(|b| b.0.card_ids);
    blocking(move || e.accept_proposal(&id, ids, false)).await
}

async fn snooze_proposal(
    State(st): State<ApiState>,
    Path(id): Path<String>,
    Json(b): Json<SnoozeBody>,
) -> R<()> {
    let e = st.engine;
    blocking(move || e.snooze_proposal(&id, b.minutes)).await
}

async fn dismiss_proposal(State(st): State<ApiState>, Path(id): Path<String>) -> R<()> {
    let e = st.engine;
    blocking(move || e.dismiss_proposal(&id)).await
}

async fn stop_proposal(State(st): State<ApiState>, Path(id): Path<String>) -> R<usize> {
    let e = st.engine;
    blocking(move || e.stop_proposal(&id)).await
}

async fn quota(State(st): State<ApiState>, Query(q): Query<Limit>) -> R<Vec<QuotaSample>> {
    let e = st.engine;
    blocking(move || Ok(e.quota_history(q.limit.unwrap_or(200)))).await
}

async fn calibration(State(st): State<ApiState>) -> Json<Calibration> {
    Json(st.engine.calibration())
}

async fn projects(State(st): State<ApiState>) -> R<Vec<Project>> {
    let e = st.engine;
    blocking(move || Ok(e.projects())).await
}

async fn stop_all(State(st): State<ApiState>) -> R<usize> {
    let e = st.engine;
    blocking(move || e.stop_all()).await
}

// ---------------------------------------------------------------- router

/// The full router: health without a token, everything else behind it.
pub fn router(engine: Arc<Engine>, token: String) -> Router {
    let st = ApiState {
        engine,
        token: Arc::new(token),
    };
    let v1 = Router::new()
        .route("/board", get(board))
        .route("/events", get(events))
        .route("/refresh", post(refresh))
        .route("/cards", post(add_task))
        .route("/cards/reorder", post(reorder_cards))
        .route(
            "/cards/{id}",
            axum::routing::patch(patch_card).delete(delete_card),
        )
        .route("/cards/{id}/move", post(move_card))
        .route("/cards/{id}/start", post(start_card))
        .route("/cards/{id}/stop", post(stop_card))
        .route("/cards/{id}/summarize", post(summarize_card))
        .route("/cards/{id}/jump", post(jump))
        .route("/cards/{id}/jobs", get(job_log))
        .route("/columns", get(get_columns).put(set_columns))
        .route("/columns/reset", post(reset_columns))
        .route(
            "/lease",
            get(get_lease).post(claim_lease).delete(release_lease),
        )
        .route("/permissions", get(permissions))
        .route("/permissions/{id}", post(answer_permission))
        .route("/settings", get(get_settings).put(set_settings))
        .route("/proposal", get(get_proposal).post(propose_now))
        .route("/proposals", get(proposal_history))
        .route("/proposals/{id}/accept", post(accept_proposal))
        .route("/proposals/{id}/snooze", post(snooze_proposal))
        .route("/proposals/{id}/dismiss", post(dismiss_proposal))
        .route("/proposals/{id}/stop", post(stop_proposal))
        .route("/quota", get(quota))
        .route("/calibration", get(calibration))
        .route("/projects", get(projects))
        .route("/stop-all", post(stop_all));
    let guarded = Router::new()
        .route(hooks::HOOK_PATH, post(hook))
        .route("/kari/board", get(board))
        .nest("/kari/v1", v1)
        .route_layer(middleware::from_fn_with_state(st.clone(), require_token));
    Router::new()
        .route("/kari/health", get(health))
        .merge(guarded)
        .with_state(st)
}

/// Bind and serve until the process ends. Refuses a non-loopback address unless
/// `allow_remote` is set: the transport of choice is an SSH port forward.
pub async fn serve(
    engine: Arc<Engine>,
    addr: SocketAddr,
    allow_remote: bool,
) -> anyhow::Result<()> {
    serve_all(engine, vec![addr], allow_remote).await
}

/// One router on several addresses: loopback for the hook relay, and a private
/// network address for a hub that cannot open an SSH forward. The first
/// address is the one the hook relay posts to.
pub async fn serve_all(
    engine: Arc<Engine>,
    addrs: Vec<SocketAddr>,
    allow_remote: bool,
) -> anyhow::Result<()> {
    let Some(first) = addrs.first().copied() else {
        anyhow::bail!("no address to listen on");
    };
    for addr in &addrs {
        if !addr.ip().is_loopback() && !allow_remote {
            anyhow::bail!(
                "{addr} is not a loopback address; pass --allow-remote to bind it anyway"
            );
        }
    }
    let token = hooks::token()?;
    if let Err(e) = hooks::refresh_script(first.port()) {
        warn!("hook relay script not refreshed: {e}");
    }
    let app = router(engine, token);
    let mut servers = Vec::new();
    for addr in addrs {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!("kari api on http://{addr}");
        servers.push(axum::serve(listener, app.clone()).into_future());
    }
    // The first server to stop ends the whole thing; the process is then on its way out.
    let (r, _, _) = futures_util::future::select_all(servers).await;
    r?;
    Ok(())
}

/// Serve the API on the fixed addresses, and on every private address of this
/// machine while the setting asks for it.
///
/// The private set is read again every 20 seconds: a VPN interface that comes
/// up after the start is bound without a restart, and one that goes away is
/// dropped. A public address is never bound. The node advertises what it bound
/// in its identity, so a hub on a phone finds it without a typed address.
pub async fn serve_dynamic(
    engine: Arc<Engine>,
    fixed: Vec<SocketAddr>,
    allow_remote: bool,
    force_private: bool,
) -> anyhow::Result<()> {
    let Some(first) = fixed.first().copied() else {
        anyhow::bail!("no address to listen on");
    };
    for addr in &fixed {
        if !addr.ip().is_loopback() && !allow_remote {
            anyhow::bail!(
                "{addr} is not a loopback address; pass --allow-remote to bind it anyway"
            );
        }
    }
    let token = hooks::token()?;
    if let Err(e) = hooks::refresh_script(first.port()) {
        warn!("hook relay script not refreshed: {e}");
    }
    let app = router(engine.clone(), token);
    let mut pinned = Vec::new();
    for addr in &fixed {
        let listener = tokio::net::TcpListener::bind(addr).await?;
        info!("kari api on http://{addr}");
        pinned.push(tokio::spawn(
            axum::serve(listener, app.clone()).into_future(),
        ));
    }
    let port = first.port();
    // One graceful-shutdown sender per address that the private scan added.
    let mut extra: std::collections::HashMap<SocketAddr, tokio::sync::oneshot::Sender<()>> =
        Default::default();
    loop {
        if pinned.iter().any(|h| h.is_finished()) {
            anyhow::bail!("the api stopped listening");
        }
        let only = if force_private {
            "*".to_string()
        } else {
            engine.settings().listen_on
        };
        let want: Vec<SocketAddr> = crate::net::private_sockets(port, &only)
            .into_iter()
            .filter(|a| !fixed.contains(a))
            .collect();
        extra.retain(|addr, _| {
            let keep = want.contains(addr);
            if !keep {
                info!("kari api left {addr}");
            }
            keep
        });
        for addr in want {
            if extra.contains_key(&addr) {
                continue;
            }
            match tokio::net::TcpListener::bind(addr).await {
                Ok(listener) => {
                    let (tx, rx) = tokio::sync::oneshot::channel::<()>();
                    let app = app.clone();
                    tokio::spawn(async move {
                        let _ = axum::serve(listener, app)
                            .with_graceful_shutdown(async move {
                                let _ = rx.await;
                            })
                            .await;
                    });
                    info!("kari api on http://{addr}");
                    extra.insert(addr, tx);
                }
                Err(e) => warn!("kari api cannot bind {addr}: {e}"),
            }
        }
        let mut bound: Vec<SocketAddr> = fixed.clone();
        bound.extend(extra.keys().copied());
        crate::net::set_bound(bound);
        tokio::time::sleep(std::time::Duration::from_secs(20)).await;
    }
}

/// The same server as a background task, for the desktop app. Errors are logged.
/// Loopback is fixed; the private addresses follow the setting.
pub fn spawn(engine: Arc<Engine>, port: u16) {
    let fixed = vec![SocketAddr::from(([127, 0, 0, 1], port))];
    tokio::spawn(async move {
        if let Err(e) = serve_dynamic(engine, fixed, false, false).await {
            warn!("kari api stopped: {e}");
        }
    });
}
