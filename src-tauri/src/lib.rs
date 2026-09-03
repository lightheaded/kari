use axum::{
    extract::State as AxState,
    http::{HeaderMap, StatusCode},
    routing::{get, post},
    Json, Router,
};
use kari_core::{
    BoardView, Calibration, Card, CardPatch, Column, Engine, Event, NewTask, Proposal, QuotaSample,
    Settings, Summary,
};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

struct AppState {
    engine: Arc<Engine>,
}

/// The tray keeps its own small state: the stop item, and the arm time that
/// turns "Stop all kari jobs" into a two-step action.
struct TrayState {
    stop_all: MenuItem<tauri::Wry>,
    armed_at: std::sync::Mutex<Option<std::time::Instant>>,
    jobs: std::sync::atomic::AtomicUsize,
}

const ARM_SECONDS: u64 = 10;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

#[tauri::command]
fn get_board(state: State<'_, AppState>) -> BoardView {
    state.engine.board()
}

#[tauri::command]
fn refresh(state: State<'_, AppState>) {
    let e = Arc::clone(&state.engine);
    std::thread::spawn(move || e.refresh_all());
}

#[tauri::command]
fn move_card(state: State<'_, AppState>, card_id: String, column_id: String) -> R<()> {
    state.engine.move_card(&card_id, &column_id).map_err(err)
}

#[tauri::command]
fn add_task(state: State<'_, AppState>, task: NewTask) -> R<Card> {
    state.engine.add_task(task).map_err(err)
}

#[tauri::command]
fn patch_card(state: State<'_, AppState>, card_id: String, patch: CardPatch) -> R<Card> {
    state.engine.patch_card(&card_id, patch).map_err(err)
}

#[tauri::command]
fn delete_card(state: State<'_, AppState>, card_id: String) -> R<()> {
    state.engine.delete_card(&card_id).map_err(err)
}

#[tauri::command]
fn get_columns(state: State<'_, AppState>) -> Vec<Column> {
    state.engine.columns()
}

#[tauri::command]
fn set_columns(state: State<'_, AppState>, columns: Vec<Column>) -> R<()> {
    state.engine.set_columns(columns).map_err(err)
}

#[tauri::command]
fn reset_columns(state: State<'_, AppState>) -> R<()> {
    state.engine.reset_columns().map_err(err)
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.engine.settings()
}

#[tauri::command]
fn set_settings(state: State<'_, AppState>, settings: Settings) -> R<()> {
    state.engine.set_settings(settings).map_err(err)
}

#[tauri::command]
fn jump_in(state: State<'_, AppState>, card_id: String) -> R<String> {
    state.engine.jump_in(&card_id).map_err(err)
}

#[tauri::command]
fn start_card(state: State<'_, AppState>, card_id: String, prompt: Option<String>) -> R<String> {
    state.engine.start_card(&card_id, prompt).map_err(err)
}

#[tauri::command]
fn stop_card(state: State<'_, AppState>, card_id: String) -> R<()> {
    state.engine.stop_card(&card_id).map_err(err)
}

#[tauri::command]
fn stop_all(state: State<'_, AppState>) -> R<usize> {
    state.engine.stop_all().map_err(err)
}

#[tauri::command]
fn quota_history(state: State<'_, AppState>, limit: usize) -> Vec<QuotaSample> {
    state.engine.quota_history(limit)
}

#[tauri::command]
fn list_projects(state: State<'_, AppState>) -> Vec<(String, String)> {
    state.engine.projects()
}

#[tauri::command]
fn statusline_wrapper(original_command: String) -> String {
    kari_core::quota::wrapper_script(&original_command)
}

#[tauri::command]
fn install_hooks(state: State<'_, AppState>) -> R<String> {
    state.engine.install_hooks().map_err(err)
}

#[tauri::command]
fn uninstall_hooks(state: State<'_, AppState>) -> R<()> {
    state.engine.uninstall_hooks().map_err(err)
}

#[tauri::command]
async fn summarize_card(state: State<'_, AppState>, card_id: String) -> R<Summary> {
    let e = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || e.summarize_card(&card_id).map_err(err))
        .await
        .map_err(err)?
}

#[tauri::command]
fn job_log(
    state: State<'_, AppState>,
    card_id: String,
    limit: usize,
) -> Vec<kari_core::JobLogEntry> {
    state.engine.job_log(&card_id, limit)
}

#[tauri::command]
fn get_proposal(state: State<'_, AppState>) -> Option<Proposal> {
    state.engine.proposal()
}

#[tauri::command]
async fn propose_now(state: State<'_, AppState>) -> R<Proposal> {
    let e = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || e.propose_now().map_err(err))
        .await
        .map_err(err)?
}

#[tauri::command]
async fn accept_proposal(
    state: State<'_, AppState>,
    proposal_id: String,
    card_ids: Option<Vec<String>>,
) -> R<usize> {
    let e = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || {
        e.accept_proposal(&proposal_id, card_ids, false)
            .map_err(err)
    })
    .await
    .map_err(err)?
}

