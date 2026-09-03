//! Rate-limit samples. Primary source: the status line wrapper writes
//! `~/.config/kari/rate-limits.json` on every refresh.

use crate::model::{QuotaSample, QuotaWindow};
use crate::paths;
use chrono::{DateTime, TimeZone, Utc};
use serde_json::Value;

fn window(v: Option<&Value>) -> Option<QuotaWindow> {
    let v = v?;
    let used = v.get("used_percentage")?.as_f64()?;
    let resets_at = v
        .get("resets_at")
        .and_then(|r| r.as_i64())
        .and_then(|s| Utc.timestamp_opt(s, 0).single());
    Some(QuotaWindow {
        used_percentage: used,
        resets_at,
    })
}

/// Parse a status line payload (or the wrapper's envelope around it).
pub fn sample_from_statusline(v: &Value, at: DateTime<Utc>) -> Option<QuotaSample> {
    let rl = v
        .get("rate_limits")
        .or_else(|| v.get("payload").and_then(|p| p.get("rate_limits")))?;
    let five_hour = window(rl.get("five_hour"));
    let seven_day = window(rl.get("seven_day"));
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    Some(QuotaSample {
        at,
        five_hour,
        seven_day,
        source: "statusline".into(),
    })
}

pub fn read_latest() -> Option<QuotaSample> {
    let p = paths::rate_limits_file();
    let text = std::fs::read_to_string(&p).ok()?;
    let v: Value = serde_json::from_str(&text).ok()?;
    let at = v
        .get("ts")
        .and_then(|t| t.as_i64())
        .and_then(|s| Utc.timestamp_opt(s, 0).single())
        .or_else(|| {
            std::fs::metadata(&p)
                .ok()
                .and_then(|m| m.modified().ok())
                .map(DateTime::<Utc>::from)
        })
        .unwrap_or_else(Utc::now);
    sample_from_statusline(&v, at)
}

/// Shell wrapper installed around the user's status line command.
pub fn wrapper_script(original_command: &str) -> String {
    let file = paths::rate_limits_file();
    format!(
        "#!/bin/bash\n# kari status line wrapper. Captures rate limits, then runs the original status line.\n\
         input=$(cat)\n\
         printf '%s' \"$input\" | jq -c --argjson ts \"$(date +%s)\" '{{ts:$ts, session_id:.session_id, rate_limits:.rate_limits}}' > '{}.tmp' 2>/dev/null && mv -f '{}.tmp' '{}'\n\
         printf '%s' \"$input\" | {}\n",
        file.display(),
        file.display(),
        file.display(),
        original_command
    )
}

// ------------------------------------------------------------------ fallback

/// Minimum gap between two calls to the usage endpoint.
const ENDPOINT_FLOOR_SECS: u64 = 180;
/// The status line is the primary source. Older than this, kari asks the endpoint.
pub const STALE_AFTER_SECS: i64 = 300;

static LAST_FETCH: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);

fn field_pct(v: &Value) -> Option<f64> {
    for k in [
        "used_percentage",
        "utilization",
        "usedPercentage",
        "used_pct",
        "percent_used",
    ] {
        if let Some(n) = v.get(k).and_then(|x| x.as_f64()) {
            // Some shapes report a 0..1 fraction.
            return Some(if n > 0.0 && n <= 1.0 && k == "utilization" {
                n * 100.0
            } else {
                n
            });
        }
    }
    None
}

fn field_reset(v: &Value) -> Option<DateTime<Utc>> {
    for k in ["resets_at", "resetsAt", "reset_at", "resets_at_unix"] {
        if let Some(x) = v.get(k) {
            if let Some(secs) = x.as_i64() {
                return Utc.timestamp_opt(secs, 0).single();
            }
            if let Some(s) = x.as_str() {
                if let Ok(d) = DateTime::parse_from_rfc3339(s) {
                    return Some(d.with_timezone(&Utc));
                }
            }
        }
    }
    None
}

fn window_any(v: &Value) -> Option<QuotaWindow> {
    let used = field_pct(v)?;
    Some(QuotaWindow {
        used_percentage: used,
        resets_at: field_reset(v),
    })
}

/// Read both windows out of an undocumented usage payload. Tolerant by design:
/// the endpoint is not a contract, so kari accepts several key styles.
pub fn sample_from_usage(v: &Value, at: DateTime<Utc>) -> Option<QuotaSample> {
    let root = v
        .get("rate_limits")
        .or_else(|| v.get("rateLimits"))
        .unwrap_or(v);
    let pick = |names: &[&str]| -> Option<QuotaWindow> {
        for n in names {
            if let Some(w) = root.get(*n).and_then(window_any) {
                return Some(w);
            }
        }
        None
    };
    let five = pick(&["five_hour", "fiveHour", "five_hour_limit", "session"]);
    let seven = pick(&["seven_day", "sevenDay", "seven_day_limit", "weekly"]);
    if five.is_none() && seven.is_none() {
        return None;
    }
    Some(QuotaSample {
        at,
        five_hour: five,
        seven_day: seven,
        source: "endpoint".into(),
    })
}

