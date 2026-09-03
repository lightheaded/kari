//! Incremental transcript reader for `~/.claude/projects/<slug>/<session-id>.jsonl`.
//!
//! The format is internal to Claude Code. Every field is optional here, unknown
//! record types are skipped, and a broken line never stops the parse.

use crate::model::{truncate, PendingQuestion, PendingTool, SessionFacts};
use chrono::{DateTime, Utc};
use serde_json::Value;
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

/// Read new bytes from `path` starting at `facts.bytes_parsed` and fold them into `facts`.
/// Returns true when anything changed.
pub fn update(path: &Path, facts: &mut SessionFacts) -> anyhow::Result<bool> {
    let meta = std::fs::metadata(path)?;
    let len = meta.len();
    if let Ok(m) = meta.modified() {
        facts.file_mtime = Some(DateTime::<Utc>::from(m));
    }
    if len < facts.bytes_parsed {
        // Truncated or rewritten: start over.
        let keep_path = facts.transcript_path.clone();
        let keep_id = facts.session_id.clone();
        *facts = SessionFacts {
            session_id: keep_id,
            transcript_path: keep_path,
            ..Default::default()
        };
    }
    if len == facts.bytes_parsed {
        return Ok(false);
    }
    let mut f = std::fs::File::open(path)?;
    f.seek(SeekFrom::Start(facts.bytes_parsed))?;
    let mut reader = BufReader::with_capacity(1 << 16, f);
    let mut line = String::new();
    let mut consumed = facts.bytes_parsed;
    loop {
        line.clear();
        let n = reader.read_line(&mut line)?;
        if n == 0 {
            break;
        }
        if !line.ends_with('\n') {
            // Partial line: a writer is mid-append. Parse it next time.
            break;
        }
        consumed += n as u64;
        if let Ok(v) = serde_json::from_str::<Value>(&line) {
            fold(facts, &v);
        }
    }
    facts.bytes_parsed = consumed;
    Ok(true)
}

/// Parse a whole file from scratch. Used for tests and first scans.
pub fn read_full(path: &Path, session_id: &str) -> anyhow::Result<SessionFacts> {
    let mut facts = SessionFacts {
        session_id: session_id.to_string(),
        transcript_path: path.to_string_lossy().into_owned(),
        ..Default::default()
    };
    update(path, &mut facts)?;
    Ok(facts)
}

fn ts(v: &Value) -> Option<DateTime<Utc>> {
    v.get("timestamp")
        .and_then(|t| t.as_str())
        .and_then(|s| DateTime::parse_from_rfc3339(s).ok())
        .map(|d| d.with_timezone(&Utc))
}

fn text_of_content(content: &Value) -> Option<String> {
    match content {
        Value::String(s) => Some(s.clone()),
        Value::Array(parts) => {
            let mut out = String::new();
            for p in parts {
                if p.get("type").and_then(|t| t.as_str()) == Some("text") {
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        if !out.is_empty() {
                            out.push('\n');
                        }
                        out.push_str(t);
                    }
                }
            }
            if out.is_empty() {
                None
            } else {
                Some(out)
            }
        }
        _ => None,
    }
}

