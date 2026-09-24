//! The notifications of the operating system.
//!
//! On macOS kari posts through `mac-notification-sys` itself, not through the
//! notification plugin, for three things the plugin cannot do:
//!
//! - A click reports back. kari then focuses the herdr pane of the card, or
//!   opens the card in the window. A click on the plugin's notification only
//!   brings kari to the front.
//! - A notice for a session that waits for an answer carries buttons. macOS
//!   shows a notification with buttons as an alert, which stays on screen
//!   until the user acts on it.
//! - kari takes a notification back when its card moves on. So an answer given
//!   in the terminal also clears the alert, and the Notification Center holds
//!   only what still waits.
//!
//! Each notification with a card holds one thread, which waits for the click.
//! The thread ends when the user acts or when kari takes the notification
//! back. A new notice for a card replaces the old one, so a card holds one
//! thread at most.
//!
//! Other platforms show the plugin's notification, as before.

use kari_core::hubapi::HubApi;
use kari_core::DerivedState;
use std::sync::Arc;
use tauri::AppHandle;

/// One notice from the hub, as the shell shows it.
pub struct Notice {
    pub node_id: String,
    pub card_id: Option<String>,
    pub title: String,
    pub body: String,
    pub sticky: bool,
}

/// True while a notification about a card in `state` still says something
/// true. A sticky one lasts while the session waits for the answer. A plain
/// one lasts until the session works again: then the user answered it, in
/// kari or in the terminal.
pub fn still_true(sticky: bool, state: DerivedState) -> bool {
    if sticky {
        matches!(
            state,
            DerivedState::NeedsApproval | DerivedState::NeedsDecision
        )
    } else {
        state != DerivedState::Working
    }
}

#[cfg(target_os = "macos")]
pub use mac::{show, withdraw_stale};

#[cfg(not(target_os = "macos"))]
pub fn show(app: &AppHandle, _hub: &Arc<dyn HubApi>, n: Notice) {
    use tauri_plugin_notification::NotificationExt;
    let _ = app
        .notification()
        .builder()
        .title(&n.title)
        .body(&n.body)
        .show();
}

#[cfg(not(target_os = "macos"))]
pub fn withdraw_stale(_app: &AppHandle, _hub: &Arc<dyn HubApi>) {}

/// Focus the herdr pane of a card, when this machine runs it. Otherwise show
/// the window with the card open. A pane on another node is on another
/// machine, so the window is the place for that card.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn open_card(app: &AppHandle, hub: &Arc<dyn HubApi>, node_id: &str, card_id: &str) {
    use tauri::Emitter;
    let has_pane = hub
        .board()
        .cards
        .iter()
        .any(|c| c.node_id == node_id && c.view.card.id == card_id && c.view.herdr.is_some());
    if node_id == kari_core::hub::LOCAL && has_pane {
        match hub.jump_in(node_id, card_id) {
            Ok(_) => return,
            Err(e) => tracing::warn!("notification click: jump in failed: {e}"),
        }
    }
    crate::show_main(app);
    let _ = app.emit(
        "open_card",
        serde_json::json!({ "node_id": node_id, "card_id": card_id }),
    );
}

#[cfg(target_os = "macos")]
mod mac {
    use super::{open_card, still_true, Notice};
    use kari_core::hubapi::HubApi;
    use mac_notification_sys::{MainButton, Notification, NotificationResponse};
    use std::collections::HashMap;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::{Arc, Mutex, OnceLock};
    use tauri::AppHandle;

    /// A notification on screen, or in the Notification Center.
    struct Shown {
        title: String,
        sticky: bool,
        /// Tells this notification from a newer one with the same title.
        gen: u64,
    }

