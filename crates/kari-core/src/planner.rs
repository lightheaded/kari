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

/// The length of the short rate-limit window. Claude Code reports the reset
/// time of the window in use, and every window after it is this long.
pub const FIVE_HOUR_WINDOW: Duration = Duration::hours(5);

/// A scheduled run starts this long after the reset it was booked for. The
/// reset time is a report, not a promise, so a run that starts on the second
/// can still meet the old window.
pub const RESET_MARGIN_MINUTES: i64 = 2;

/// The next reset of a window, from a sample that can be old.
///
/// A window that reset while the machine slept reports a time in the past. The
/// windows are the same length one after another, so stepping forward by one
/// window at a time gives the reset that is still to come.
fn next_reset(
    w: Option<&QuotaWindow>,
    length: Duration,
    now: DateTime<Utc>,
) -> Option<DateTime<Utc>> {
    let mut at = w?.resets_at?;
    let mut guard = 0;
    while at <= now && guard < 400 {
        at += length;
        guard += 1;
    }
    (at > now).then_some(at)
}

/// Turn the cycle a caller named into a time on this node.
///
/// The caller names a cycle instead of a time because the reset times belong
/// to the account this node is signed in to. A phone that schedules a run on
/// three nodes must not send one time to all three.
pub fn resolve_schedule(
    when: ScheduleWhen,
    quota: Option<&QuotaSample>,
    now: DateTime<Utc>,
) -> anyhow::Result<(DateTime<Utc>, String)> {
    let margin = Duration::minutes(RESET_MARGIN_MINUTES);
    let five = || -> anyhow::Result<DateTime<Utc>> {
        next_reset(
            quota.and_then(|q| q.five_hour.as_ref()),
            FIVE_HOUR_WINDOW,
            now,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "this node does not know when the 5-hour window resets. Install the status line, or pick a time."
            )
        })
    };
    Ok(match when {
        ScheduleWhen::NextReset => (five()? + margin, "when the 5-hour window resets".into()),
        ScheduleWhen::FollowingCycle => (
            five()? + FIVE_HOUR_WINDOW + margin,
            "in the 5-hour window after the next one".into(),
        ),
        ScheduleWhen::WeeklyReset => (
            next_reset(
                quota.and_then(|q| q.seven_day.as_ref()),
                Duration::days(7),
                now,
            )
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "this node does not know when the weekly window resets. Install the status line, or pick a time."
                )
            })? + margin,
            "when the weekly window resets".into(),
        ),
        ScheduleWhen::At { at } => {
            if at <= now {
                anyhow::bail!("that time has passed");
            }
            (at, "at the time you picked".into())
        }
    })
}

