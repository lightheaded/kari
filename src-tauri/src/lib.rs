use kari_core::hub::{Hub, HubEvent};
use kari_core::{
    AutomationMode, Calibration, Card, CardPatch, Column, Engine, HubBoard, NewNode, NewTask,
    NodePatch, NodeStatus, Proposal, QuotaSample, Settings, Summary,
};
use std::sync::Arc;
#[cfg(desktop)]
use tauri::menu::{Menu, MenuItem};
#[cfg(desktop)]
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

struct AppState {
    hub: Arc<Hub>,
    /// The window holds unsaved input: a task draft, an edited card. A quit
    /// from the tray or from Cmd+Q asks before it throws that away.
    dirty: std::sync::atomic::AtomicBool,
}

/// The tray keeps its own small state: the stop item, and the arm time that
/// turns "Stop all kari jobs" into a two-step action.
#[cfg(desktop)]
struct TrayState {
    stop_all: MenuItem<tauri::Wry>,
    armed_at: std::sync::Mutex<Option<std::time::Instant>>,
    jobs: std::sync::atomic::AtomicUsize,
}

#[cfg(desktop)]
const ARM_SECONDS: u64 = 10;

type R<T> = Result<T, String>;

fn err<E: std::fmt::Display>(e: E) -> String {
    e.to_string()
}

/// Remote nodes answer over the network. Every command that can reach one runs
/// off the main thread.
async fn off_thread<T, F>(hub: &Arc<Hub>, f: F) -> R<T>
where
    T: Send + 'static,
    F: FnOnce(Arc<Hub>) -> anyhow::Result<T> + Send + 'static,
{
    let h = Arc::clone(hub);
    tauri::async_runtime::spawn_blocking(move || f(h).map_err(err))
        .await
        .map_err(err)?
}

/// The board of every node, as a JSON string the caller parses.
///
/// Not an object on purpose. Tauri answers an object-shaped result through a
/// channel, and on Android that channel makes the page fetch the payload over
/// the internal protocol. That fetch does not complete on every device, and a
/// page then waits for a board that never arrives. A string comes back on the
/// direct path instead.
///
/// The phone has no console, so the call also writes a line to the system log
/// when it starts and another with what it answered. `adb logcat | grep kari`
/// shows the pair.
#[tauri::command]
async fn get_board_json(state: State<'_, AppState>) -> R<String> {
    let b = get_board(state).await?;
    serde_json::to_string(&b).map_err(err)
}

#[tauri::command]
async fn get_board(state: State<'_, AppState>) -> R<HubBoard> {
    #[cfg(mobile)]
    let t0 = std::time::Instant::now();
    #[cfg(mobile)]
    tracing::info!("get_board: start");
    let b = off_thread(&state.hub, |h| Ok(h.board())).await;
    #[cfg(mobile)]
    match &b {
        Ok(v) => tracing::info!(
            "get_board: {} cards, {} nodes, {} ms",
            v.cards.len(),
            v.nodes.len(),
            t0.elapsed().as_millis()
        ),
        Err(e) => tracing::warn!(
            "get_board: failed after {} ms: {e}",
            t0.elapsed().as_millis()
        ),
    }
    b
}

#[tauri::command]
fn refresh(state: State<'_, AppState>) {
    let h = Arc::clone(&state.hub);
    std::thread::spawn(move || h.refresh_all());
}

#[tauri::command]
async fn move_card(
    state: State<'_, AppState>,
    node_id: String,
    card_id: String,
    column_id: String,
) -> R<()> {
    off_thread(&state.hub, move |h| {
        h.move_card(&node_id, &card_id, &column_id)
    })
    .await
}

#[tauri::command]
async fn add_task(state: State<'_, AppState>, node_id: String, task: NewTask) -> R<Card> {
    off_thread(&state.hub, move |h| h.add_task(&node_id, task)).await
}

#[tauri::command]
async fn patch_card(
    state: State<'_, AppState>,
    node_id: String,
    card_id: String,
    patch: CardPatch,
) -> R<Card> {
    off_thread(&state.hub, move |h| h.patch_card(&node_id, &card_id, patch)).await
}

#[tauri::command]
async fn delete_card(state: State<'_, AppState>, node_id: String, card_id: String) -> R<()> {
    off_thread(&state.hub, move |h| h.delete_card(&node_id, &card_id)).await
}

