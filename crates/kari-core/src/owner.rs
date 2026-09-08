//! Which process runs the engine on this machine.
//!
//! A host needs a daemon, because the state of its sessions must accrue while
//! nobody has a window open — see rule 4 in `AGENTS.md`. A host with a window
//! open also has the app, and the app has an engine of its own. Both cannot
//! run: one engine per machine is not a preference but a requirement, and three
//! separate things break when two of them run.
//!
//! - They bind the same port. The hook relay is registered in
//!   `~/.claude/settings.json` as one address, so the second engine to start
//!   gets nothing and the hooks reach whichever won.
//! - They have the same node id, which lives in the store they share. A server
//!   sees one node dialling twice and drops each link as the other arrives, so
//!   the machine flickers on every other client's board.
//! - They both plan and both start jobs, against one Claude Code login. Two
//!   planners that each believe they own the window overrun it together.
//!
//! So ownership is a file, and the rule is simple: the window wins, and the
//! daemon waits. The app takes the file when it starts. The daemon takes it
//! only while nothing else holds it, and steps down within seconds of losing
//! it — it exits, and whatever supervises it starts it again, so it comes back
//! waiting rather than running.
//!
//! The file holds a pid, and a pid is checked for life before it is believed.
//! That is what makes a crash safe: a process that dies without clearing its
//! claim leaves a pid that answers nothing, and the next reader takes over.

use crate::paths;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::time::{Duration, Instant};
use tracing::{info, warn};

/// How often a daemon asks whether it still owns the engine.
const CHECK_EVERY: Duration = Duration::from_secs(3);

/// What the app calls itself in the claim.
pub const APP: &str = "kari";
/// What the daemon calls itself in the claim.
pub const DAEMON: &str = "kari-node";

/// What the owner file says.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Holder {
    pub pid: u32,
    /// `kari` for the app, `kari-node` for the daemon. For a log line and for
    /// the message the app shows while it waits.
    pub what: String,
    /// The port that owner serves the API on, so a reader can wait for it to
    /// close before binding it.
    #[serde(default)]
    pub port: u16,
    pub at: chrono::DateTime<chrono::Utc>,
}

fn file() -> PathBuf {
    paths::kari_dir().join("engine-owner.json")
}

/// The live owner of the engine on this machine, if it is not this process.
///
/// A claim by a pid that is no longer alive is no claim: the answer is None,
/// and the caller takes the file.
pub fn current() -> Option<Holder> {
    let h: Holder = serde_json::from_str(&std::fs::read_to_string(file()).ok()?).ok()?;
    if h.pid == std::process::id() {
        return None;
    }
    if !crate::registry::pid_alive(h.pid) {
        return None;
    }
    Some(h)
}

/// The engine of this process, and the claim that says so. Dropping it clears
/// the claim, so the next process does not have to wait for a pid check.
pub struct Owned {
    what: String,
}

impl Drop for Owned {
    fn drop(&mut self) {
        // Only if it is still ours: another process may have taken over.
        if let Some(h) = read() {
            if h.pid == std::process::id() {
                let _ = std::fs::remove_file(file());
                info!("{} released the engine of this machine", self.what);
            }
        }
    }
}

fn read() -> Option<Holder> {
    serde_json::from_str(&std::fs::read_to_string(file()).ok()?).ok()
}

/// Claim the engine for this process, whatever the file says now.
pub fn take(what: &str, port: u16) -> anyhow::Result<Owned> {
    let dir = paths::kari_dir();
    std::fs::create_dir_all(&dir)?;
    let h = Holder {
        pid: std::process::id(),
        what: what.to_string(),
        port,
        at: chrono::Utc::now(),
    };
    std::fs::write(file(), serde_json::to_string_pretty(&h)?)?;
    Ok(Owned {
        what: what.to_string(),
    })
}