fn fold(f: &mut SessionFacts, v: &Value) {
    let Some(kind) = v.get("type").and_then(|t| t.as_str()) else {
        return;
    };
    if let Some(t) = ts(v) {
        if f.first_at.is_none_or(|x| t < x) {
            f.first_at = Some(t);
        }
        if f.last_at.is_none_or(|x| t > x) {
            f.last_at = Some(t);
        }
    }
    if let Some(cwd) = v.get("cwd").and_then(|c| c.as_str()) {
        f.cwd = Some(cwd.to_string());
    }
    if let Some(b) = v.get("gitBranch").and_then(|c| c.as_str()) {
        if !b.is_empty() {
            f.git_branch = Some(b.to_string());
        }
    }
    if let Some(ver) = v.get("version").and_then(|c| c.as_str()) {
        f.version = Some(ver.to_string());
    }

    match kind {
        "ai-title" => {
            if let Some(t) = v.get("aiTitle").and_then(|t| t.as_str()) {
                f.ai_title = Some(t.to_string());
            }
        }
        "custom-title" => {
            if let Some(t) = v.get("customTitle").and_then(|t| t.as_str()) {
                f.custom_title = Some(t.to_string());
            }
        }
        "permission-mode" => {
            if let Some(t) = v.get("permissionMode").and_then(|t| t.as_str()) {
                f.permission_mode = Some(t.to_string());
            }
        }
        "pr-link" => {
            for key in ["url", "prUrl", "link"] {
                if let Some(u) = v.get(key).and_then(|t| t.as_str()) {
                    if !f.pr_links.iter().any(|x| x == u) {
                        f.pr_links.push(u.to_string());
                    }
                }
            }
        }
        "system" => {
            if v.get("subtype").and_then(|s| s.as_str()) == Some("turn_duration") {
                f.turn_closed = f.pending_tools.is_empty();
            }
        }
        "user" => fold_user(f, v),
        "assistant" => fold_assistant(f, v),
        _ => {}
    }
}

fn fold_user(f: &mut SessionFacts, v: &Value) {
    if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
        return;
    }
    let Some(msg) = v.get("message") else { return };
    let content = msg.get("content").cloned().unwrap_or(Value::Null);

    // Tool results close pending tool calls.
    if let Value::Array(parts) = &content {
        let mut any_result = false;
        for p in parts {
            if p.get("type").and_then(|t| t.as_str()) == Some("tool_result") {
                any_result = true;
                if let Some(id) = p.get("tool_use_id").and_then(|t| t.as_str()) {
                    f.pending_tools.retain(|t| t.id != id);
                }
            }
        }
        if any_result {
            return;
        }
    }
    if v.get("isMeta").and_then(|b| b.as_bool()) == Some(true) {
        return;
    }
    let human = match v
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(|k| k.as_str())
    {
        Some(k) => k == "human",
        None => true,
    };
    if !human {
        return;
    }
    let Some(text) = text_of_content(&content) else {
        return;
    };
    let text = text.trim();
    if text.is_empty()
        || text.starts_with("<command-")
        || text.starts_with("<local-command")
        || text.starts_with("<system-reminder>")
        || text.starts_with("<task-notification>")
    {
        return;
    }
    f.turns += 1;
    f.turn_closed = false;
    if f.first_prompt.is_none() {
        f.first_prompt = Some(truncate(text, 300));
    }
    f.last_prompt = Some(truncate(text, 300));
    if let Some(t) = ts(v) {
        f.last_user_at = Some(t);
    }
    // A new prompt supersedes any question Claude asked before it.
    f.pending_tools.clear();
}

fn fold_assistant(f: &mut SessionFacts, v: &Value) {
    if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
        return;
    }
    let Some(msg) = v.get("message") else { return };
    if let Some(model) = msg.get("model").and_then(|m| m.as_str()) {
        if !model.starts_with('<') {
            f.models.insert(model.to_string());
        }
    }
    if let Some(u) = msg.get("usage") {
        let g = |k: &str| u.get(k).and_then(|x| x.as_u64()).unwrap_or(0);
        f.tokens.input += g("input_tokens");
        f.tokens.output += g("output_tokens");
        f.tokens.cache_read += g("cache_read_input_tokens");
        f.tokens.cache_write += g("cache_creation_input_tokens");
        f.tokens.messages += 1;
    }
    if let Some(t) = ts(v) {
        f.last_assistant_at = Some(t);
    }
    if let Some(Value::Array(parts)) = msg.get("content") {
        for p in parts {
            match p.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        if !t.trim().is_empty() {
                            f.last_assistant_text = Some(truncate(t, 400));
                        }
                    }
                }
                Some("tool_use") => {
                    let id = p
                        .get("id")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let name = p
                        .get("name")
                        .and_then(|t| t.as_str())
                        .unwrap_or("")
                        .to_string();
                    let questions = if name == "AskUserQuestion" {
                        parse_questions(p.get("input"))
                    } else {
                        vec![]
                    };
                    if !id.is_empty() {
                        f.pending_tools.push(PendingTool {
                            id,
                            name,
                            questions,
                        });
                    }
                }
                _ => {}
            }
        }
    }
    f.turn_closed = false;
}

