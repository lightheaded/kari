//! Hand a prompt to a Claude Code session that is running on this machine.
//!
//! Every running Claude Code process listens on a Unix socket for messages
//! from its peers: `messagingSocketPath` in its registry file under
//! `~/.claude/sessions/`, and a key file beside it that holds the token a
//! sender must present. A message that arrives there goes into the session's
//! own prompt queue, so an idle session answers it at once and a busy one
//! takes it after the current turn. That is how one Claude Code session talks
//! to another, and kari uses the same door.
//!
//! The alternative, `claude --bg --resume <id>`, cannot reach a running
//! session: Claude Code refuses to run one session twice and starts a copy
//! instead, so the prompt lands in a second transcript and the terminal the
//! user is looking at never sees it. That was the failure this module ends.
//!
//! The wire format is two lines of JSON on one connection: an auth line with
//! the peer token, then the frame. The format is internal to Claude Code, so
//! every field is read defensively and a refusal is reported, never guessed at.

use crate::paths;
use serde_json::{json, Value};
use std::path::PathBuf;

/// Where a running session takes messages, as its registry file says.
pub struct Inbox {
    pub pid: u32,
    pub socket: String,
    pub token: Option<String>,
}

/// Read the registry file of one process and find its inbox and key.
///
/// The key file is `<pid>.<hash>.key` where the hash is over the socket path.
/// The hash is not recomputed here: a file that starts with the pid and ends in
/// `.key` is the key of that process, and the registry directory holds one per
/// process. Recomputing it would tie kari to a canonicalisation rule that is
/// not ours to know.
pub fn inbox_of(pid: u32) -> anyhow::Result<Inbox> {
    let dir = paths::claude_sessions_dir();
    let reg = std::fs::read_to_string(dir.join(format!("{pid}.json")))
        .map_err(|e| anyhow::anyhow!("no registry file for pid {pid}: {e}"))?;
    let v: Value = serde_json::from_str(&reg)?;
    let socket = v
        .get("messagingSocketPath")
        .and_then(|s| s.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            anyhow::anyhow!("the session (pid {pid}) registered no message socket; its Claude Code may be too old")
        })?
        .to_string();
    let token = key_file(&dir, pid).and_then(|p| {
        let text = std::fs::read_to_string(p).ok()?;
        let v: Value = serde_json::from_str(&text).ok()?;
        v.get("peerToken")
            .and_then(|t| t.as_str())
            .map(|t| t.to_string())
    });
    Ok(Inbox { pid, socket, token })
}

fn key_file(dir: &std::path::Path, pid: u32) -> Option<PathBuf> {
    let prefix = format!("{pid}.");
    let rd = std::fs::read_dir(dir).ok()?;
    let mut found: Vec<PathBuf> = rd
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| is_key_name(n, &prefix))
        })
        .collect();
    found.sort();
    found.pop()
}

/// `<pid>.<64 hex>.key`, and nothing that only looks like it.
fn is_key_name(name: &str, prefix: &str) -> bool {
    let Some(rest) = name.strip_prefix(prefix) else {
        return false;
    };
    let Some(hash) = rest.strip_suffix(".key") else {
        return false;
    };
    hash.len() == 64 && hash.chars().all(|c| c.is_ascii_hexdigit())
}

/// The two lines the socket takes: the auth line, then one `user` frame.
///
/// `session_id` lets the receiver drop a frame meant for a session that no
/// longer owns the pid. `from` names the sender in the receiver's transcript.
/// `priority: next` queues the prompt behind the turn in progress, which is
/// what a person typing into the terminal gets as well.
pub fn frame(token: Option<&str>, session_id: &str, text: &str) -> String {
    let mut out = String::new();
    if let Some(t) = token {
        out.push_str(&json!({ "type": "auth", "token": t }).to_string());
        out.push('\n');
    }
    let msg_id = uuid::Uuid::new_v4().simple().to_string();
    out.push_str(
        &json!({
            "type": "user",
            "message": { "content": text },
            "session_id": session_id,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "msg_id": msg_id,
            "from": "kari",
            "priority": "next",
        })
        .to_string(),
    );
    out.push('\n');
    out
}

/// Deliver `text` to the session `session_id` that runs as `pid`.
#[cfg(unix)]
pub fn send(pid: u32, session_id: &str, text: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::Duration;

    let inbox = inbox_of(pid)?;
    let payload = frame(inbox.token.as_deref(), session_id, text);
    let mut s = UnixStream::connect(&inbox.socket).map_err(|e| {
        anyhow::anyhow!("the session (pid {pid}) does not answer on its message socket: {e}")
    })?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    s.set_read_timeout(Some(Duration::from_millis(400)))?;
    s.write_all(payload.as_bytes())?;
    s.flush()?;
    // Claude Code's own sender keeps the connection open for a moment before it
    // ends it, so the receiver reads the whole frame before the close arrives.
    // A read with a short timeout does the same wait, and picks up a refusal
    // if the receiver writes one.
    let mut buf = [0u8; 512];
    let _ = std::io::Read::read(&mut s, &mut buf);
    let _ = s.shutdown(std::net::Shutdown::Both);
    Ok(())
}

#[cfg(not(unix))]
pub fn send(pid: u32, _session_id: &str, _text: &str) -> anyhow::Result<()> {
    let _ = pid;
    anyhow::bail!("this node cannot hand a prompt to a running session; jump in instead")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_key_name_is_the_pid_and_a_hash() {
        let hash = "a".repeat(64);
        assert!(is_key_name(&format!("1538.{hash}.key"), "1538."));
        // Another pid that starts with the same digits is another process.
        assert!(!is_key_name(&format!("15380.{hash}.key"), "1538."));
        assert!(!is_key_name(&format!("1538.{hash}.key.tmp.ab"), "1538."));
        assert!(!is_key_name("1538.json", "1538."));
        assert!(!is_key_name("1538.zz.key", "1538."));
    }

    #[test]
    fn the_frame_is_an_auth_line_then_a_user_frame() {
        let f = frame(Some("tok"), "sid", "push it");
        let mut lines = f.lines();
        let auth: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(auth["type"], "auth");
        assert_eq!(auth["token"], "tok");
        let user: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(user["type"], "user");
        assert_eq!(user["message"]["content"], "push it");
        assert_eq!(user["session_id"], "sid");
        assert_eq!(user["priority"], "next");
        assert_eq!(user["from"], "kari");
        assert_eq!(user["msg_id"].as_str().unwrap().len(), 32);
        assert!(lines.next().is_none());
        assert!(f.ends_with('\n'));
    }

    /// A hand test against a session that runs on this machine:
    /// `KARI_PEER_PID=<pid> KARI_PEER_SID=<session id> cargo test -p kari-core peer -- --ignored`.
    /// The message arrives in that session as a prompt, so pick one of your own.
    #[test]
    #[ignore]
    fn sends_to_a_running_session() {
        let pid: u32 = std::env::var("KARI_PEER_PID").unwrap().parse().unwrap();
        let sid = std::env::var("KARI_PEER_SID").unwrap();
        send(
            pid,
            &sid,
            "kari test message: reply with the single word ok, and do nothing else.",
        )
        .unwrap();
    }

    #[test]
    fn no_token_means_no_auth_line() {
        let f = frame(None, "sid", "x");
        assert_eq!(f.lines().count(), 1);
    }
}
