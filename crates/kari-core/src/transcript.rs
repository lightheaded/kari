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
    // A prompt another process handed in (kari, or a peer session) is a
    // prompt the session answers, so it counts as a turn like a typed one. It
    // carries isMeta, which every other meta line uses for Claude Code's own
    // notes; only the peer origin tells them apart.
    let origin = v
        .get("origin")
        .and_then(|o| o.get("kind"))
        .and_then(|k| k.as_str());
    let human = match origin {
        Some(k) => k == "human" || k == "peer",
        None => true,
    };
    if !human {
        return;
    }
    if origin != Some("peer") && v.get("isMeta").and_then(|b| b.as_bool()) == Some(true) {
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

/// The drawer shows the last reply whole, so it is kept whole. `truncate` is
/// wrong for it twice over: it clips at a few hundred characters, and it folds
/// newlines into spaces, which flattens the markdown Claude writes.
///
/// The cap is a guard against a runaway reply reaching the board payload, not a
/// display limit — it sits far above any answer a session ends on.
const REPLY_MAX: usize = 8000;

fn reply_text(s: &str) -> String {
    let s = s.trim();
    if s.chars().count() <= REPLY_MAX {
        return s.to_string();
    }
    let mut out: String = s.chars().take(REPLY_MAX - 1).collect();
    out.push('\u{2026}');
    out
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
                            f.last_assistant_text = Some(reply_text(t));
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

/// One line of a transcript that the conversation view or the summary shows.
struct Turn {
    /// `user`, `assistant`, `peer` (a prompt another process sent in), or
    /// `tool` for a git or gh command the assistant ran.
    role: &'static str,
    text: String,
    at: Option<DateTime<Utc>>,
}

/// Noise that Claude Code writes as user lines and a person never typed.
fn is_injected(t: &str) -> bool {
    t.starts_with("<command-")
        || t.starts_with("<local-command")
        || t.starts_with("<system-reminder>")
        || t.starts_with("<task-notification>")
}

/// A Bash command the summary should see: the ones that move the work along.
fn is_delivery_command(cmd: &str) -> bool {
    let c = cmd.trim_start();
    [
        "git ",
        "gh ",
        "scripts/release",
        "cargo publish",
        "npm publish",
    ]
    .iter()
    .any(|p| c.starts_with(p))
        || c.contains("&& git ")
        || c.contains("&& gh ")
        || c.contains("| git ")
}

/// Read one transcript line into the turns it carries. A user line is one turn
/// or nothing; an assistant line can be a text turn and several tool turns.
fn turns_of(v: &Value, with_tools: bool) -> Vec<Turn> {
    let kind = v.get("type").and_then(|t| t.as_str()).unwrap_or("");
    if kind != "user" && kind != "assistant" {
        return vec![];
    }
    if v.get("isSidechain").and_then(|b| b.as_bool()) == Some(true) {
        return vec![];
    }
    let at = ts(v);
    let Some(msg) = v.get("message") else {
        return vec![];
    };
    let mut out = vec![];
    if kind == "user" {
        // A message from a peer process carries isMeta, and still is a prompt
        // the session answered. Every other meta line is Claude Code's own.
        let origin = v
            .get("origin")
            .and_then(|o| o.get("kind"))
            .and_then(|k| k.as_str());
        let peer = origin == Some("peer");
        if v.get("isMeta").and_then(|b| b.as_bool()) == Some(true) && !peer {
            return out;
        }
        if let Some(text) = msg.get("content").and_then(text_of_content) {
            let t = text.trim();
            if !t.is_empty() && !is_injected(t) {
                out.push(Turn {
                    role: if peer { "peer" } else { "user" },
                    text: t.to_string(),
                    at,
                });
            }
        }
        return out;
    }
    if let Some(Value::Array(parts)) = msg.get("content") {
        let mut text = String::new();
        for p in parts {
            match p.get("type").and_then(|t| t.as_str()) {
                Some("text") => {
                    if let Some(t) = p.get("text").and_then(|t| t.as_str()) {
                        if !text.is_empty() {
                            text.push('\n');
                        }
                        text.push_str(t);
                    }
                }
                Some("tool_use")
                    if with_tools && p.get("name").and_then(|n| n.as_str()) == Some("Bash") =>
                {
                    let cmd = p
                        .get("input")
                        .and_then(|i| i.get("command"))
                        .and_then(|c| c.as_str());
                    if let Some(cmd) = cmd.filter(|c| is_delivery_command(c)) {
                        out.push(Turn {
                            role: "tool",
                            text: truncate(cmd, 240),
                            at,
                        });
                    }
                }
                _ => {}
            }
        }
        let t = text.trim();
        if !t.is_empty() {
            out.insert(
                0,
                Turn {
                    role: "assistant",
                    text: t.to_string(),
                    at,
                },
            );
        }
    }
    out
}

/// The last `n` user and assistant text messages, as "USER: …" / "ASSISTANT: …"
/// lines, with a "TOOL: …" line for every git or gh command between them.
/// The tool lines ride along and do not count towards `n`: they are what lets
/// a summary say whether the work is pushed or merged when the reply text
/// does not.
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
        let mut turns: Vec<Turn> = vec![];
        for line in text.lines().skip(if start > 0 { 1 } else { 0 }) {
            let Ok(v) = serde_json::from_str::<Value>(line) else {
                continue;
            };
            turns.extend(turns_of(&v, true));
        }
        let spoken = turns.iter().filter(|t| t.role != "tool").count();
        if spoken >= n || take >= len || take >= max_bytes {
            // Keep the last n spoken turns and every tool line among them.
            let mut skip = spoken.saturating_sub(n);
            let mut out = vec![];
            for t in turns {
                if t.role != "tool" && skip > 0 {
                    skip -= 1;
                    continue;
                }
                if t.role == "tool" && out.is_empty() && skip > 0 {
                    continue;
                }
                let label = match t.role {
                    "assistant" => "ASSISTANT",
                    "tool" => "TOOL",
                    _ => "USER",
                };
                out.push(format!("{label}: {}", truncate(&t.text, 1500)));
            }
            return Ok(out);
        }
        window *= 4;
    }
}

