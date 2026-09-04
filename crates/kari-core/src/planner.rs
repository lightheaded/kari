//! The quota-aware planner: when to offer a run, and which tasks fit.
//!
//! Percentages come from the 5-hour window, because that window limits what can
//! run right now. The 7-day window only decides whether an offer is worth making.

use crate::model::*;
use chrono::{DateTime, Duration, Local, TimeZone, Utc};

/// What the planner needs to know about the machine right now.
pub struct Context<'a> {
    pub now: DateTime<Utc>,
    pub quota: Option<&'a QuotaSample>,
    /// Background jobs kari started that still run.
    pub running_jobs: u32,
    /// Newest activity of any interactive session.
    pub last_interactive_at: Option<DateTime<Utc>>,
    /// True when a session is busy this second.
    pub any_busy: bool,
}

/// One card that may run unattended.
pub struct Candidate {
    pub card_id: String,
    pub title: String,
    pub project_name: Option<String>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub priority: i32,
    pub created_at: DateTime<Utc>,
    pub estimate: Estimate,
}

fn in_working_hours(now: DateTime<Utc>, s: &Settings) -> bool {
    let local = Local.from_utc_datetime(&now.naive_utc());
    let h = chrono::Timelike::hour(&local);
    if s.working_hours_start <= s.working_hours_end {
        h >= s.working_hours_start && h < s.working_hours_end
    } else {
        // A window that crosses midnight, for example 20 to 8.
        h >= s.working_hours_start || h < s.working_hours_end
    }
}

/// Percent of the 5-hour window the planner may spend now.
pub fn budget_pct(ctx: &Context<'_>, s: &Settings) -> f64 {
    let Some(q) = ctx.quota else { return 0.0 };
    let Some(five) = q.five_hour.as_ref() else {
        return 0.0;
    };
    let mut budget = s.fill_ceiling_pct - five.used_percentage;
    if in_working_hours(ctx.now, s) {
        budget -= s.working_hours_reserve_pct;
    }
    // A weekly window near its ceiling stops every plan.
    if let Some(seven) = q.seven_day.as_ref() {
        let weekly_left = s.fill_ceiling_pct - seven.used_percentage;
        if weekly_left <= 0.0 {
            return 0.0;
        }
    }
    budget.max(0.0)
}

/// Decide whether the state of the two windows deserves an offer.
pub fn detect_trigger(ctx: &Context<'_>, s: &Settings) -> Option<(ProposalTrigger, String)> {
    let q = ctx.quota?;
    if let Some(seven) = q.seven_day.as_ref() {
        let unused = 100.0 - seven.used_percentage;
        if let Some(reset) = seven.resets_at {
            let left = reset - ctx.now;
            if unused > s.weekly_unused_pct
                && left > Duration::zero()
                && left <= Duration::hours(s.weekly_hours_before_reset)
            {
                return Some((
                    ProposalTrigger::WeeklyReset,
                    format!(
                        "{:.0} percent of the weekly window is unused and it resets in {} hours",
                        unused,
                        left.num_hours().max(1)
                    ),
                ));
            }
        }
    }
    if let Some(five) = q.five_hour.as_ref() {
        let idle_long_enough = ctx
            .last_interactive_at
            .is_none_or(|t| ctx.now - t > Duration::minutes(s.idle_minutes));
        if five.used_percentage < s.five_hour_idle_pct && idle_long_enough && !ctx.any_busy {
            return Some((
                ProposalTrigger::IdleFiveHour,
                format!(
                    "the 5-hour window is at {:.0} percent and nobody worked for {} minutes",
                    five.used_percentage, s.idle_minutes
                ),
            ));
        }
    }
    None
}

/// Pack candidates into the budget. Highest priority first, oldest first inside
/// a priority. A candidate that does not fit is skipped, not dropped from the board.
pub fn plan(
    trigger: ProposalTrigger,
    reason: String,
    mut candidates: Vec<Candidate>,
    ctx: &Context<'_>,
    s: &Settings,
) -> Option<Proposal> {
    let budget = budget_pct(ctx, s);
    if budget <= 1.0 {
        return None;
    }
    let slots = s.max_parallel_bg.saturating_sub(ctx.running_jobs);
    if slots == 0 {
        return None;
    }
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.created_at.cmp(&b.created_at))
    });
    let mut items: Vec<ProposalItem> = vec![];
    let mut total = 0.0f64;
    let mut skipped = 0u32;
    for c in candidates {
        if items.len() as u32 >= slots {
            skipped += 1;
            continue;
        }
        // The high end of the band must fit, so an overrun does not break the ceiling.
        if total + c.estimate.pct_five_hour > budget {
            skipped += 1;
            continue;
        }
        total += c.estimate.pct_five_hour;
        items.push(ProposalItem {
            card_id: c.card_id,
            title: c.title,
            project_name: c.project_name,
            prompt: c.prompt,
            model: c.model,
            estimate: c.estimate,
            job_id: None,
            error: None,
        });
    }
    if items.is_empty() {
        return None;
    }
    let used_before = ctx
        .quota
        .and_then(|q| q.five_hour.as_ref())
        .map(|w| w.used_percentage)
        .unwrap_or(0.0);
    Some(Proposal {
        id: uuid::Uuid::new_v4().to_string(),
        created_at: ctx.now,
        trigger,
        reason,
        items,
        budget_pct: budget,
        used_pct_before: used_before,
        total_pct: total,
        used_pct_after: used_before + total,
        skipped,
        expires_at: ctx.now + Duration::hours(2),
        state: "open".into(),
        auto: false,
        accepted_at: None,
    })
}