/// The frontend reports whether any form holds unsaved input.
#[tauri::command]
fn set_dirty(state: State<'_, AppState>, dirty: bool) {
    state
        .dirty
        .store(dirty, std::sync::atomic::Ordering::Relaxed);
}

/// The user confirmed the quit in the window.
#[tauri::command]
fn quit_now(app: AppHandle) {
    app.exit(0);
}

/// Store a manual order for one column on one node.
#[tauri::command]
async fn reorder_cards(
    state: State<'_, AppState>,
    node_id: String,
    ranked: Vec<String>,
    unranked: Vec<String>,
) -> R<()> {
    off_thread(&state.hub, move |h| {
        h.reorder_cards(&node_id, ranked, unranked)
    })
    .await
}

/// Set how much automatic behaviour is allowed. An empty node id sets every
/// node that answers, and names the ones that did not.
#[tauri::command]
async fn set_automation_mode(
    state: State<'_, AppState>,
    node_id: String,
    mode: String,
) -> R<String> {
    let m = match AutomationMode::parse(&mode) {
        Some(m) => m,
        None => return Err(format!("unknown automation mode {mode}")),
    };
    off_thread(&state.hub, move |h| {
        if node_id.is_empty() {
            let failed = h.set_automation_mode_all(m);
            return Ok(if failed.is_empty() {
                format!("Automation set to {mode}")
            } else {
                format!("Automation set to {mode}, except on {}", failed.join(", "))
            });
        }
        h.set_automation_mode(&node_id, m)?;
        Ok(format!("Automation set to {mode}"))
    })
    .await
}

#[tauri::command]
fn get_columns(state: State<'_, AppState>) -> Vec<Column> {
    state.hub.engine().columns()
}

#[tauri::command]
async fn set_columns(state: State<'_, AppState>, columns: Vec<Column>) -> R<()> {
    off_thread(&state.hub, move |h| h.set_columns(columns)).await
}

#[tauri::command]
async fn reset_columns(state: State<'_, AppState>) -> R<()> {
    off_thread(&state.hub, |h| h.reset_columns()).await
}

#[tauri::command]
fn get_settings(state: State<'_, AppState>) -> Settings {
    state.hub.engine().settings()
}

/// The settings as a JSON string. See `get_board_json` for why.
#[tauri::command]
fn get_settings_json(state: State<'_, AppState>) -> R<String> {
    serde_json::to_string(&state.hub.engine().settings()).map_err(err)
}

#[tauri::command]
fn set_settings(state: State<'_, AppState>, settings: Settings) -> R<()> {
    state.hub.engine().set_settings(settings).map_err(err)
}

#[tauri::command]
async fn jump_in(state: State<'_, AppState>, node_id: String, card_id: String) -> R<String> {
    off_thread(&state.hub, move |h| h.jump_in(&node_id, &card_id)).await
}

#[tauri::command]
async fn start_card(
    state: State<'_, AppState>,
    node_id: String,
    card_id: String,
    prompt: Option<String>,
) -> R<String> {
    off_thread(&state.hub, move |h| {
        h.start_card(&node_id, &card_id, prompt)
    })
    .await
}

#[tauri::command]
async fn stop_card(state: State<'_, AppState>, node_id: String, card_id: String) -> R<()> {
    off_thread(&state.hub, move |h| h.stop_card(&node_id, &card_id)).await
}

#[tauri::command]
async fn stop_all(state: State<'_, AppState>) -> R<usize> {
    off_thread(&state.hub, |h| h.stop_all()).await
}

#[tauri::command]
async fn quota_history(
    state: State<'_, AppState>,
    node_id: String,
    limit: usize,
) -> R<Vec<QuotaSample>> {
    off_thread(&state.hub, move |h| Ok(h.quota_history(&node_id, limit))).await
}

#[tauri::command]
async fn list_projects(state: State<'_, AppState>, node_id: String) -> R<Vec<(String, String)>> {
    off_thread(&state.hub, move |h| Ok(h.projects(&node_id))).await
}

#[tauri::command]
fn statusline_wrapper(original_command: String) -> String {
    kari_core::quota::wrapper_script(&original_command)
}

#[tauri::command]
fn install_hooks(state: State<'_, AppState>) -> R<String> {
    state.hub.engine().install_hooks().map_err(err)
}

#[tauri::command]
fn uninstall_hooks(state: State<'_, AppState>) -> R<()> {
    state.hub.engine().uninstall_hooks().map_err(err)
}