    /// The notifications that kari can still take back, by node and card.
    fn shown() -> &'static Mutex<HashMap<(String, String), Shown>> {
        static SHOWN: OnceLock<Mutex<HashMap<(String, String), Shown>>> = OnceLock::new();
        SHOWN.get_or_init(Default::default)
    }

    /// Tell the library which app posts. It takes the name once, and the
    /// notification plugin gives the same name, so whichever runs first wins.
    fn set_application(app: &AppHandle) {
        static SET: OnceLock<()> = OnceLock::new();
        SET.get_or_init(|| {
            // A dev build runs outside its bundle, and macOS drops a
            // notification from an app it does not know. The plugin uses
            // Terminal for the same reason.
            let id = if tauri::is_dev() {
                "com.apple.Terminal".to_string()
            } else {
                app.config().identifier.clone()
            };
            let _ = mac_notification_sys::set_application(&id);
        });
    }

    pub fn show(app: &AppHandle, hub: &Arc<dyn HubApi>, n: Notice) {
        set_application(app);
        let Some(card_id) = n.card_id.clone() else {
            // Nothing to open and nothing to take back: post and forget.
            std::thread::spawn(move || {
                let _ = mac_notification_sys::send_notification(&n.title, None, &n.body, None);
            });
            return;
        };
        static GEN: AtomicU64 = AtomicU64::new(0);
        let gen = GEN.fetch_add(1, Ordering::Relaxed);
        let key = (n.node_id.clone(), card_id.clone());
        let old = shown().lock().unwrap().insert(
            key.clone(),
            Shown {
                title: n.title.clone(),
                sticky: n.sticky,
                gen,
            },
        );
        let (app, hub) = (app.clone(), Arc::clone(hub));
        std::thread::spawn(move || {
            // Take the old one back first. It can carry the same title, and a
            // removal after the post would take the new one too.
            if let Some(old) = old {
                remove_delivered(&app, vec![old.title]);
            }
            let mut opts = Notification::new();
            if n.sticky {
                opts.main_button(MainButton::SingleAction("Open"))
                    .close_button("Dismiss")
                    .default_sound();
            }
            opts.wait_for_click(true);
            let answer =
                mac_notification_sys::send_notification(&n.title, None, &n.body, Some(&opts));
            {
                // Forget this one, unless a newer notice for the card took its place.
                let mut map = shown().lock().unwrap();
                if map.get(&key).is_some_and(|s| s.gen == gen) {
                    map.remove(&key);
                }
            }
            match answer {
                Ok(NotificationResponse::Click) | Ok(NotificationResponse::ActionButton(_)) => {
                    open_card(&app, &hub, &n.node_id, &card_id)
                }
                Ok(_) => {}
                Err(e) => tracing::warn!("notification not shown: {e}"),
            }
        });
    }

    /// Take back each notification whose card moved on, or left the board.
    pub fn withdraw_stale(app: &AppHandle, hub: &Arc<dyn HubApi>) {
        if shown().lock().unwrap().is_empty() {
            return;
        }
        let board = hub.board();
        let stale: Vec<String> = {
            let mut map = shown().lock().unwrap();
            let gone: Vec<(String, String)> = map
                .iter()
                .filter(|((node, card), s)| {
                    !board.cards.iter().any(|c| {
                        &c.node_id == node
                            && &c.view.card.id == card
                            && still_true(s.sticky, c.view.state)
                    })
                })
                .map(|(k, _)| k.clone())
                .collect();
            gone.iter()
                .filter_map(|k| map.remove(k).map(|s| s.title))
                .collect()
        };
        if !stale.is_empty() {
            remove_delivered(app, stale);
        }
    }

    /// Remove the delivered notifications with these titles. The library names
    /// its notifications with ids of its own and does not return them, so the
    /// title is the handle. The title names the card, and a card holds one
    /// notification at a time. The waiting thread sees its notification go and
    /// ends.
    ///
    /// Call it off the main thread: it waits up to a second for the main
    /// thread to finish the removal.
    fn remove_delivered(app: &AppHandle, titles: Vec<String>) {
        let (done, wait) = std::sync::mpsc::channel();
        let posted = app.run_on_main_thread(move || {
            #[allow(deprecated)]
            {
                use objc2_foundation::NSUserNotificationCenter;
                let center = NSUserNotificationCenter::defaultUserNotificationCenter();
                let delivered = center.deliveredNotifications();
                for i in 0..delivered.count() {
                    let n = delivered.objectAtIndex(i);
                    let title = n.title().map(|t| t.to_string()).unwrap_or_default();
                    if titles.contains(&title) {
                        center.removeDeliveredNotification(&n);
                    }
                }
            }
            let _ = done.send(());
        });
        if posted.is_ok() {
            let _ = wait.recv_timeout(std::time::Duration::from_secs(1));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sticky_notice_lasts_while_the_session_waits() {
        assert!(still_true(true, DerivedState::NeedsApproval));
        assert!(still_true(true, DerivedState::NeedsDecision));
        assert!(!still_true(true, DerivedState::Working));
        assert!(!still_true(true, DerivedState::MyTurn));
    }

    #[test]
    fn a_plain_notice_lasts_until_the_session_works_again() {
        assert!(still_true(false, DerivedState::MyTurn));
        assert!(still_true(false, DerivedState::Validate));
        assert!(!still_true(false, DerivedState::Working));
    }
}