/// How much of one turn the conversation view carries. A reply longer than
/// this is a wall of tool output pasted back, not something to read in a
/// side panel; the tail is dropped and marked.
const TURN_MAX: usize = 12_000;

/// Every spoken turn of the transcript, oldest first, with tool calls left
/// out. `limit` keeps the last that many; `total` says how many there were.
pub fn messages(
    path: &Path,
    session_id: &str,
    limit: usize,
) -> anyhow::Result<crate::model::Conversation> {
    let f = std::fs::File::open(path)?;
    let reader = BufReader::with_capacity(1 << 16, f);
    let mut all: Vec<crate::model::TranscriptMessage> = vec![];
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let Ok(v) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        for t in turns_of(&v, false) {
            let text = if t.text.chars().count() > TURN_MAX {
                let mut s: String = t.text.chars().take(TURN_MAX).collect();
                s.push_str("\n\u{2026} (cut here)");
                s
            } else {
                t.text
            };
            all.push(crate::model::TranscriptMessage {
                role: t.role.to_string(),
                text,
                at: t.at,
            });
        }
    }
    let total = all.len();
    let keep = all.split_off(total.saturating_sub(limit));
    Ok(crate::model::Conversation {
        session_id: session_id.to_string(),
        total,
        messages: keep,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reply_keeps_the_markdown_shape() {
        let md = "# Done\n\n- one\n- two\n\n```sh\ncargo test\n```";
        assert_eq!(reply_text(&format!("  {md}\n\n")), md);
    }

    #[test]
    fn reply_caps_a_runaway_answer() {
        let long = "x".repeat(REPLY_MAX * 2);
        let out = reply_text(&long);
        assert_eq!(out.chars().count(), REPLY_MAX);
        assert!(out.ends_with('\u{2026}'));
    }

    fn write_lines(lines: &[&str]) -> std::path::PathBuf {
        let p =
            std::env::temp_dir().join(format!("kari-transcript-{}.jsonl", uuid::Uuid::new_v4()));
        std::fs::write(&p, lines.join("\n") + "\n").unwrap();
        p
    }

    const USER: &str = r#"{"type":"user","timestamp":"2026-09-09T07:00:00Z","message":{"role":"user","content":"merge it"}}"#;
    const PEER: &str = r#"{"type":"user","isMeta":true,"origin":{"kind":"peer","from":"kari"},"message":{"role":"user","content":"push it"}}"#;
    const META: &str = r#"{"type":"user","isMeta":true,"message":{"role":"user","content":"<system-reminder>x</system-reminder>"}}"#;
    const RESULT: &str = r#"{"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":"ok"}]}}"#;
    const REPLY: &str = r#"{"type":"assistant","timestamp":"2026-09-09T07:01:00Z","message":{"role":"assistant","content":[{"type":"text","text":"Merging."},{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"git merge --ff-only topic && git push"}},{"type":"tool_use","id":"t2","name":"Bash","input":{"command":"ls -la"}}]}}"#;

    #[test]
    fn the_conversation_keeps_prompts_and_replies_and_drops_the_rest() {
        let p = write_lines(&[USER, REPLY, RESULT, META, PEER]);
        let c = messages(&p, "s", 100).unwrap();
        let roles: Vec<&str> = c.messages.iter().map(|m| m.role.as_str()).collect();
        assert_eq!(roles, ["user", "assistant", "peer"]);
        assert_eq!(c.total, 3);
        assert_eq!(c.messages[2].text, "push it");
        assert!(c.messages[0].at.is_some());
    }

    #[test]
    fn the_conversation_limit_keeps_the_tail() {
        let p = write_lines(&[USER, REPLY, RESULT, META, PEER]);
        let c = messages(&p, "s", 1).unwrap();
        assert_eq!(c.total, 3);
        assert_eq!(c.messages.len(), 1);
        assert_eq!(c.messages[0].role, "peer");
    }

    #[test]
    fn the_summary_excerpt_carries_the_git_commands() {
        let p = write_lines(&[USER, REPLY, RESULT]);
        let lines = tail_messages(&p, 30, 1 << 20).unwrap();
        assert_eq!(
            lines,
            [
                "USER: merge it",
                "ASSISTANT: Merging.",
                "TOOL: git merge --ff-only topic && git push"
            ]
        );
    }

    #[test]
    fn the_tool_lines_do_not_use_up_the_message_budget() {
        let p = write_lines(&[USER, REPLY, RESULT, PEER]);
        // Two spoken turns fit; the git line rides along with its reply.
        let lines = tail_messages(&p, 2, 1 << 20).unwrap();
        assert_eq!(
            lines,
            [
                "ASSISTANT: Merging.",
                "TOOL: git merge --ff-only topic && git push",
                "USER: push it"
            ]
        );
    }
}