#[tauri::command]
async fn summarize_card(
    state: State<'_, AppState>,
    node_id: String,
    card_id: String,
) -> R<Summary> {
    off_thread(&state.hub, move |h| h.summarize_card(&node_id, &card_id)).await
}

#[tauri::command]
async fn job_log(
    state: State<'_, AppState>,
    node_id: String,
    card_id: String,
    limit: usize,
) -> R<Vec<kari_core::JobLogEntry>> {
    off_thread(
        &state.hub,
        move |h| Ok(h.job_log(&node_id, &card_id, limit)),
    )
    .await
}

#[tauri::command]
async fn get_proposal(state: State<'_, AppState>, node_id: String) -> R<Option<Proposal>> {
    off_thread(&state.hub, move |h| Ok(h.proposal(&node_id))).await
}

#[tauri::command]
async fn propose_now(state: State<'_, AppState>, node_id: String) -> R<Proposal> {
    off_thread(&state.hub, move |h| h.propose_now(&node_id)).await
}

#[tauri::command]
async fn accept_proposal(
    state: State<'_, AppState>,
    node_id: String,
    proposal_id: String,
    card_ids: Option<Vec<String>>,
) -> R<usize> {
    off_thread(&state.hub, move |h| {
        h.accept_proposal(&node_id, &proposal_id, card_ids)
    })
    .await
}

#[tauri::command]
async fn snooze_proposal(
    state: State<'_, AppState>,
    node_id: String,
    proposal_id: String,
    minutes: i64,
) -> R<()> {
    off_thread(&state.hub, move |h| {
        h.snooze_proposal(&node_id, &proposal_id, minutes)
    })
    .await
}

#[tauri::command]
async fn dismiss_proposal(
    state: State<'_, AppState>,
    node_id: String,
    proposal_id: String,
) -> R<()> {
    off_thread(&state.hub, move |h| {
        h.dismiss_proposal(&node_id, &proposal_id)
    })
    .await
}

#[tauri::command]
async fn stop_proposal(
    state: State<'_, AppState>,
    node_id: String,
    proposal_id: String,
) -> R<usize> {
    off_thread(&state.hub, move |h| h.stop_proposal(&node_id, &proposal_id)).await
}

#[tauri::command]
async fn proposal_history(
    state: State<'_, AppState>,
    node_id: String,
    limit: usize,
) -> R<Vec<Proposal>> {
    off_thread(&state.hub, move |h| Ok(h.proposal_history(&node_id, limit))).await
}

#[tauri::command]
fn get_calibration(state: State<'_, AppState>) -> Calibration {
    state.hub.engine().calibration()
}

