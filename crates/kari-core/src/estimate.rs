//! Calibration of rate-limit percent per token, and per-task estimates.
//!
//! Rate limits are percentages. Transcripts are tokens. kari learns the factor
//! between them from pairs of quota samples and the token growth between them.

use crate::model::*;
use chrono::Duration;
use std::collections::HashMap;

/// Percent of the 5-hour window per million weighted tokens, before kari learns.
/// Measured from about 12 hours of samples on one subscription. The band is
/// wide on purpose. Five learned pairs replace it.
pub const PRIOR_PCT_PER_MTOK: f64 = 3.0;
/// Turns a continuation of an existing session is expected to run.
pub const CONTINUE_TURNS: f64 = 8.0;
/// Weighted tokens for a task when no session history exists.
pub const DEFAULT_TASK_TOKENS: f64 = 4_000_000.0;
/// Pairs needed before kari trusts the learned factor.
pub const MIN_PAIRS: usize = 5;
/// A pair of samples further apart than this can hold work kari did not see.
pub const MAX_PAIR_GAP_MINUTES: i64 = 30;
/// Token growth below this is noise.
pub const MIN_PAIR_TOKENS: f64 = 20_000.0;
/// Percent growth below this is rounding.
pub const MIN_PAIR_PCT: f64 = 0.05;
/// Ratios outside this band come from bad pairs, not from work.
pub const MIN_RATIO: f64 = 0.05;
pub const MAX_RATIO: f64 = 50.0;

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[idx.min(sorted.len() - 1)]
}

fn sorted_of(mut v: Vec<f64>) -> Vec<f64> {
    v.retain(|x| x.is_finite() && *x > 0.0);
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v
}

fn prior() -> Calibration {
    Calibration::default()
}

/// Learn percent per million weighted tokens from the quota samples.
///
/// The status line reports whole percent steps and refreshes every few seconds,
/// so consecutive samples usually read the same number. kari first reduces the
/// series to the points where the number changed, then pairs those.
/// `samples` must be sorted by time, oldest first. `deltas` may be in any order.
pub fn calibrate(samples: &[QuotaSample], deltas: &[TokenDelta]) -> Calibration {
    // Prefix sums over the delta timestamps: a pair then costs one binary search.
    let mut times: Vec<i64> = deltas.iter().map(|d| d.at.timestamp()).collect();
    times.sort_unstable();
    let mut order: Vec<usize> = (0..deltas.len()).collect();
    order.sort_unstable_by_key(|i| deltas[*i].at.timestamp());
    let mut prefix: Vec<f64> = Vec::with_capacity(deltas.len() + 1);
    prefix.push(0.0);
    for i in &order {
        let last = *prefix.last().unwrap();
        prefix.push(last + deltas[*i].weighted);
    }
    // Weighted tokens recorded in (from, to].
    let span = |from: i64, to: i64| -> f64 {
        let lo = times.partition_point(|t| *t <= from);
        let hi = times.partition_point(|t| *t <= to);
        prefix[hi] - prefix[lo]
    };
    // Reduce to the points where the reported percent or the window changed.
    let mut points: Vec<&QuotaSample> = vec![];
    for s in samples.iter().filter(|s| s.five_hour.is_some()) {
        let f = s.five_hour.as_ref().unwrap();
        match points.last().and_then(|p| p.five_hour.as_ref()) {
            Some(prev)
                if prev.used_percentage == f.used_percentage && prev.resets_at == f.resets_at => {}
            _ => points.push(s),
        }
    }
    let mut ratios: Vec<f64> = vec![];
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let (Some(fa), Some(fb)) = (a.five_hour.as_ref(), b.five_hour.as_ref()) else {
            continue;
        };
        let gap = b.at - a.at;
        if gap <= Duration::zero() || gap > Duration::minutes(MAX_PAIR_GAP_MINUTES) {
            continue;
        }
        // A window reset between the samples makes the percent delta meaningless.
        if let (Some(ra), Some(rb)) = (fa.resets_at, fb.resets_at) {
            if ra != rb {
                continue;
            }
        }
        let d_pct = fb.used_percentage - fa.used_percentage;
        if d_pct < MIN_PAIR_PCT {
            continue;
        }
        let tok = span(a.at.timestamp(), b.at.timestamp());
        if tok < MIN_PAIR_TOKENS {
            continue;
        }
        let ratio = d_pct / (tok / 1_000_000.0);
        if ratio.is_finite() && (MIN_RATIO..=MAX_RATIO).contains(&ratio) {
            ratios.push(ratio);
        }
    }
    let ratios = sorted_of(ratios);
    if ratios.len() < MIN_PAIRS {
        return prior();
    }
    Calibration {
        pct_per_mtok: percentile(&ratios, 0.5),
        low: percentile(&ratios, 0.25),
        high: percentile(&ratios, 0.75),
        samples: ratios.len() as u32,
        source: "learned".into(),
        updated_at: chrono::Utc::now(),
    }
}

/// Weighted token totals of sessions that carry real work, per project and global.
/// Built once per board render, then shared by every card.
#[derive(Debug, Clone, Default)]
pub struct Pools {
    by_cwd: HashMap<String, Vec<f64>>,
    global: Vec<f64>,
}

pub fn pools(facts: &HashMap<String, SessionFacts>) -> Pools {
    let mut by_cwd: HashMap<String, Vec<f64>> = HashMap::new();
    let mut global: Vec<f64> = vec![];
    for f in facts.values() {
        if f.turns < 2 {
            continue;
        }
        let w = f.tokens.weighted();
        if w <= 100_000.0 {
            continue;
        }
        global.push(w);
        if let Some(c) = &f.cwd {
            by_cwd.entry(c.clone()).or_default().push(w);
        }
    }
    Pools {
        by_cwd: by_cwd.into_iter().map(|(k, v)| (k, sorted_of(v))).collect(),
        global: sorted_of(global),
    }
}