#[tauri::command]
fn snooze_proposal(state: State<'_, AppState>, proposal_id: String, minutes: i64) -> R<()> {
    state
        .engine
        .snooze_proposal(&proposal_id, minutes)
        .map_err(err)
}

#[tauri::command]
fn dismiss_proposal(state: State<'_, AppState>, proposal_id: String) -> R<()> {
    state.engine.dismiss_proposal(&proposal_id).map_err(err)
}

#[tauri::command]
async fn stop_proposal(state: State<'_, AppState>, proposal_id: String) -> R<usize> {
    let e = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(move || e.stop_proposal(&proposal_id).map_err(err))
        .await
        .map_err(err)?
}

#[tauri::command]
fn proposal_history(state: State<'_, AppState>, limit: usize) -> Vec<Proposal> {
    state.engine.proposal_history(limit)
}

#[tauri::command]
fn get_calibration(state: State<'_, AppState>) -> Calibration {
    state.engine.calibration()
}

/// Ask the OAuth usage endpoint once, outside the stale check. For a manual test.
#[tauri::command]
async fn fetch_usage_now(state: State<'_, AppState>) -> R<QuotaSample> {
    let _ = Arc::clone(&state.engine);
    tauri::async_runtime::spawn_blocking(|| kari_core::quota::fetch_usage().map_err(err))
        .await
        .map_err(err)?
}

#[tauri::command]
fn kari_paths() -> serde_json::Value {
    serde_json::json!({
        "kari_dir": kari_core::paths::kari_dir(),
        "db": kari_core::paths::kari_db(),
        "rate_limits": kari_core::paths::rate_limits_file(),
        "claude_dir": kari_core::paths::claude_dir(),
        "herdr_socket": kari_core::paths::herdr_socket(),
        "version": kari_core::version(),
    })
}

/// The receiver state: the engine and the shared secret from the token file.
#[derive(Clone)]
struct Receiver {
    engine: Arc<Engine>,
    token: Arc<String>,
}

/// Only a caller that read the token file may post events or read the board.
fn authorized(rx: &Receiver, headers: &HeaderMap) -> bool {
    headers
        .get(kari_core::hooks::TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v == rx.token.as_str())
}

