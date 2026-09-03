//! Live session registry: `~/.claude/sessions/<pid>.json`, one file per running Claude Code process.

use crate::model::LiveSession;
use crate::paths;
use chrono::{DateTime, TimeZone, Utc};
use serde::Deserialize;
use std::collections::HashMap;

#[derive(Deserialize)]
struct Raw {
    pid: u32,
    #[serde(rename = "sessionId")]
    session_id: String,
    cwd: String,
    name: Option<String>,
    #[serde(rename = "nameSource")]
    name_source: Option<String>,
    status: Option<String>,
    kind: Option<String>,
    #[serde(rename = "startedAt")]
    started_at: Option<i64>,
    #[serde(rename = "statusUpdatedAt")]
    status_updated_at: Option<i64>,
}

fn ms(v: Option<i64>) -> Option<DateTime<Utc>> {
    v.and_then(|m| Utc.timestamp_millis_opt(m).single())
}

pub fn pid_alive(pid: u32) -> bool {
    // kill(pid, 0) succeeds when the process exists and we may signal it.
    unsafe { libc::kill(pid as i32, 0) == 0 }
}

/// Read every registry file. Dead pids are kept with `alive = false` so the caller can prune.
pub fn read_all() -> HashMap<String, LiveSession> {
    let mut out = HashMap::new();
    let dir = paths::claude_sessions_dir();
    let Ok(rd) = std::fs::read_dir(&dir) else {
        return out;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("json") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let Ok(raw) = serde_json::from_str::<Raw>(&text) else {
            continue;
        };
        let alive = pid_alive(raw.pid);
        let live = LiveSession {
            pid: raw.pid,
            session_id: raw.session_id.clone(),
            cwd: raw.cwd,
            name: raw.name,
            name_source: raw.name_source,
            status: raw.status,
            kind: raw.kind,
            started_at: ms(raw.started_at),
            status_updated_at: ms(raw.status_updated_at),
            alive,
        };
        // Prefer an alive entry when two files claim the same session id.
        match out.get(&raw.session_id) {
            Some(prev @ LiveSession { alive: true, .. }) if !alive => {
                let _ = prev;
            }
            _ => {
                out.insert(raw.session_id, live);
            }
        }
    }
    out
}