/// Claim the engine for a window.
///
/// A daemon's claim is taken at once, and this is the part that has to be got
/// right: a daemon steps down when it sees the claim change, so it is waiting
/// for exactly this write. Waiting for it to let go first is a deadlock, and
/// the first version of this function had one — it sat for its whole timeout
/// and then took the claim anyway, which worked only because it gave up.
///
/// A claim by another window is waited for, because two windows are a mistake
/// rather than a design, and the first one is the one someone is reading. After
/// `timeout` it is taken anyway: a window that refused to open because another
/// process would not let go is worse than two engines, which is what every
/// version before this claim existed already had.
pub fn take_when_free(what: &str, port: u16, timeout: Duration) -> anyhow::Result<Owned> {
    let deadline = Instant::now() + timeout;
    let mut said = false;
    while let Some(h) = current() {
        if h.what == DAEMON {
            info!(
                "a daemon holds the engine of this machine (pid {}); taking it",
                h.pid
            );
            break;
        }
        if !said {
            info!(
                "{} holds the engine of this machine (pid {}); waiting for it to close",
                h.what, h.pid
            );
            said = true;
        }
        if Instant::now() > deadline {
            warn!(
                "{} still holds the engine after {:?}; taking it anyway",
                h.what, timeout
            );
            break;
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    take(what, port)
}

/// Wait for the engine to be free, however long that takes, then claim it.
///
/// The daemon calls this. It never takes by force: the window is the one the
/// person is looking at, and a daemon that shoved it aside would take the board
/// away from them to serve a board nobody is reading.
pub fn take_after_wait(what: &str, port: u16) -> anyhow::Result<Owned> {
    let mut said = false;
    while let Some(h) = current() {
        if !said {
            info!(
                "{} holds the engine of this machine (pid {}); waiting",
                h.what, h.pid
            );
            said = true;
        }
        std::thread::sleep(CHECK_EVERY);
    }
    take(what, port)
}

/// Watch the claim, and call `lost` when another process takes it.
///
/// The daemon calls this and exits. It does not try to close its engine and
/// carry on: the engine holds watchers, a port, a link to a server and a
/// planner, and unwinding all of that in place is more machinery than a
/// restart. Whatever supervises the daemon starts it again, and it comes back
/// into `take_after_wait`.
pub fn step_down_when_taken(what: &str, port: u16, lost: impl Fn(Holder) + Send + 'static) {
    let me = std::process::id();
    let what = what.to_string();
    std::thread::Builder::new()
        .name("kari-owner-watch".into())
        .spawn(move || {
            loop {
                std::thread::sleep(CHECK_EVERY);
                match read() {
                    // Ours, or a dead claim we can ignore.
                    Some(h) if h.pid == me => {}
                    Some(h) if !crate::registry::pid_alive(h.pid) => {}
                    // Someone else runs the engine now.
                    Some(h) => {
                        lost(h);
                        return;
                    }
                    // The file is gone. Nobody claims the engine, and this
                    // process is the one running it, so claim it again rather
                    // than leave the next reader to think it is free.
                    None => {
                        if let Err(e) = take(&what, port) {
                            warn!("the engine claim is not written: {e}");
                        }
                    }
                }
            }
        })
        .expect("spawn");
}

/// Wait until nothing listens on `port` any more.
///
/// The process that steps down has to close its listener before the next one
/// can bind it, and it does that a moment after it drops the claim.
pub fn wait_for_port(port: u16, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    loop {
        match std::net::TcpListener::bind(("127.0.0.1", port)) {
            Ok(l) => {
                drop(l);
                return true;
            }
            Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(200)),
            Err(_) => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_claim_by_a_dead_process_is_no_claim() {
        // The crash case, and the reason the claim holds a pid rather than a
        // flag: a process that dies without clearing it must not keep the
        // engine of a machine to itself for ever.
        let mut child = std::process::Command::new("cargo")
            .arg("--version")
            .stdout(std::process::Stdio::null())
            .spawn()
            .expect("spawn a process that ends at once");
        let dead = child.id();
        child.wait().unwrap();
        // Reaped, so the pid names nothing.
        assert!(
            !crate::registry::pid_alive(dead),
            "pid {dead} has ended and must read as dead"
        );

        let json = serde_json::to_string(&Holder {
            pid: dead,
            what: "kari-node".into(),
            port: 47311,
            at: chrono::Utc::now(),
        })
        .unwrap();
        let back: Holder = serde_json::from_str(&json).unwrap();
        assert!(!crate::registry::pid_alive(back.pid));
    }

    #[test]
    fn a_nonsense_pid_is_not_alive() {
        // kill(-1, 0) asks about every process and succeeds, so a pid that
        // casts to a negative number must be refused before the syscall.
        assert!(!crate::registry::pid_alive(u32::MAX));
        assert!(!crate::registry::pid_alive(0));
    }

    #[test]
    fn a_free_port_is_seen_as_free() {
        let port = crate::tunnel::free_port().unwrap();
        assert!(wait_for_port(port, Duration::from_millis(500)));
    }

    #[test]
    fn a_held_port_is_seen_as_held() {
        let l = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(!wait_for_port(port, Duration::from_millis(300)));
        drop(l);
    }
}