/// Ask the OAuth usage endpoint once, outside the stale check. For a manual test.
#[tauri::command]
async fn fetch_usage_now(state: State<'_, AppState>) -> R<QuotaSample> {
    let e = state.hub.engine().clone();
    tauri::async_runtime::spawn_blocking(move || e.fetch_usage_now().map_err(err))
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

// ---------------------------------------------------------------- nodes

#[tauri::command]
fn list_nodes(state: State<'_, AppState>) -> Vec<NodeStatus> {
    state.hub.nodes()
}

#[tauri::command]
async fn add_node(state: State<'_, AppState>, node: NewNode) -> R<NodeStatus> {
    off_thread(&state.hub, move |h| h.add_node(node)).await
}

#[tauri::command]
async fn update_node(
    state: State<'_, AppState>,
    node_id: String,
    patch: NodePatch,
) -> R<NodeStatus> {
    off_thread(&state.hub, move |h| h.update_node(&node_id, patch)).await
}

#[tauri::command]
async fn remove_node(state: State<'_, AppState>, node_id: String) -> R<()> {
    off_thread(&state.hub, move |h| h.remove_node(&node_id)).await
}

#[tauri::command]
async fn pair_node(state: State<'_, AppState>, node_id: String) -> R<String> {
    off_thread(&state.hub, move |h| h.pair_node(&node_id)).await
}

/// Answer a permission prompt a node holds for us: allow or deny.
#[tauri::command]
async fn answer_permission(
    state: State<'_, AppState>,
    node_id: String,
    permission_id: String,
    behavior: String,
) -> R<()> {
    off_thread(&state.hub, move |h| {
        h.answer_permission(&node_id, &permission_id, &behavior)
    })
    .await
}

/// Hold permission prompts on a node for a remote answer, or stop.
#[tauri::command]
async fn set_away_mode(state: State<'_, AppState>, node_id: String, on: bool) -> R<()> {
    off_thread(&state.hub, move |h| h.set_away_mode(&node_id, on)).await
}

/// Take the column lease on every node. This device then pushes columns.
#[tauri::command]
async fn claim_primary(state: State<'_, AppState>) -> R<String> {
    #[cfg(mobile)]
    tracing::info!("claim_primary: start");
    off_thread(&state.hub, |h| h.claim_primary()).await
}

/// A pairing code for a phone: every node with its address and token.
#[tauri::command]
async fn pairing_code(state: State<'_, AppState>) -> R<String> {
    off_thread(&state.hub, |h| h.pairing_code()).await
}

// ------------------------------------------------------- window geometry

/// The size and the place of the main window, in logical pixels.
///
/// `tauri-plugin-window-state` does this too, but it hides the window before it
/// restores and shows it again only with its `VISIBLE` flag. kari lives in the
/// tray, so whether the window was open at exit must not decide whether it
/// opens at the next start. Two fields of our own cost less than that trade.
#[cfg(desktop)]
#[derive(serde::Serialize, serde::Deserialize)]
struct Geometry {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

#[cfg(desktop)]
fn geometry_file() -> std::path::PathBuf {
    kari_core::paths::kari_dir().join("window.json")
}

/// Read the last geometry and put the window back. A rectangle that no display
/// covers any more keeps its size and lets the system place it, so a window
/// never opens off screen after a monitor goes away.
#[cfg(desktop)]
fn restore_geometry(window: &tauri::Window) {
    let Ok(text) = std::fs::read_to_string(geometry_file()) else {
        return;
    };
    let Ok(g) = serde_json::from_str::<Geometry>(&text) else {
        return;
    };
    if g.w >= 480.0 && g.h >= 320.0 {
        let _ = window.set_size(tauri::LogicalSize::new(g.w, g.h));
    }
    let on_screen = window.available_monitors().is_ok_and(|ms| {
        ms.iter().any(|m| {
            let s = m.scale_factor();
            let p = m.position();
            let sz = m.size();
            let (mx, my) = (p.x as f64 / s, p.y as f64 / s);
            let (mw, mh) = (sz.width as f64 / s, sz.height as f64 / s);
            // The title bar must be reachable, so ask for the top-left corner
            // plus a little of the width to fall inside this display.
            g.x + 120.0 > mx && g.x < mx + mw && g.y + 24.0 > my && g.y < my + mh
        })
    });
    if on_screen {
        let _ = window.set_position(tauri::LogicalPosition::new(g.x, g.y));
    } else {
        tracing::info!("saved window position is off screen; letting the system place it");
    }
}

/// How many geometry events arrived. A pending write compares its own number
/// with this one and gives way to a newer event.
#[cfg(desktop)]
static GEOMETRY_EVENTS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Write the geometry once the window is quiet.
///
/// A drag sends an event per frame. Writing on each one is wasteful, and
/// writing only the first one loses the size the user settled on, so the write
/// waits for the last event of the run and then happens once.
#[cfg(desktop)]
fn save_geometry(window: &tauri::Window) {
    use std::sync::atomic::Ordering::SeqCst;
    let mine = GEOMETRY_EVENTS.fetch_add(1, SeqCst) + 1;
    let window = window.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(400));
        if GEOMETRY_EVENTS.load(SeqCst) != mine {
            return; // a newer move or resize owns the write
        }
        write_geometry(&window);
    });
}

/// Write the geometry now. The close path uses it, because the window is about
/// to hide and a hidden window reports nothing worth keeping.
#[cfg(desktop)]
fn write_geometry(window: &tauri::Window) {
    // A minimized or hidden window reports a place worth nothing.
    if !window.is_visible().unwrap_or(false) || window.is_minimized().unwrap_or(false) {
        return;
    }
    let (Ok(pos), Ok(size), Ok(scale)) = (
        window.outer_position(),
        window.inner_size(),
        window.scale_factor(),
    ) else {
        return;
    };
    let g = Geometry {
        x: pos.x as f64 / scale,
        y: pos.y as f64 / scale,
        w: size.width as f64 / scale,
        h: size.height as f64 / scale,
    };
    if let Ok(text) = serde_json::to_string(&g) {
        let path = geometry_file();
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(path, text);
    }
}

/// The interfaces of this machine, for the "let a phone reach this machine"
/// picker in Settings.
#[tauri::command]
fn local_addresses() -> Vec<kari_core::net::LocalAddress> {
    kari_core::net::local_addresses()
}

