//! The file watcher's two guards: which events count as a change, and how
//! often `claude agents` may run.
//!
//! Both exist because of one failure. The inotify backend of `notify` reports
//! `IN_OPEN`, so a *read* of a watched file is an event. `claude agents` reads
//! every `~/.claude/jobs/<id>/state.json`, and so does the engine when it
//! enriches the list. With a debouncer that forwards every event kind, each
//! run of `claude agents` woke the watcher, which ran `claude agents` again:
//! two to three Node processes per second, for as long as the daemon lived.
//! macOS never showed it, because FSEvents does not report opens.
//!
//! So the watcher drops access events before they reach the engine, and the
//! engine runs `claude agents` through a gate: one run at a time, and a pause
//! between runs. The filter removes the cause. The gate caps the damage if a
//! future reader wakes the watcher some other way.

use notify::{Event, EventKind};
use std::path::PathBuf;
use std::sync::mpsc::{Receiver, RecvTimeoutError};
use std::time::{Duration, Instant};
use tracing::warn;

/// True when an event can mean that a watched file changed. A read is not a
/// change: the engine reads these files itself, and must not wake itself up.
pub fn is_change(kind: &EventKind) -> bool {
    !matches!(kind, EventKind::Access(_))
}

/// Wait for the first change event, then take every change that arrives
/// within `window` of it, and return the paths, each once. `None` when the
/// watcher is gone. A read-only event never ends the wait.
pub fn coalesce(rx: &Receiver<notify::Result<Event>>, window: Duration) -> Option<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = Vec::new();
    let first_at = loop {
        match rx.recv() {
            Ok(Ok(ev)) if is_change(&ev.kind) => {
                paths.extend(ev.paths);
                break Instant::now();
            }
            Ok(Ok(_)) => continue,
            Ok(Err(e)) => {
                warn!("watch error: {e}");
                continue;
            }
            Err(_) => return None,
        }
    };
    loop {
        let left = window.saturating_sub(first_at.elapsed());
        if left.is_zero() {
            break;
        }
        match rx.recv_timeout(left) {
            Ok(Ok(ev)) => {
                if is_change(&ev.kind) {
                    paths.extend(ev.paths);
                }
            }
            Ok(Err(e)) => warn!("watch error: {e}"),
            // Deliver what arrived. A closed channel ends the next call.
            Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => break,
        }
    }
    paths.sort();
    paths.dedup();
    Some(paths)
}

/// Spaces out the runs of one expensive scan. The caller holds the gate in a
/// mutex, so runs never overlap; this part decides whether a caller runs at
/// all, and how long it waits first.
#[derive(Default)]
pub struct Gate {
    last_done: Option<Instant>,
}

impl Gate {
    /// For a caller that arrived at `arrived`: `None` when a run finished
    /// after the arrival, because that run saw everything the caller knows
    /// about. Otherwise the pause before the caller may run, so that two
    /// runs are at least `gap` apart. A change is never lost: a caller that
    /// waits runs after its own change, and a caller that skips was covered
    /// by a run that started after its change.
    pub fn wait_for(&self, arrived: Instant, now: Instant, gap: Duration) -> Option<Duration> {
        match self.last_done {
            Some(done) if done > arrived => None,
            Some(done) => Some(gap.saturating_sub(now.saturating_duration_since(done))),
            None => Some(Duration::ZERO),
        }
    }

    pub fn done(&mut self, at: Instant) {
        self.last_done = Some(at);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use notify::event::{AccessKind, ModifyKind};
    use std::path::Path;
    use std::sync::mpsc::channel;

    fn ev(kind: EventKind, path: &str) -> notify::Result<Event> {
        Ok(Event::new(kind).add_path(Path::new(path).to_path_buf()))
    }

    #[test]
    fn a_read_is_not_a_change() {
        assert!(!is_change(&EventKind::Access(AccessKind::Open(
            notify::event::AccessMode::Read
        ))));
        assert!(!is_change(&EventKind::Access(AccessKind::Close(
            notify::event::AccessMode::Read
        ))));
        assert!(is_change(&EventKind::Modify(ModifyKind::Any)));
        assert!(is_change(&EventKind::Create(
            notify::event::CreateKind::Any
        )));
        assert!(is_change(&EventKind::Any));
    }

    #[test]
    fn reads_alone_never_wake_the_engine() {
        let (tx, rx) = channel();
        tx.send(ev(
            EventKind::Access(AccessKind::Open(notify::event::AccessMode::Read)),
            "/j/state.json",
        ))
        .unwrap();
        drop(tx);
        assert_eq!(coalesce(&rx, Duration::from_millis(50)), None);
    }

    #[test]
    fn changes_in_one_window_arrive_as_one_batch() {
        let (tx, rx) = channel();
        tx.send(ev(
            EventKind::Access(AccessKind::Open(notify::event::AccessMode::Read)),
            "/j/a/state.json",
        ))
        .unwrap();
        tx.send(ev(EventKind::Modify(ModifyKind::Any), "/j/b/state.json"))
            .unwrap();
        tx.send(ev(EventKind::Modify(ModifyKind::Any), "/j/a/state.json"))
            .unwrap();
        tx.send(ev(EventKind::Modify(ModifyKind::Any), "/j/b/state.json"))
            .unwrap();
        drop(tx);
        let got = coalesce(&rx, Duration::from_millis(50)).unwrap();
        assert_eq!(
            got,
            vec![
                PathBuf::from("/j/a/state.json"),
                PathBuf::from("/j/b/state.json")
            ]
        );
        assert_eq!(coalesce(&rx, Duration::from_millis(50)), None);
    }

    #[test]
    fn the_first_run_starts_at_once() {
        let g = Gate::default();
        let now = Instant::now();
        assert_eq!(
            g.wait_for(now, now, Duration::from_secs(3)),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn a_run_that_finished_after_the_arrival_covers_the_caller() {
        let mut g = Gate::default();
        let arrived = Instant::now();
        g.done(arrived + Duration::from_millis(10));
        assert_eq!(
            g.wait_for(
                arrived,
                arrived + Duration::from_millis(20),
                Duration::from_secs(3)
            ),
            None
        );
    }

    #[test]
    fn a_caller_after_a_run_waits_out_the_gap() {
        let mut g = Gate::default();
        let t0 = Instant::now();
        g.done(t0);
        let arrived = t0 + Duration::from_secs(1);
        assert_eq!(
            g.wait_for(arrived, arrived, Duration::from_secs(3)),
            Some(Duration::from_secs(2))
        );
        let late = t0 + Duration::from_secs(10);
        assert_eq!(
            g.wait_for(late, late, Duration::from_secs(3)),
            Some(Duration::ZERO)
        );
    }
}
