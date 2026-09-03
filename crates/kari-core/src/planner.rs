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
}
