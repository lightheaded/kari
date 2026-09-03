//! Derive one state per card from every signal, then place it in a column.

use crate::model::*;
use chrono::{DateTime, Duration, Utc};

pub struct Inputs<'a> {
    pub card: &'a Card,
    pub facts: Option<&'a SessionFacts>,
    pub live: Option<&'a LiveSession>,
    pub bg: Option<&'a BgJob>,
    pub herdr: Option<&'a HerdrAgent>,
    pub hooks: Option<&'a HookState>,
    /// A permission prompt kari holds for a remote answer.
    pub permission: Option<&'a PendingPermission>,
    pub summary: Option<&'a Summary>,
    pub now: DateTime<Utc>,
    pub settings: &'a Settings,
}

pub fn last_activity(
    facts: Option<&SessionFacts>,
    live: Option<&LiveSession>,
    bg: Option<&BgJob>,
) -> Option<DateTime<Utc>> {
    let mut best: Option<DateTime<Utc>> = None;
    let mut bump = |t: Option<DateTime<Utc>>| {
        if let Some(t) = t {
            if best.is_none_or(|b| t > b) {
                best = Some(t);
            }
        }
    };
    if let Some(f) = facts {
        bump(f.last_at);
        bump(f.file_mtime);
    }
    if let Some(l) = live {
        bump(l.status_updated_at);
        if l.alive {
            bump(l.started_at);
        }
    }
    if let Some(b) = bg {
        bump(b.started_at);
    }
    best
}

/// Returns the derived state and a short human reason.
pub fn derive(i: &Inputs<'_>) -> (DerivedState, String) {
    use DerivedState::*;
    let card = i.card;

    if card.done_at.is_some() {
        return (Done, "marked done".into());
    }

    // Background job signals win: they are exact.
    if let Some(bg) = i.bg {
        match bg.state.as_deref() {
            Some("working") => return (Working, "background job working".into()),
            Some("blocked") => {
                let w = bg.waiting_for.clone().unwrap_or_default();
                return if w.contains("input") {
                    (NeedsDecision, format!("background job waits: {w}"))
                } else {
                    (NeedsApproval, format!("background job waits: {w}"))
                };
            }
            Some("done") => return (Validate, "background job finished".into()),
            Some("failed") => return (MyTurn, "background job failed".into()),
            Some("stopped") => return (MyTurn, "background job stopped".into()),
            _ => {}
        }
    }

    // The job list forgets a finished job. The card remembers what happened,
    // until the session shows work that came after the job ended.
    if i.bg.is_none() {
        if let (Some(state), Some(at)) = (card.last_job_state.as_deref(), card.last_job_at) {
            let fresh = i.now - at < Duration::hours(12);
            let no_newer_work = i
                .facts
                .and_then(|f| f.last_at)
                .is_none_or(|t| t <= at + Duration::minutes(5));
            if fresh && no_newer_work {
                match state {
                    "done" => return (Validate, "background job finished".into()),
                    "failed" => return (MyTurn, "background job failed".into()),
                    "stopped" => return (MyTurn, "background job stopped".into()),
                    "blocked" => return (NeedsApproval, "background job waits for input".into()),
                    _ => {}
                }
            }
        }
    }

    // Task cards without a session.
    if card.kind == CardKind::Task && card.session_id.is_none() {
        return if card.auto_run
            && card
                .run_prompt
                .as_deref()
                .is_some_and(|p| !p.trim().is_empty())
        {
            (Ready, "auto-run enabled".into())
        } else {
            (Backlog, "task".into())
        };
    }

    let alive = i.live.is_some_and(|l| l.alive);

    // kari holds a permission prompt of this session for a remote answer.
    if let Some(p) = i.permission {
        return (
            NeedsApproval,
            format!("{} waits for permission", p.tool_name),
        );
    }

    // Pending questions and plan approvals at the transcript tail.
    if let Some(f) = i.facts {
        if let Some(t) = f.pending_tools.iter().find(|t| t.name == "AskUserQuestion") {
            let n = t.questions.len();
            return (NeedsDecision, format!("{n} open question(s)"));
        }
        if f.pending_tools.iter().any(|t| t.name == "ExitPlanMode") {
            return (NeedsApproval, "plan waits for approval".into());
        }
    }

    // A hook reported a permission prompt that no tool run or prompt cleared since.
    if let (Some(h), true) = (i.hooks, alive) {
        if let Some(since) = h.permission_pending_since {
            let after_prompt = i
                .facts
                .and_then(|f| f.last_user_at)
                .is_none_or(|t| since >= t - Duration::seconds(2));
            if after_prompt {
                let what = h
                    .permission_message
                    .as_deref()
                    .map(|m| crate::truncate(m, 80))
                    .unwrap_or_else(|| "permission prompt".into());
                return (NeedsApproval, format!("hook: {what}"));
            }
        }
    }

    // herdr saw an approval or question UI on screen.
    if let Some(h) = i.herdr {
        if h.agent_status.as_deref() == Some("blocked") && alive {
            return (NeedsApproval, "herdr: agent blocked on a prompt".into());
        }
    }

    if alive {
        let status = i.live.and_then(|l| l.status.as_deref()).unwrap_or("");
        let herdr_working = i.herdr.and_then(|h| h.agent_status.as_deref()) == Some("working");
        let hook_turn_active = i
            .hooks
            .is_some_and(|h| h.turn_active && h.idle_since.is_none());
        return match status {
            "busy" => (Working, "session busy".into()),
            _ if herdr_working => (Working, "herdr: agent working".into()),
            _ if hook_turn_active && status.is_empty() => {
                (Working, "hook: turn in progress".into())
            }
            "shell" => (MyTurn, "in shell mode".into()),
            _ => judged_or(i, MyTurn, "idle, between prompts"),
        };
    }

    // No live process. Age decides.
    let last = last_activity(i.facts, i.live, i.bg);
    let age = last.map(|t| i.now - t);
    match age {
        Some(a) if a > Duration::days(i.settings.stale_after_days) => {
            (Stale, format!("no activity for {} days", a.num_days()))
        }
        Some(a) if a > Duration::days(i.settings.done_after_days) => (
            Done,
            format!("exited, no activity for {} days", a.num_days()),
        ),
        Some(_) => judged_or(i, MyTurn, "exited, resume to continue"),
        None => (Unknown, "no signals".into()),
    }
}