/// One card the user booked for a time.
pub struct Booked {
    pub card_id: String,
    pub title: String,
    pub project_name: Option<String>,
    pub model: Option<String>,
    pub at: DateTime<Utc>,
    pub reason: String,
    pub estimate: Estimate,
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
/// a priority. A candidate that does not fit stays in the plan as an item that
/// does not fit, so the user can still pick it. An automatic trigger returns no
/// plan when nothing fits. A manual request always returns the full list.
pub fn plan(
    trigger: ProposalTrigger,
    reason: String,
    mut candidates: Vec<Candidate>,
    ctx: &Context<'_>,
    s: &Settings,
) -> Option<Proposal> {
    let manual = trigger == ProposalTrigger::Manual;
    let budget = budget_pct(ctx, s);
    if budget <= 1.0 && !manual {
        return None;
    }
    let slots = s.max_parallel_bg.saturating_sub(ctx.running_jobs);
    if slots == 0 && !manual {
        return None;
    }
    if candidates.is_empty() {
        return None;
    }
    candidates.sort_by(|a, b| {
        b.priority
            .cmp(&a.priority)
            .then(a.created_at.cmp(&b.created_at))
    });
    let mut items: Vec<ProposalItem> = vec![];
    let mut total = 0.0f64;
    let mut taken = 0u32;
    let mut skipped = 0u32;
    for c in candidates {
        let skip_reason = if taken >= slots {
            Some("slots")
        } else if total + c.estimate.pct_five_hour > budget {
            Some("budget")
        } else {
            None
        };
        let fits = skip_reason.is_none();
        if fits {
            total += c.estimate.pct_five_hour;
            taken += 1;
        } else {
            skipped += 1;
        }
        items.push(ProposalItem {
            card_id: c.card_id,
            title: c.title,
            project_name: c.project_name,
            prompt: c.prompt,
            model: c.model,
            estimate: c.estimate,
            job_id: None,
            error: None,
            fits,
            skip_reason: skip_reason.map(str::to_owned),
        });
    }
    if taken == 0 && !manual {
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
/// the budget or not, with the runs the user booked for a time in front of
/// them. This is what the queue strip shows. It starts nothing.
///
/// A booked run is a manual start with a delay, so `blocked` never speaks for
/// one: the field says why the *planner* would take no step.
pub fn queue(
    mut booked: Vec<Booked>,
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

    // The booked runs come first, soonest first. A run that starts after the
    // 5-hour window resets meets an empty window, so the running total starts
    // again at the first such run instead of adding to a percent that will be
    // gone. It starts again once only: the sample names one reset, so a run
    // two windows out is as far as this can honestly reach.
    booked.sort_by_key(|b| b.at);
    let reset = ctx
        .quota
        .and_then(|q| q.five_hour.as_ref())
        .and_then(|w| w.resets_at);
    let mut base = used;
    let mut booked_total = 0.0f64;
    let mut crossed = false;
    for b in booked {
        if !crossed && reset.is_some_and(|r| b.at >= r) {
            base = 0.0;
            booked_total = 0.0;
            crossed = true;
        }
        booked_total += b.estimate.pct_five_hour;
        steps.push(QueueStep {
            card_id: b.card_id,
            title: b.title,
            project_name: b.project_name,
            model: b.model,
            window_after_pct: base + booked_total,
            estimate: b.estimate,
            fits: true,
            starts_at: Some(b.at),
            reason: b.reason,
            scheduled: true,
        });
    }

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
            scheduled: false,
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
        // Every candidate stays in the list. Two fit, one does not.
        assert_eq!(p.items.len(), 3);
        assert_eq!(p.items[0].card_id, "urgent");
        assert!(p.items[0].fits);
        assert_eq!(p.items[1].card_id, "big");
        assert!(!p.items[1].fits);
        assert_eq!(p.items[1].skip_reason.as_deref(), Some("budget"));
        assert_eq!(p.items[2].card_id, "small");
        assert!(p.items[2].fits);
        assert_eq!(p.skipped, 1);
        assert!((p.total_pct - 4.0).abs() < 0.001);
        assert!((p.used_pct_after - 84.0).abs() < 0.001);
    }

    #[test]
    fn a_full_window_plans_nothing_by_itself() {
        let q = quota(90.0, 10.0, 100);
        assert!(plan(
            ProposalTrigger::IdleFiveHour,
            "m".into(),
            vec![candidate("a", 1.0, 0)],
            &ctx(&q, 0),
            &settings()
        )
        .is_none());
    }

    #[test]
    fn a_manual_plan_lists_what_does_not_fit() {
        let q = quota(90.0, 10.0, 100);
        let p = plan(
            ProposalTrigger::Manual,
            "m".into(),
            vec![candidate("a", 1.0, 0)],
            &ctx(&q, 0),
            &settings(),
        )
        .unwrap();
        assert_eq!(p.items.len(), 1);
        assert!(!p.items[0].fits);
        assert_eq!(p.skipped, 1);
        assert!(p.total_pct.abs() < 0.001);
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
        assert_eq!(p.items.iter().filter(|i| i.fits).count(), 1);
        assert_eq!(p.items[1].skip_reason.as_deref(), Some("slots"));
        assert_eq!(p.items[2].skip_reason.as_deref(), Some("slots"));
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
            vec![],
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
            vec![],
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
            vec![],
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
            vec![],
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

    fn booked(name: &str, pct: f64, at: DateTime<Utc>) -> Booked {
        let c = candidate(name, pct, 0);
        Booked {
            card_id: c.card_id,
            title: c.title,
            project_name: None,
            model: None,
            at,
            reason: "when the 5-hour window resets".into(),
            estimate: c.estimate,
        }
    }

    #[test]
    fn the_next_reset_steps_past_a_sample_taken_before_it() {
        // A machine that slept reports a reset time that has passed. The next
        // reset is one window later, not the time in the sample.
        let now = Utc::now();
        let w = QuotaWindow {
            used_percentage: 90.0,
            resets_at: Some(now - Duration::hours(6)),
        };
        let q = QuotaSample {
            at: now,
            five_hour: Some(w),
            seven_day: None,
            source: "test".into(),
        };
        let (at, _) = resolve_schedule(ScheduleWhen::NextReset, Some(&q), now).unwrap();
        assert!(at > now, "the reset must be in the future");
        assert!(at - now < FIVE_HOUR_WINDOW);
    }

    #[test]
    fn the_following_cycle_is_one_window_after_the_next_reset() {
        let now = Utc::now();
        let q = quota(90.0, 20.0, 100);
        let (first, _) = resolve_schedule(ScheduleWhen::NextReset, Some(&q), now).unwrap();
        let (second, why) = resolve_schedule(ScheduleWhen::FollowingCycle, Some(&q), now).unwrap();
        assert_eq!(second - first, FIVE_HOUR_WINDOW);
        assert!(why.contains("after the next one"));
    }

    #[test]
    fn a_node_without_a_sample_says_so_instead_of_guessing() {
        // A guessed reset time would start a run into a window that is still
        // full, which is the failure the user asked kari to avoid.
        let e = resolve_schedule(ScheduleWhen::NextReset, None, Utc::now()).unwrap_err();
        assert!(e.to_string().contains("does not know"));
    }

    #[test]
    fn a_time_that_has_passed_is_refused() {
        let now = Utc::now();
        let at = now - Duration::minutes(1);
        assert!(resolve_schedule(ScheduleWhen::At { at }, None, now).is_err());
    }

    #[test]
    fn a_booked_run_fits_although_automation_is_off() {
        // The user set the time, so the mode, the budget and the slots do not
        // speak for it. Only the planner's own steps read `blocked`.
        let q = quota(95.0, 99.0, 200);
        let at = Utc::now() + Duration::hours(1);
        let plan = queue(
            vec![booked("a", 5.0, at)],
            vec![candidate("b", 5.0, 0)],
            &ctx(&q, 0),
            &settings(),
            AutomationMode::Off,
            false,
            Utc::now(),
        );
        assert_eq!(plan.blocked.as_deref(), Some("automation is off"));
        assert!(plan.steps[0].scheduled);
        assert!(plan.steps[0].fits);
        assert_eq!(plan.steps[0].starts_at, Some(at));
        assert!(!plan.steps[1].scheduled);
        assert!(!plan.steps[1].fits);
    }

    #[test]
    fn a_booked_run_after_the_reset_counts_against_an_empty_window() {
        // The 5-hour window is at 80 percent and resets in 3 hours. A run
        // booked after that meets a window at zero, so the strip must not add
        // its cost to a percent that will be gone.
        let q = quota(80.0, 10.0, 200);
        let after = Utc::now() + Duration::hours(4);
        let plan = queue(
            vec![booked("a", 6.0, after)],
            vec![],
            &ctx(&q, 0),
            &settings(),
            AutomationMode::Ask,
            false,
            Utc::now(),
        );
        assert!((plan.steps[0].window_after_pct - 6.0).abs() < 0.01);
    }

    #[test]
    fn two_booked_runs_in_the_new_window_add_up() {
        // Both runs meet the window that opens at the reset, so the second one
        // must count on top of the first, not start the total again.
        let q = quota(80.0, 10.0, 200);
        let a = Utc::now() + Duration::hours(4);
        let b = Utc::now() + Duration::hours(5);
        let plan = queue(
            vec![booked("a", 6.0, a), booked("b", 4.0, b)],
            vec![],
            &ctx(&q, 0),
            &settings(),
            AutomationMode::Ask,
            false,
            Utc::now(),
        );
        assert!((plan.steps[0].window_after_pct - 6.0).abs() < 0.01);
        assert!((plan.steps[1].window_after_pct - 10.0).abs() < 0.01);
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