// ---------------------------------------------------------------- shell

#[cfg(desktop)]
fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Quit, unless the window holds unsaved input. Then show the window and let
/// the frontend ask. It calls `quit_now` when the user confirms.
///
/// Returns true when the app is on its way out.
#[cfg(desktop)]
fn request_quit(app: &AppHandle) -> bool {
    let dirty = app
        .try_state::<AppState>()
        .is_some_and(|s| s.dirty.load(std::sync::atomic::Ordering::Relaxed));
    if dirty {
        show_main(app);
        let _ = app.emit("confirm_quit", ());
        return false;
    }
    app.exit(0);
    true
}

/// Show the counts that matter on the tray icon.
#[cfg(desktop)]
fn update_tray(app: &AppHandle, hub: &Arc<Hub>) {
    let board = hub.board();
    let working = board
        .cards
        .iter()
        .filter(|c| c.view.state == kari_core::DerivedState::Working)
        .count();
    let need = board
        .cards
        .iter()
        .filter(|c| {
            matches!(
                c.view.state,
                kari_core::DerivedState::NeedsDecision | kari_core::DerivedState::NeedsApproval
            )
        })
        .count();
    let jobs = board
        .cards
        .iter()
        .filter(|c| {
            c.view.card.bg_job_id.is_some()
                && c.view.bg_job.as_ref().and_then(|j| j.state.as_deref()) == Some("working")
        })
        .count();
    let offline = board
        .nodes
        .iter()
        .filter(|n| n.enabled && !n.online)
        .count();
    if let Some(tray) = app.tray_by_id("kari-tray") {
        let mut tip = format!("kari — {working} working · {need} need you");
        if offline > 0 {
            tip.push_str(&format!(" · {offline} node(s) offline"));
        }
        let _ = tray.set_tooltip(Some(tip));
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

fn forward_events(app: AppHandle, hub: Arc<Hub>) {
    let mut rx = hub.subscribe();
    #[cfg(desktop)]
    let mut last_tray = std::time::Instant::now() - std::time::Duration::from_secs(10);
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(HubEvent::BoardChanged { node_id }) => {
                    let _ = app.emit("board_changed", serde_json::json!({ "node_id": node_id }));
                    #[cfg(desktop)]
                    if last_tray.elapsed().as_secs() >= 3 {
                        last_tray = std::time::Instant::now();
                        let (a, h) = (app.clone(), Arc::clone(&hub));
                        tauri::async_runtime::spawn_blocking(move || update_tray(&a, &h));
                    }
                }
                Ok(HubEvent::Notice {
                    node_id,
                    node_name,
                    title,
                    body,
                    card_id,
                }) => {
                    let shown_title = if node_id == kari_core::hub::LOCAL {
                        title.clone()
                    } else {
                        format!("{node_name} · {title}")
                    };
                    let _ = app.emit(
                        "notice",
                        serde_json::json!({
                            "title": shown_title,
                            "body": body,
                            "card_id": card_id,
                            "node_id": node_id,
                            "node_name": node_name,
                        }),
                    );
                    let _ = app
                        .notification()
                        .builder()
                        .title(&shown_title)
                        .body(&body)
                        .show();
                }
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => continue,
                Err(_) => break,
            }
        }
    });
}

/// Every command the UI can call. One list, used by both shells.
macro_rules! handlers {
    () => {
        tauri::generate_handler![
            get_board,
            get_board_json,
            get_settings_json,
            refresh,
            move_card,
            add_task,
            patch_card,
            delete_card,
            set_dirty,
            quit_now,
            reorder_cards,
            set_automation_mode,
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
            kari_paths,
            list_nodes,
            add_node,
            update_node,
            remove_node,
            pair_node,
            claim_primary,
            answer_permission,
            set_away_mode,
            pairing_code,
            local_addresses,
        ]
    };
}

