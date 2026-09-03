use kari_core::hub::{Hub, HubEvent};
use kari_core::{
    Calibration, Card, CardPatch, Column, Engine, HubBoard, NewNode, NewTask, NodePatch,
    NodeStatus, Proposal, QuotaSample, Settings, Summary,
};
use std::sync::Arc;
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

struct AppState {
    hub: Arc<Hub>,
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

#[tauri::command]
async fn get_board(state: State<'_, AppState>) -> R<HubBoard> {
    off_thread(&state.hub, |h| Ok(h.board())).await
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
async fn fetch_usage_now() -> R<QuotaSample> {
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

// ---------------------------------------------------------------- shell

fn show_main(app: &AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

/// Show the counts that matter on the tray icon.
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
    let mut last_tray = std::time::Instant::now() - std::time::Duration::from_secs(10);
    tauri::async_runtime::spawn(async move {
        loop {
            match rx.recv().await {
                Ok(HubEvent::BoardChanged { node_id }) => {
                    let _ = app.emit("board_changed", serde_json::json!({ "node_id": node_id }));
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
    let hub = Hub::new(Arc::clone(&engine));

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_opener::init())
        .manage(AppState {
            hub: Arc::clone(&hub),
        })
        .setup(move |app| {
            // The API serves the hook relay on this Mac and, over an SSH
            // forward, any other kari that treats this Mac as a node.
            let port = engine.settings().hooks_port;
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
            kari_paths,
            list_nodes,
            add_node,
            update_node,
            remove_node,
            pair_node
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
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