fn parse_questions(input: Option<&Value>) -> Vec<PendingQuestion> {
    let mut out = vec![];
    let Some(qs) = input
        .and_then(|i| i.get("questions"))
        .and_then(|q| q.as_array())
    else {
        return out;
    };
    for q in qs {
        let question = q
            .get("question")
            .and_then(|s| s.as_str())
            .unwrap_or("")
            .to_string();
        let options = q
            .get("options")
            .and_then(|o| o.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|o| {
                        o.get("label")
                            .and_then(|l| l.as_str())
                            .map(|s| s.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default();
        out.push(PendingQuestion { question, options });
    }
    out
}

/// Read the tail of a transcript as plain text for summaries. Cheap, bounded.
pub fn tail_text(path: &Path, max_bytes: u64) -> anyhow::Result<String> {
    let mut f = std::fs::File::open(path)?;
    let len = f.metadata()?.len();
    let start = len.saturating_sub(max_bytes);
    f.seek(SeekFrom::Start(start))?;
    let mut buf = String::new();
    f.read_to_string(&mut buf)?;
    let mut out = String::new();
    for line in buf.lines().skip(if start > 0 { 1 } else { 0 }) {
        let Ok(v) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
        if kind != "user" && kind != "assistant" {
            continue;
        }
        let Some(text) = v
            .get("message")
            .and_then(|m| m.get("content"))
            .and_then(text_of_content)
        else {
            continue;
        };
        out.push_str(if kind == "user" {
            "USER: "
        } else {
            "ASSISTANT: "
        });
        out.push_str(&truncate(&text, 1200));
        out.push('\n');
    }
    Ok(out)
}

/// The last `n` user and assistant text messages, as "USER: …" / "ASSISTANT: …" lines.
/// Reads the file tail in growing windows, because tool results can be megabytes.
pub fn tail_messages(path: &Path, n: usize, max_bytes: u64) -> anyhow::Result<Vec<String>> {
    let len = std::fs::metadata(path)?.len();
    let mut window: u64 = 512 << 10;
    loop {
        let take = window.min(len).min(max_bytes);
        let start = len - take;
        let mut f = std::fs::File::open(path)?;
        f.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::with_capacity(take as usize);
        f.read_to_end(&mut buf)?;
        let text = String::from_utf8_lossy(&buf);
        let mut out: Vec<String> = vec![];
        for line in text.lines().skip(if start > 0 { 1 } else { 0 }) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
            if kind != "user" && kind != "assistant" {
                continue;
            }
            if v.get("isMeta").and_then(|b| b.as_bool()) == Some(true)
                || v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true)
            {
                continue;
            }
            let Some(text) = v
                .get("message")
                .and_then(|m| m.get("content"))
                .and_then(text_of_content)
            else {
                continue;
            };
            let t = text.trim();
            if t.is_empty()
                || t.starts_with("<command-")
                || t.starts_with("<local-command")
                || t.starts_with("<system-reminder>")
                || t.starts_with("<task-notification>")
            {
                continue;
            }
            out.push(format!(
                "{}: {}",
                if kind == "user" { "USER" } else { "ASSISTANT" },
                truncate(t, 1500)
            ));
        }
        if out.len() >= n || take >= len || take >= max_bytes {
            let start_idx = out.len().saturating_sub(n);
            return Ok(out[start_idx..].to_vec());
        }
        window *= 4;
    }
}