/// Estimate the cost of one run of this card, from prepared pools.
///
/// A task card starts a new session, so the estimate is a whole session.
/// A session card continues, so the estimate is the cost of a few more turns.
pub fn estimate_with(
    p: &Pools,
    card: &Card,
    session: Option<&SessionFacts>,
    cal: &Calibration,
) -> Estimate {
    let mk = |tokens: f64, low: f64, high: f64, source: &str, n: usize| Estimate {
        weighted_tokens: tokens,
        low,
        high,
        pct_five_hour: tokens / 1_000_000.0 * cal.pct_per_mtok,
        pct_low: low / 1_000_000.0 * cal.low,
        pct_high: high / 1_000_000.0 * cal.high,
        source: source.into(),
        sessions: n as u32,
    };
    if let Some(t) = card.estimate_weighted_tokens {
        return mk(t, t * 0.7, t * 1.4, "manual", 0);
    }
    // Continuing a session costs a few turns at this session's own rate.
    if let Some(f) = session {
        if f.turns >= 2 {
            let per_turn = f.tokens.weighted() / f.turns as f64;
            let t = per_turn * CONTINUE_TURNS;
            return mk(t, t * 0.5, t * 2.0, "continue", f.turns as usize);
        }
    }
    let cwd = card
        .project_cwd
        .as_deref()
        .or_else(|| session.and_then(|f| f.cwd.as_deref()));
    if let Some(v) = cwd.and_then(|c| p.by_cwd.get(c)) {
        if v.len() >= 3 {
            return mk(
                percentile(v, 0.5),
                percentile(v, 0.25),
                percentile(v, 0.75),
                "project",
                v.len(),
            );
        }
    }
    if p.global.len() >= 3 {
        let g = &p.global;
        return mk(
            percentile(g, 0.5),
            percentile(g, 0.25),
            percentile(g, 0.75),
            "global",
            g.len(),
        );
    }
    mk(
        DEFAULT_TASK_TOKENS,
        DEFAULT_TASK_TOKENS * 0.5,
        DEFAULT_TASK_TOKENS * 1.5,
        "default",
        0,
    )
}

/// Convenience for one card outside a board render.
pub fn estimate_for(
    card: &Card,
    facts: &HashMap<String, SessionFacts>,
    cal: &Calibration,
) -> Estimate {
    let session = card.session_id.as_ref().and_then(|s| facts.get(s));
    estimate_with(&pools(facts), card, session, cal)
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample(min: i64, pct: f64) -> QuotaSample {
        QuotaSample {
            at: Utc.timestamp_opt(1_700_000_000 + min * 60, 0).unwrap(),
            five_hour: Some(QuotaWindow {
                used_percentage: pct,
                resets_at: Utc.timestamp_opt(1_800_000_000, 0).single(),
            }),
            seven_day: None,
            source: "statusline".into(),
        }
    }

    #[test]
    fn learns_the_factor_from_clean_pairs() {
        // 2 percent per 1M weighted tokens, six pairs.
        let mut samples = vec![];
        let mut deltas = vec![];
        for i in 0..7 {
            samples.push(sample(i * 5, i as f64 * 2.0));
            if i > 0 {
                deltas.push(TokenDelta {
                    at: Utc
                        .timestamp_opt(1_700_000_000 + (i * 5 - 1) * 60, 0)
                        .unwrap(),
                    session_id: "s".into(),
                    weighted: 1_000_000.0,
                });
            }
        }
        let c = calibrate(&samples, &deltas);
        assert_eq!(c.source, "learned");
        assert!(
            (c.pct_per_mtok - 2.0).abs() < 0.001,
            "got {}",
            c.pct_per_mtok
        );
    }

    #[test]
    fn pairs_across_repeated_readings() {
        // The status line repeats the same whole percent between changes.
        let mut samples = vec![];
        let mut deltas = vec![];
        let mut t = 0i64;
        for step in 0..7 {
            for _ in 0..4 {
                samples.push(sample(t, step as f64 * 2.0));
                t += 1;
            }
            deltas.push(TokenDelta {
                at: Utc.timestamp_opt(1_700_000_000 + (t - 1) * 60, 0).unwrap(),
                session_id: "s".into(),
                weighted: 1_000_000.0,
            });
        }
        let c = calibrate(&samples, &deltas);
        assert_eq!(c.source, "learned");
        assert!(c.pct_per_mtok > 0.0);
    }

    #[test]
    fn falls_back_to_the_prior() {
        let c = calibrate(&[sample(0, 1.0), sample(5, 2.0)], &[]);
        assert_eq!(c.source, "prior");
        assert_eq!(c.samples, 0);
    }

    #[test]
    fn ignores_a_pair_that_spans_a_reset() {
        let mut a = sample(0, 90.0);
        let mut b = sample(5, 1.0);
        a.five_hour.as_mut().unwrap().resets_at = Utc.timestamp_opt(1_800_000_000, 0).single();
        b.five_hour.as_mut().unwrap().resets_at = Utc.timestamp_opt(1_800_018_000, 0).single();
        let d = vec![TokenDelta {
            at: Utc.timestamp_opt(1_700_000_100, 0).unwrap(),
            session_id: "s".into(),
            weighted: 1_000_000.0,
        }];
        assert_eq!(calibrate(&[a, b], &d).source, "prior");
    }
}