/// Soft signal: a fresh, confident summary can move a quiet session to
/// `waiting_on_others` or `validate`. It never overrides a hard signal.
fn judged_or(i: &Inputs<'_>, fallback: DerivedState, reason: &str) -> (DerivedState, String) {
    use DerivedState::*;
    if let Some(s) = i.summary {
        let fresh = match (s.based_on_at, i.facts.and_then(|f| f.last_at)) {
            (Some(b), Some(l)) => b >= l - Duration::seconds(5),
            (None, _) => false,
            (Some(_), None) => true,
        };
        if fresh && s.confidence >= 0.6 {
            match s.judged_state {
                WaitingOnOthers => {
                    return (
                        WaitingOnOthers,
                        format!(
                            "summary: {}",
                            s.next_step.as_deref().unwrap_or("waits on someone else")
                        ),
                    )
                }
                Validate => {
                    return (
                        Validate,
                        "summary: work looks complete, not verified".into(),
                    )
                }
                Done if s.confidence >= 0.8 => {
                    return (Validate, "summary: judged done, confirm and close".into())
                }
                _ => {}
            }
        }
    }
    (fallback, reason.into())
}

/// Pick the column for a state, honoring a manual lock when the signal is weaker.
/// Returns (column id, locked, lock_broken).
pub fn resolve_column(
    card: &Card,
    state: DerivedState,
    columns: &[Column],
) -> (Option<String>, bool, bool) {
    if let Some(manual) = &card.manual_column {
        if columns.iter().any(|c| &c.id == manual) {
            let lock_prio = card.manual_lock_priority.unwrap_or(0);
            let breaks = state.breaks_lock() || state.priority() > lock_prio;
            if !breaks {
                return (Some(manual.clone()), true, false);
            }
            let col = column_for(state, columns);
            return (col, false, true);
        }
    }
    (column_for(state, columns), false, false)
}

pub fn column_for(state: DerivedState, columns: &[Column]) -> Option<String> {
    let mut sorted: Vec<&Column> = columns.iter().collect();
    sorted.sort_by_key(|c| c.order);
    if let Some(c) = sorted.iter().find(|c| c.accepts.contains(&state)) {
        return Some(c.id.clone());
    }
    if state == DerivedState::Stale {
        return None;
    }
    sorted
        .iter()
        .find(|c| c.accepts.contains(&DerivedState::Unknown))
        .copied()
        .or_else(|| sorted.first().copied())
        .map(|c| c.id.clone())
}