/// The phone: no Claude Code here, so no engine scan, no API and no tray. The
/// store lives in the app's data directory and the hub shows remote nodes only.
#[cfg(mobile)]
#[tauri::mobile_entry_point]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kari_core=info".parse().unwrap())
                // Without this the app crate's own lines never reach the log,
                // and the log is the only window into a phone.
                .add_directive("kari=info".parse().unwrap()),
        )
        .init();
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let dir = app.path().app_data_dir()?;
            std::fs::create_dir_all(&dir)?;
            let engine = Engine::open_at(&dir)?;
            let hub = Hub::without_local(engine);
            app.manage(AppState {
                hub: Arc::clone(&hub),
                dirty: std::sync::atomic::AtomicBool::new(false),
            });
            forward_events(app.handle().clone(), hub);
            // Android 13 and later ask the user once before the first
            // notification. Not from here: this runs on the thread that also
            // drives the webview, and the message pump that carries every
            // answer into the page stops for good after one failed call on
            // that thread. The ask happens a moment later, off this thread.
            let handle = app.handle().clone();
            std::thread::spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(3));
                match handle.notification().request_permission() {
                    Ok(state) => tracing::info!("notification permission: {state:?}"),
                    Err(e) => tracing::warn!("notification permission not asked: {e}"),
                }
            });
            tracing::info!("mobile setup done");
            Ok(())
        })
        .invoke_handler(handlers!())
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

#[cfg(desktop)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("kari_core=info".parse().unwrap())
                // Without this the app crate's own lines never reach the log.
                .add_directive("kari=info".parse().unwrap()),
        )
        .init();

    let engine = Engine::open().expect("open kari store");
    engine.start_watchers();
    let hub = Hub::new(Arc::clone(&engine));

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            hub: Arc::clone(&hub),
            dirty: std::sync::atomic::AtomicBool::new(false),
        })
        .setup(move |app| {
            // Put the window back where it was, before it is on screen.
            if let Some(w) = app.get_webview_window("main") {
                restore_geometry(&w.as_ref().window());
            }
            // The API serves the hook relay on this Mac and, over an SSH
            // forward, any other kari that treats this Mac as a node.
            let port = engine.settings().hooks_port;
            // The settings decide whether the private addresses are bound too,
            // for a hub on a phone. The listener follows a change on its own.
            let e = Arc::clone(&engine);
            tauri::async_runtime::spawn(async move {
                kari_core::api::spawn(e, port);
            });
            forward_events(app.handle().clone(), Arc::clone(&hub));

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
            let h = Arc::clone(&hub);
            TrayIconBuilder::with_id("kari-tray")
                .icon(app.default_window_icon().cloned().expect("icon"))
                .icon_as_template(true)
                .tooltip("kari")
                .menu(&menu)
                .show_menu_on_left_click(true)
                .on_menu_event(move |app, ev| match ev.id().as_ref() {
                    "show" => show_main(app),
                    "refresh" => {
                        let h = Arc::clone(&h);
                        std::thread::spawn(move || h.refresh_all());
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
                            let stopped = h.stop_all().unwrap_or(0);
                            let _ = ts.stop_all.set_text("Stop all kari jobs");
                            let _ = app
                                .notification()
                                .builder()
                                .title("kari stopped its jobs")
                                .body(format!("{stopped} job(s) stopped on every node."))
                                .show();
                        } else {
                            let _ = ts
                                .stop_all
                                .set_text(format!("Click again to stop {n} job(s)"));
                        }
                    }
                    "quit" => {
                        request_quit(app);
                    }
                    _ => {}
                })
                .build(app)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            match event {
                // Closing the window hides it. The tray keeps kari alive, so
                // the geometry is written now, while the window still has one.
                tauri::WindowEvent::CloseRequested { api, .. } => {
                    write_geometry(window);
                    let _ = window.hide();
                    api.prevent_close();
                }
                tauri::WindowEvent::Moved(_) | tauri::WindowEvent::Resized(_) => {
                    save_geometry(window)
                }
                _ => {}
            }
        })
        .invoke_handler(handlers!())
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Cmd+Q and the app menu ask for an exit with no code. With unsaved
            // input kari keeps running and asks in the window. `quit_now` exits
            // with a code, and that passes.
            #[cfg(desktop)]
            if let tauri::RunEvent::ExitRequested {
                ref api,
                code: None,
                ..
            } = event
            {
                if !request_quit(app) {
                    api.prevent_exit();
                }
            }
            // Closing the window only hides it, so the dock icon must bring it
            // back. macOS sends Reopen for a dock click, and for "Open" again.
            #[cfg(target_os = "macos")]
            if let tauri::RunEvent::Reopen {
                has_visible_windows,
                ..
            } = event
            {
                if !has_visible_windows {
                    show_main(app);
                }
            }
            #[cfg(not(target_os = "macos"))]
            {
                let _ = (app, event);
            }
        });
}