/// When a trigger fires next, as far as the reset times allow a guess.
///
/// Two triggers exist. The weekly one fires a set number of hours before the
/// 7-day window resets. The idle one fires once nobody has worked for long
/// enough, and only while the 5-hour window sits below its threshold. The
/// earlier of the two wins. A trigger that is live already returns `now`.
pub fn next_trigger_at(
    ctx: &Context<'_>,
    s: &Settings,
) -> Option<(ProposalTrigger, DateTime<Utc>)> {
    if let Some((t, _)) = detect_trigger(ctx, s) {
        return Some((t, ctx.now));
    }
    let q = ctx.quota?;
    let mut best: Option<(ProposalTrigger, DateTime<Utc>)> = None;
    let mut offer = |t: ProposalTrigger, at: DateTime<Utc>| {
        if at < ctx.now {
            return;
        }
        if best.as_ref().is_none_or(|(_, b)| at < *b) {
            best = Some((t, at));
        }
    };
    if let Some(seven) = q.seven_day.as_ref() {
        let unused = 100.0 - seven.used_percentage;
        if let Some(reset) = seven.resets_at {
            if unused > s.weekly_unused_pct {
                offer(
                    ProposalTrigger::WeeklyReset,
                    reset - Duration::hours(s.weekly_hours_before_reset),
                );
            }
        }
    }
    if let Some(five) = q.five_hour.as_ref() {
        // Below the threshold the wait is only the idle timer. Above it, the
        // window must reset first, and the timer runs from there.
        if five.used_percentage < s.five_hour_idle_pct {
            let from = ctx.last_interactive_at.unwrap_or(ctx.now);
            offer(
                ProposalTrigger::IdleFiveHour,
                from + Duration::minutes(s.idle_minutes),
            );
        } else if let Some(reset) = five.resets_at {
            offer(
                ProposalTrigger::IdleFiveHour,
                reset + Duration::minutes(s.idle_minutes),
            );
        }
    }
    best
}

