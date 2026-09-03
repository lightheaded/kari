//! Background agents: `claude agents --json --all` plus `~/.claude/jobs/<id>/state.json`.

use crate::model::BgJob;
use crate::paths;
use chrono::{TimeZone, Utc};
use serde_json::Value;
use std::process::Command;
use std::time::Duration;

pub fn list() -> anyhow::Result<Vec<BgJob>> {
    let claude =
        paths::which("claude").ok_or_else(|| anyhow::anyhow!("claude not found on PATH"))?;
    let mut cmd = Command::new(claude);
    cmd.args(["agents", "--json", "--all"])
        .env("PATH", paths::child_path());
    let out = run_with_timeout(cmd, Duration::from_secs(20))?;
    let v: Value = serde_json::from_slice(&out)?;
    let Some(arr) = v.as_array() else {
        return Ok(vec![]);
    };
    let mut jobs = vec![];
    for j in arr {
        let s = |k: &str| j.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
        let mut job = BgJob {
            id: s("id"),
            session_id: s("sessionId"),
            cwd: s("cwd"),
            kind: s("kind"),
            state: s("state"),
            status: s("status"),
            waiting_for: s("waitingFor"),
            name: s("name"),
            pid: j.get("pid").and_then(|p| p.as_u64()).map(|p| p as u32),
            started_at: j
                .get("startedAt")
                .and_then(|p| p.as_i64())
                .and_then(|m| Utc.timestamp_millis_opt(m).single()),
        };
        if job.kind.as_deref() != Some("background") && job.id.is_none() {
            // Interactive sessions come from the registry already.
            continue;
        }
        enrich_from_state_file(&mut job);
        jobs.push(job);
    }
    Ok(jobs)
}

fn enrich_from_state_file(job: &mut BgJob) {
    let Some(id) = &job.id else { return };
    // The id names a directory under ~/.claude/jobs. Never let it climb out.
    if id.is_empty() || id.contains('/') || id.contains("..") {
        return;
    }
    let p = paths::claude_jobs_dir().join(id).join("state.json");
    let Ok(text) = std::fs::read_to_string(p) else {
        return;
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return;
    };
    if job.state.is_none() {
        job.state = v
            .get("state")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
    }
    if job.waiting_for.is_none() {
        job.waiting_for = v
            .get("waitingFor")
            .and_then(|s| s.as_str())
            .map(|s| s.to_string());
    }
}

fn run_with_timeout(mut cmd: Command, timeout: Duration) -> anyhow::Result<Vec<u8>> {
    use std::process::Stdio;
    cmd.stdout(Stdio::piped())
        .stderr(Stdio::null())
        .stdin(Stdio::null());
    let mut child = cmd.spawn()?;
    let start = std::time::Instant::now();
    loop {
        if let Some(_status) = child.try_wait()? {
            let mut out = Vec::new();
            if let Some(mut so) = child.stdout.take() {
                std::io::Read::read_to_end(&mut so, &mut out)?;
            }
            return Ok(out);
        }
        if start.elapsed() > timeout {
            let _ = child.kill();
            anyhow::bail!("timed out");
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