/// The Claude Code login token. Read from the keychain, never written to a log.
fn oauth_token() -> Option<String> {
    let from_json = |text: &str| -> Option<String> {
        let v: Value = serde_json::from_str(text).ok()?;
        for path in [
            &["claudeAiOauth", "accessToken"][..],
            &["accessToken"][..],
            &["oauth", "access_token"][..],
        ] {
            let mut cur = &v;
            let mut ok = true;
            for k in path {
                match cur.get(*k) {
                    Some(next) => cur = next,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                if let Some(s) = cur.as_str() {
                    return Some(s.to_string());
                }
            }
        }
        None
    };
    if cfg!(target_os = "macos") {
        let out = std::process::Command::new("/usr/bin/security")
            .args([
                "find-generic-password",
                "-s",
                "Claude Code-credentials",
                "-w",
            ])
            .stdin(std::process::Stdio::null())
            .output()
            .ok()?;
        if out.status.success() {
            if let Some(t) = from_json(String::from_utf8_lossy(&out.stdout).trim()) {
                return Some(t);
            }
        }
    }
    let f = paths::claude_dir().join(".credentials.json");
    std::fs::read_to_string(f).ok().and_then(|t| from_json(&t))
}

/// Ask the OAuth usage endpoint. Rate limited to one call per 3 minutes.
/// The token goes to curl through stdin, so it never appears in the process list.
pub fn fetch_usage() -> anyhow::Result<QuotaSample> {
    {
        let mut last = LAST_FETCH.lock().unwrap();
        if let Some(t) = *last {
            if t.elapsed().as_secs() < ENDPOINT_FLOOR_SECS {
                anyhow::bail!("usage endpoint asked less than {ENDPOINT_FLOOR_SECS}s ago");
            }
        }
        *last = Some(std::time::Instant::now());
    }
    let token = oauth_token().ok_or_else(|| anyhow::anyhow!("no Claude Code login token found"))?;
    let version = crate::version();
    let config = format!(
        "url = \"https://api.anthropic.com/api/oauth/usage\"\n\
         header = \"Authorization: Bearer {token}\"\n\
         header = \"anthropic-beta: oauth-2025-04-20\"\n\
         header = \"Accept: application/json\"\n\
         user-agent = \"kari/{version}\"\n\
         silent\n\
         show-error\n\
         max-time = 10\n"
    );
    let mut child = std::process::Command::new("/usr/bin/curl")
        .args(["--config", "-"])
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    {
        use std::io::Write;
        let mut si = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("curl stdin"))?;
        si.write_all(config.as_bytes())?;
    }
    let out = child.wait_with_output()?;
    if !out.status.success() {
        anyhow::bail!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    let body = String::from_utf8_lossy(&out.stdout).to_string();
    let v: Value = serde_json::from_str(&body)
        .map_err(|e| anyhow::anyhow!("usage endpoint sent no JSON: {e}"))?;
    sample_from_usage(&v, Utc::now())
        .ok_or_else(|| anyhow::anyhow!("usage payload holds no rate limit windows"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_a_fraction_utilization() {
        let v: Value = serde_json::from_str(
            r#"{"five_hour":{"utilization":0.42,"resets_at":1800000000},"seven_day":{"utilization":11.5}}"#,
        )
        .unwrap();
        let s = sample_from_usage(&v, Utc::now()).unwrap();
        assert!((s.five_hour.unwrap().used_percentage - 42.0).abs() < 0.001);
        assert!((s.seven_day.unwrap().used_percentage - 11.5).abs() < 0.001);
    }

    #[test]
    fn reads_camel_case_and_strings() {
        let v: Value =
            serde_json::from_str(r#"{"rateLimits":{"fiveHour":{"used_percentage":7,"resetsAt":"2026-09-03T05:00:00Z"}}}"#).unwrap();
        let s = sample_from_usage(&v, Utc::now()).unwrap();
        let w = s.five_hour.unwrap();
        assert_eq!(w.used_percentage, 7.0);
        assert!(w.resets_at.is_some());
    }

    #[test]
    fn rejects_an_unrelated_payload() {
        let v: Value =
            serde_json::from_str(r#"{"error":{"type":"authentication_error"}}"#).unwrap();
        assert!(sample_from_usage(&v, Utc::now()).is_none());
    }
}