/// Every candidate in the order the planner would take them, whether it fits
/// the budget or not. This is what the queue strip shows. It starts nothing.
pub fn queue(
    mut candidates: Vec<Candidate>,
    ctx: &Context<'_>,
    s: &Settings,
    mode: AutomationMode,
    open_proposal: bool,
    next_check_at: DateTime<Utc>,
) -> QueuePlan {
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.created_at.cmp(&b.created_at))
    });
    let budget = budget_pct(ctx, s);
    let used = ctx
        .quota
        .and_then(|q| q.five_hour.as_ref())
        .map(|w| w.used_percentage)
        .unwrap_or(0.0);
    let slots = s.max_parallel_bg.saturating_sub(ctx.running_jobs);
    let next = next_trigger_at(ctx, s);

    let blocked = if mode == AutomationMode::Off {
        Some("automation is off".into())
    } else if ctx.quota.is_none() {
        Some("no quota sample yet".into())
    } else if slots == 0 {
        Some(format!(
            "every one of the {} job slots is busy",
            s.max_parallel_bg
        ))
    } else if budget <= 1.0 {
        Some(format!(
            "the budget is {budget:.0} percent of the 5-hour window"
        ))
    } else if candidates.is_empty() {
        Some("no card is marked may run unattended".into())
    } else {
        None
    };

    let mut steps = vec![];
    let mut total = 0.0f64;
    for (i, c) in candidates.into_iter().enumerate() {
        let cost = c.estimate.pct_five_hour;
        let over_slots = i as u32 >= slots;
        let fits = !over_slots && total + cost <= budget && blocked.is_none();
        if fits {
            total += cost;
        }
        let (starts_at, reason) = if fits {
            if open_proposal {
                // The plan panel holds the buttons. The strip says so once, in
                // its header, so every step keeps a short answer here.
                (Some(ctx.now), "now".into())
            } else {
                match next {
                    Some((_, at)) if at <= ctx.now => (Some(ctx.now), "now".into()),
                    Some((_, at)) => (Some(at), "at the next trigger".into()),
                    None => (None, "no trigger in sight".into()),
                }
            }
        } else if let Some(b) = &blocked {
            (None, b.clone())
        } else if over_slots {
            (None, "no free job slot".into())
        } else {
            (None, "does not fit the budget".into())
        };
        steps.push(QueueStep {
            card_id: c.card_id,
            title: c.title,
            project_name: c.project_name,
            model: c.model,
            window_after_pct: used + total,
            estimate: c.estimate,
            fits,
            starts_at,
            reason,
        });
    }

    QueuePlan {
        steps,
        budget_pct: budget,
        used_pct: used,
        next_check_at,
        next_trigger_at: next.map(|(_, at)| at),
        next_trigger: next.map(|(t, _)| t),
        mode,
        blocked,
        open_proposal,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn settings() -> Settings {
        // Keep the working-hours reserve out of the arithmetic in these tests.
        Settings {
            working_hours_reserve_pct: 0.0,
            working_hours_start: 0,
            working_hours_end: 0,
            ..Default::default()
        }
    }

    fn quota(five: f64, seven: f64, reset_in_hours: i64) -> QuotaSample {
        QuotaSample {
            at: Utc::now(),
            five_hour: Some(QuotaWindow {
                used_percentage: five,
                resets_at: Some(Utc::now() + Duration::hours(3)),
            }),
            seven_day: Some(QuotaWindow {
                used_percentage: seven,
                resets_at: Some(Utc::now() + Duration::hours(reset_in_hours)),
            }),
            source: "test".into(),
        }
    }

    fn candidate(name: &str, pct: f64, priority: i32) -> Candidate {
        Candidate {
            card_id: name.into(),
            title: name.into(),
            project_name: None,
            prompt: Some("do the thing".into()),
            model: None,
            priority,
            created_at: Utc::now(),
            estimate: Estimate {
                weighted_tokens: pct * 1e6,
                low: pct * 0.5e6,
                high: pct * 2e6,
                pct_five_hour: pct,
                pct_low: pct * 0.5,
                pct_high: pct * 2.0,
                source: "test".into(),
                sessions: 3,
            },
        }
    }

    fn ctx<'a>(q: &'a QuotaSample, running: u32) -> Context<'a> {
        Context {
            now: Utc::now(),
            quota: Some(q),
            running_jobs: running,
            last_interactive_at: None,
            any_busy: false,
        }
    }

    #[test]
    fn weekly_reset_fires_when_much_is_unused() {
        let q = quota(10.0, 20.0, 20);
        let (t, why) = detect_trigger(&ctx(&q, 0), &settings()).unwrap();
        assert_eq!(t, ProposalTrigger::WeeklyReset);
        assert!(why.contains("weekly"));
    }

    #[test]
    fn idle_window_fires_when_the_weekly_reset_is_far_away() {
        let q = quota(10.0, 20.0, 100);
        let (t, _) = detect_trigger(&ctx(&q, 0), &settings()).unwrap();
        assert_eq!(t, ProposalTrigger::IdleFiveHour);
    }

    #[test]
    fn nothing_fires_when_the_windows_are_busy() {
        let q = quota(70.0, 90.0, 100);
        assert!(detect_trigger(&ctx(&q, 0), &settings()).is_none());
    }

    #[test]
    fn packs_by_priority_and_respects_the_ceiling() {
        let q = quota(80.0, 10.0, 100); // 5 percent left below the 85 ceiling
        let s = settings();
        let p = plan(
            ProposalTrigger::Manual,
            "manual".into(),
            vec![
                candidate("big", 4.0, 0),
                candidate("urgent", 3.0, 5),
                candidate("small", 1.0, 0),
            ],
            &ctx(&q, 0),
            &s,
        )
        .unwrap();
        assert_eq!(p.items.len(), 2);
        assert_eq!(p.items[0].card_id, "urgent");
        assert_eq!(p.items[1].card_id, "small");
        assert_eq!(p.skipped, 1);
        assert!((p.used_pct_after - 84.0).abs() < 0.001);
    }

    #[test]
    fn a_full_window_plans_nothing() {
        let q = quota(90.0, 10.0, 100);
        assert!(plan(
            ProposalTrigger::Manual,
            "m".into(),
            vec![candidate("a", 1.0, 0)],
            &ctx(&q, 0),
            &settings()
        )
        .is_none());
    }

    #[test]
    fn the_parallel_cap_holds() {
        let q = quota(0.0, 0.0, 100);
        let mut s = settings();
        s.max_parallel_bg = 2;
        let p = plan(
            ProposalTrigger::Manual,
            "m".into(),
            vec![
                candidate("a", 1.0, 0),
                candidate("b", 1.0, 0),
                candidate("c", 1.0, 0),
            ],
            &ctx(&q, 1),
            &s,
        )
        .unwrap();
        assert_eq!(p.items.len(), 1);
    }

    #[test]
    fn the_queue_marks_what_fits_and_what_does_not() {
        // 30 percent used, a ceiling of 85, so the budget is 55 percent.
        let q = quota(30.0, 20.0, 200);
        let s = Settings {
            max_parallel_bg: 4,
            ..settings()
        };
        let cands = vec![
            candidate("a", 20.0, 5),
            candidate("b", 20.0, 3),
            candidate("c", 40.0, 1),
        ];
        let plan = queue(
            cands,
            &ctx(&q, 0),
            &s,
            AutomationMode::Ask,
            false,
            Utc::now(),
        );
        assert!(plan.blocked.is_none());
        let fits: Vec<bool> = plan.steps.iter().map(|x| x.fits).collect();
        assert_eq!(fits, vec![true, true, false]);
        // Priority decides the order, so "a" comes first.
        assert_eq!(plan.steps[0].card_id, "a");
        // The window shows 30 + 20 and then 30 + 20 + 20.
        assert!((plan.steps[0].window_after_pct - 50.0).abs() < 0.01);
        assert!((plan.steps[1].window_after_pct - 70.0).abs() < 0.01);
        assert_eq!(plan.steps[2].reason, "does not fit the budget");
    }

    #[test]
    fn a_mode_of_off_blocks_the_whole_queue() {
        let q = quota(10.0, 20.0, 200);
        let plan = queue(
            vec![candidate("a", 5.0, 0)],
            &ctx(&q, 0),
            &settings(),
            AutomationMode::Off,
            false,
            Utc::now(),
        );
        assert_eq!(plan.blocked.as_deref(), Some("automation is off"));
        assert!(!plan.steps[0].fits);
        assert!(plan.steps[0].starts_at.is_none());
    }

    #[test]
    fn a_busy_job_slot_leaves_no_room() {
        let q = quota(10.0, 20.0, 200);
        let s = Settings {
            max_parallel_bg: 1,
            ..settings()
        };
        let plan = queue(
            vec![candidate("a", 5.0, 0)],
            &ctx(&q, 1),
            &s,
            AutomationMode::Ask,
            false,
            Utc::now(),
        );
        assert!(plan
            .blocked
            .as_deref()
            .is_some_and(|b| b.contains("job slots")));
    }

    #[test]
    fn a_live_trigger_starts_the_queue_now() {
        // 10 percent used with 80 percent of the week unused and a reset in 20
        // hours fires the weekly trigger at once.
        let q = quota(10.0, 20.0, 20);
        let plan = queue(
            vec![candidate("a", 5.0, 0)],
            &ctx(&q, 0),
            &settings(),
            AutomationMode::Ask,
            false,
            Utc::now(),
        );
        assert_eq!(plan.next_trigger, Some(ProposalTrigger::WeeklyReset));
        assert_eq!(plan.steps[0].reason, "now");
    }

    #[test]
    fn the_next_weekly_trigger_is_hours_before_the_reset() {
        // The weekly window resets in 200 hours and the trigger fires 36 hours
        // before that, so it is 164 hours away. The 5-hour window must not
        // offer anything earlier, so it carries no reset time here.
        let mut q = quota(90.0, 20.0, 200);
        q.five_hour.as_mut().unwrap().resets_at = None;
        let s = Settings {
            five_hour_idle_pct: 0.0,
            ..settings()
        };
        let (t, at) = next_trigger_at(&ctx(&q, 0), &s).unwrap();
        assert_eq!(t, ProposalTrigger::WeeklyReset);
        let hours = (at - Utc::now()).num_hours();
        assert!((163..=165).contains(&hours), "{hours} hours");
    }

    #[test]
    fn a_full_five_hour_window_waits_for_its_reset_and_the_idle_timer() {
        // 90 percent used, resets in 3 hours, then 45 idle minutes on top.
        let q = quota(90.0, 20.0, 200);
        let s = Settings {
            five_hour_idle_pct: 30.0,
            idle_minutes: 45,
            ..settings()
        };
        let (t, at) = next_trigger_at(&ctx(&q, 0), &s).unwrap();
        assert_eq!(t, ProposalTrigger::IdleFiveHour);
        let mins = (at - Utc::now()).num_minutes();
        assert!((224..=226).contains(&mins), "{mins} minutes");
    }
}