/// Hook receiver: Claude Code posts hook payloads here through the relay script.
async fn hook_handler(
    AxState(rx): AxState<Receiver>,
    headers: HeaderMap,
    Json(payload): Json<serde_json::Value>,
) -> Result<Json<serde_json::Value>, StatusCode> {
    if !authorized(&rx, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    if let Err(e) = rx.engine.ingest_hook(payload) {
        tracing::warn!("hook rejected: {e}");
    }
    Ok(Json(serde_json::json!({})))
}

/// Read-only board for scripts and tests. Send the token from
/// `~/.config/kari/hook-token` in the `x-kari-token` header.
async fn board_json(
    AxState(rx): AxState<Receiver>,
    headers: HeaderMap,
) -> Result<Json<BoardView>, StatusCode> {
    if !authorized(&rx, &headers) {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(Json(rx.engine.board()))
}

async fn health() -> Json<serde_json::Value> {
    Json(serde_json::json!({ "ok": true, "app": "kari", "version": kari_core::version() }))
}

fn start_hook_server(engine: Arc<Engine>) {
    let port = engine.settings().hooks_port;
    let token = match kari_core::hooks::token() {
        Ok(t) => t,
        Err(e) => {
            tracing::warn!("hook receiver has no token, it stays off: {e}");
            return;
        }
    };
    if let Err(e) = kari_core::hooks::refresh_script(port) {
        tracing::warn!("hook relay script not refreshed: {e}");
    }
    let rx = Receiver {
        engine,
        token: Arc::new(token),
    };
    tauri::async_runtime::spawn(async move {
        let app = Router::new()
            .route(kari_core::hooks::HOOK_PATH, post(hook_handler))
            .route("/kari/health", get(health))
            .route("/kari/board", get(board_json))
            .with_state(rx);
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        match tokio::net::TcpListener::bind(addr).await {
            Ok(listener) => {
                tracing::info!("hook receiver on http://{addr}");
                if let Err(e) = axum::serve(listener, app).await {
                    tracing::warn!("hook receiver stopped: {e}");
                }
            }
            Err(e) => tracing::warn!("hook receiver cannot bind {addr}: {e}"),
        }
    });
}

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Show the counts that matter on the tray icon.
fn update_tray(app: &AppHandle, engine: &Arc<Engine>) {
    let board = engine.board();
    let working = board
        .cards
        .iter()
        .filter(|c| c.state == kari_core::DerivedState::Working)
        .count();
    let need = board
        .cards
        .iter()
        .filter(|c| {
            matches!(
                c.state,
                kari_core::DerivedState::NeedsDecision | kari_core::DerivedState::NeedsApproval
            )
        })
        .count();
    let jobs = board
        .cards
        .iter()
        .filter(|c| {
            c.card.bg_job_id.is_some()
                && c.bg_job.as_ref().and_then(|j| j.state.as_deref()) == Some("working")
        })
        .count();
    if let Some(tray) = app.tray_by_id("kari-tray") {
        let _ = tray.set_tooltip(Some(format!("kari — {working} working · {need} need you")));
    }
    if let Some(ts) = app.try_state::<TrayState>() {
        ts.jobs.store(jobs, std::sync::atomic::Ordering::Relaxed);
        let armed = ts
            .armed_at
            .lock()
            .unwrap()
            .is_some_and(|t| t.elapsed().as_secs() < ARM_SECONDS);
        if !armed {
            let _ = ts.stop_all.set_text(if jobs > 0 {
                format!("Stop all kari jobs ({jobs})")
            } else {
                "Stop all kari jobs".to_string()
            });
        }
    }
}

fn forward_events(app: AppHandle, engine: Arc<Engine>) {
    let mut rx = engine.subscribe();
    let mut last_tray = std::time::Instant::now() - std::time::Duration::from_secs(10);
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(Event::BoardChanged) => {
                    let _ = app.emit("board_changed", ());
                    if last_tray.elapsed().as_secs() >= 3 {
                        last_tray = std::time::Instant::now();
                        update_tray(&app, &engine);
                    }
                }
                Ok(Event::Notice {
                    title,
                    body,
                    card_id,
                }) => {
                    let _ = app.emit(
                        "notice",
                        serde_json::json!({"title": title, "body": body, "card_id": card_id}),
                    );
                    let _ = app
                        .notification()
                        .builder()
                        .title(&title)
                        .body(&body)
                        .show();
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kari_core=info".parse().unwrap()),
        )
        .init();

    let engine = Engine::open().expect("open kari store");
    engine.start_watchers();
    start_hook_server(Arc::clone(&engine));

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            engine: Arc::clone(&engine),
        })
        .setup(move |app| {
            forward_events(app.handle().clone(), Arc::clone(&engine));

            let show = MenuItem::with_id(app, "show", "Open kari", true, None::<&str>)?;
            let refresh_item =
                MenuItem::with_id(app, "refresh", "Refresh now", true, None::<&str>)?;
            let stop_all_item =
                MenuItem::with_id(app, "stop_all", "Stop all kari jobs", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit kari", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &refresh_item, &stop_all_item, &quit])?;
            app.manage(TrayState {
                stop_all: stop_all_item.clone(),
                armed_at: std::sync::Mutex::new(None),
                jobs: std::sync::atomic::AtomicUsize::new(0),
            });
            let eng = Arc::clone(&engine);
            TrayIconBuilder::with_id("kari-tray")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .icon_as_template(true)
                .tooltip("kari")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, ev| match ev.id().as_ref() {
                    "show" => show_main(app),
                    "refresh" => {
                        let e = Arc::clone(&eng);
                        std::thread::spawn(move || e.refresh_all());
                    }
                    // Two steps on purpose: a mis-click must not kill running work.
                    "stop_all" => {
                        let Some(ts) = app.try_state::<TrayState>() else {
                            return;
                        };
                        let n = ts.jobs.load(std::sync::atomic::Ordering::Relaxed);
                        let armed = {
                            let mut g = ts.armed_at.lock().unwrap();
                            match *g {
                                Some(t) if t.elapsed().as_secs() < ARM_SECONDS => {
                                    *g = None;
                                    true
                                }
                                _ => {
                                    *g = Some(std::time::Instant::now());
                                    false
                                }
                            }
                        };
                        if armed {
                            let stopped = eng.stop_all().unwrap_or(0);
                            let _ = ts.stop_all.set_text("Stop all kari jobs");
                            let _ = app
                                .notification()
                                .builder()
                                .title("kari stopped its jobs")
                                .body(format!("{stopped} job(s) stopped."))
                                .show();
                        } else {
                            let _ = ts
                                .stop_all
                                .set_text(format!("Click again to stop {n} job(s)"));
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it. The tray keeps kari alive.
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let _ = window.hide();
                api.prevent_close();
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_board,
            refresh,
            move_card,
            add_task,
            patch_card,
            delete_card,
            get_columns,
            set_columns,
            reset_columns,
            get_settings,
            set_settings,
            jump_in,
            start_card,
            stop_card,
            stop_all,
            quota_history,
            list_projects,
            statusline_wrapper,
            install_hooks,
            uninstall_hooks,
            summarize_card,
            get_calibration,
            fetch_usage_now,
            get_proposal,
            job_log,
            propose_now,
            accept_proposal,
            snooze_proposal,
            dismiss_proposal,
            stop_proposal,
            proposal_history,
            kari_paths
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
