//! Session summaries from Haiku through `claude -p`.
//!
//! The call runs with `--setting-sources ""`, no tools, no MCP servers and
//! `--no-session-persistence`: it runs no hooks, keeps the system prompt small
//! and leaves no transcript behind. `--bare` is not used: it breaks OAuth login.

use crate::model::{DerivedState, SessionFacts, Summary};
use crate::{paths, transcript};
use chrono::Utc;
use serde_json::Value;
use std::io::Write;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::Duration;

const SYSTEM: &str = r#"You summarize a Claude Code session for a Kanban board. You get the tail of the transcript: USER and ASSISTANT lines.
Reply with one JSON object only, no prose, no code fence:
{"narrative": "<two short sentences: what the session is about and where it stands>",
 "open_questions": ["<question the user still has to answer>", ...],
 "next_step": "<the single next action, or null>",
 "judged_state": "<one of: working, my_turn, needs_decision, needs_approval, waiting_on_others, validate, done, unknown>",
 "confidence": <0.0 to 1.0>}
Meaning of judged_state:
- needs_decision: the assistant asked the user to choose between options.
- needs_approval: the assistant waits for permission or plan approval.
- waiting_on_others: the next step depends on a third party (review, reply, deploy, access).
- validate: the work looks complete but nobody verified it yet.
- done: the user confirmed the work is finished.
- my_turn: the assistant finished a turn and waits for the user's next instruction.
- working: the assistant is in the middle of a task.
Use unknown when unsure. Keep the narrative under 60 words."#;

/// Build the excerpt: the last `max_messages` user and assistant messages, text only.
pub fn excerpt(transcript_path: &Path, max_messages: usize) -> anyhow::Result<String> {
    let lines = transcript::tail_messages(transcript_path, max_messages, 24 << 20)?;
    Ok(lines.join("\n"))
}

fn extract_json(s: &str) -> Option<Value> {
    let start = s.find('{')?;
    let end = s.rfind('}')?;
    if end <= start {
        return None;
    }
    serde_json::from_str(&s[start..=end]).ok()
}

fn parse_state(s: &str) -> DerivedState {
    use DerivedState::*;
    match s {
        "working" => Working,
        "my_turn" => MyTurn,
        "needs_decision" => NeedsDecision,
        "needs_approval" => NeedsApproval,
        "waiting_on_others" => WaitingOnOthers,
        "validate" => Validate,
        "done" => Done,
        _ => Unknown,
    }
}

/// Run one Haiku call for the session. Blocks up to 90 seconds.
pub fn generate(facts: &SessionFacts, model: &str) -> anyhow::Result<Summary> {
    let claude =
        paths::which("claude").ok_or_else(|| anyhow::anyhow!("claude not found on PATH"))?;
    let input = excerpt(Path::new(&facts.transcript_path), 30)?;
    if input.trim().is_empty() {
        anyhow::bail!("empty transcript excerpt");
    }
    let workdir = paths::kari_dir().join("summaries");
    std::fs::create_dir_all(&workdir)?;
    let mut cmd = Command::new(claude);
    cmd.current_dir(&workdir)
        .env("PATH", paths::child_path())
        .env_remove("CLAUDECODE")
        .env_remove("CLAUDE_CODE_ENTRYPOINT")
        .args([
            "-p",
            "--no-session-persistence",
            "--setting-sources",
            "",
            "--tools",
            "",
            "--strict-mcp-config",
            "--mcp-config",
            r#"{"mcpServers":{}}"#,
            "--model",
            model,
            "--output-format",
            "json",
            "--max-turns",
            "1",
            "--permission-mode",
            "default",
            "--append-system-prompt",
            SYSTEM,
            "Summarize this session transcript excerpt as JSON.",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = cmd.spawn()?;
    {
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| anyhow::anyhow!("no stdin"))?;
        stdin.write_all(input.as_bytes())?;
    }
    let deadline = std::time::Instant::now() + Duration::from_secs(90);
    loop {
        if child.try_wait()?.is_some() {
            break;
        }
        if std::time::Instant::now() > deadline {
            let _ = child.kill();
            anyhow::bail!("summary call timed out");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let out = child.wait_with_output()?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        anyhow::bail!(
            "claude -p failed: {}",
            crate::truncate(&format!("{err} {stdout}"), 300)
        );
    }
    let outer: Value = serde_json::from_str(stdout.trim())
        .or_else(|_| extract_json(&stdout).ok_or_else(|| anyhow::anyhow!("no JSON in output")))?;
    if outer.get("is_error").and_then(|b| b.as_bool()) == Some(true) {
        anyhow::bail!(
            "claude -p reported an error: {}",
            crate::truncate(&outer.to_string(), 300)
        );
    }
    let result_text = outer.get("result").and_then(|r| r.as_str()).unwrap_or("");
    let inner = extract_json(result_text).or_else(|| {
        if outer.get("narrative").is_some() {
            Some(outer.clone())
        } else {
            None
        }
    });
    let Some(inner) = inner else {
        anyhow::bail!("summary is not JSON: {}", crate::truncate(result_text, 200));
    };
    let s = |k: &str| {
        inner
            .get(k)
            .and_then(|v| v.as_str())
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty() && v != "null")
    };
    let open_questions = inner
        .get("open_questions")
        .and_then(|v| v.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|x| x.as_str())
                .map(|x| x.trim().to_string())
                .filter(|x| !x.is_empty())
                .collect()
        })
        .unwrap_or_default();
    Ok(Summary {
        session_id: facts.session_id.clone(),
        narrative: s("narrative").unwrap_or_default(),
        open_questions,
        next_step: s("next_step"),
        judged_state: parse_state(&s("judged_state").unwrap_or_default()),
        confidence: inner
            .get("confidence")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
            .clamp(0.0, 1.0),
        generated_at: Utc::now(),
        source: "haiku".into(),
        based_on_at: facts.last_at,
        model: outer
            .get("model")
            .and_then(|m| m.as_str())
            .map(|m| m.to_string())
            .or_else(|| Some(model.to_string())),
    })
}
