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
//!
//! Two gates stand between the frame and the model that answers it.
//!
//! The first gate is the inbox. It classes the sender by permission mode and
//! holds a message that does not match, until the user of that session
//! approves it by hand. The sender states its mode in an envelope around the
//! message text, not in the JSON, so a message with no envelope reaches a
//! session that bypasses prompts only after a hold. kari states the mode it
//! runs the card with, which is the only claim it can make honestly.
//!
//! The second gate is the model. A peer message is not the user's approval,
//! and the receiver is told to refuse a peer that claims otherwise. No sender
//! can pass that gate, and none should.
//!
//! The outcome of the first gate comes back as a receipt, on a new connection
//! to the address the frame names in `from`. A sender with no address of its
//! own therefore cannot learn that its message was held. kari binds a socket
//! beside the socket it sends to and reads the receipt from there.
//!
//! The receipt carries bad news only. A message the inbox accepts gets no
//! receipt, so silence is the answer for a send that went through. The only
//! receipt that reports success is the one for a message that was held first
//! and that the user then released. A sender that waits for a receipt on
//! every send therefore waits for nothing.

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

/// The class the inbox sorts a sender into. A session that bypasses prompts
/// takes messages from another that does the same without a hold, and holds
/// everything else for its user. The two classes are the whole vocabulary:
/// the inbox knows no third state and reads no other word.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModeClass {
    Bypass,
    Prompting,
}

impl ModeClass {
    fn as_str(self) -> &'static str {
        match self {
            ModeClass::Bypass => "bypass",
            ModeClass::Prompting => "prompting",
        }
    }
}

/// The class of a Claude Code permission mode, or `None` when kari cannot
/// tell.
///
/// `bypassPermissions` is the only mode that always bypasses prompts.
/// `plan` bypasses them when the session is allowed to, which is a fact about
/// that session and not about the mode, so kari cannot state it. An unknown
/// word is unknown as well. In both cases kari claims nothing: a message with
/// no claim is held by a session that bypasses prompts, and a false claim
/// would defeat the gate its user depends on.
pub fn mode_class(mode: &str) -> Option<ModeClass> {
    match mode.trim() {
        "bypassPermissions" => Some(ModeClass::Bypass),
        "default" | "acceptEdits" | "auto" | "dontAsk" => Some(ModeClass::Prompting),
        _ => None,
    }
}

/// The tag that wraps a message from a peer.
const TAG: &str = "cross-session-message";

/// Wrap the text so the receiver knows who sent it and under which mode.
///
/// The receiver parses this envelope, then builds it again from what it parsed
/// and drops the whole envelope when the two strings differ. So the order of
/// the attributes is the receiver's order, an attribute it would not write is
/// left out, and the body arrives with its own closing tags already escaped.
pub fn envelope(
    from: Option<&str>,
    from_name: Option<&str>,
    from_mode: Option<ModeClass>,
    body: &str,
) -> String {
    let mut attrs = String::new();
    if let Some(f) = from.filter(|f| !f.is_empty()) {
        attrs.push_str(&format!(" from=\"{f}\""));
    }
    if let Some(n) = from_name.filter(|n| !n.is_empty()) {
        attrs.push_str(&format!(" from-name=\"{n}\""));
    }
    if let Some(m) = from_mode {
        attrs.push_str(&format!(" from-mode=\"{}\"", m.as_str()));
    }
    format!("<{TAG}{attrs}>\n{}\n</{TAG}>", escape_body(body))
}

/// Escape a closing tag inside the body, the way the receiver expects it.
///
/// The receiver skips a `<` that a backslash follows, so one escape too many
/// keeps the envelope exact and only shows one more backslash in the text. One
/// escape too few loses the envelope, and with it the mode, and the tags then
/// show up as text in the prompt. This matches a wider set than the receiver
/// does, never a narrower one.
fn escape_body(body: &str) -> String {
    let mut out = String::with_capacity(body.len());
    let mut rest = body;
    while let Some(at) = rest.find(is_open_angle) {
        out.push_str(&rest[..at]);
        let mut chars = rest[at..].chars();
        let angle = chars.next().unwrap_or('<');
        let after = chars.as_str();
        if !after.starts_with('\\') && starts_closing_tag(after) {
            out.push_str("<\\");
        } else {
            out.push(angle);
        }
        rest = after;
    }
    out.push_str(rest);
    out
}

/// `<`, and the characters that stand in for it.
fn is_open_angle(c: char) -> bool {
    matches!(
        c,
        '<' | '\u{ff1c}'
            | '\u{fe64}'
            | '\u{2329}'
            | '\u{27e8}'
            | '\u{3008}'
            | '\u{2039}'
            | '\u{02c2}'
            | '\u{1438}'
            | '\u{276c}'
            | '\u{276e}'
            | '\u{2770}'
            | '\u{29fc}'
            | '\u{226e}'
            | '\u{227a}'
            | '\u{22d6}'
    )
}

/// `/`, and the characters that stand in for it.
fn is_slash(c: char) -> bool {
    matches!(c, '/' | '\u{ff0f}' | '\u{2215}' | '\u{2044}')
}

/// A character the receiver steps over between the letters of the tag: the
/// invisible ones, the combining marks, and the control codes that are not
/// tab or newline.
fn is_filler(c: char) -> bool {
    let c = c as u32;
    matches!(c,
        0x00..=0x08 | 0x0b | 0x0c | 0x0e..=0x1f | 0x7f..=0x9f
        | 0x00ad | 0x034f | 0x0600..=0x0605 | 0x061c | 0x06dd | 0x070f
        | 0x0890 | 0x0891 | 0x08e2 | 0x115f | 0x1160 | 0x17b4 | 0x17b5
        | 0x180b..=0x180f | 0x200b..=0x200f | 0x202a..=0x202e
        | 0x2060..=0x206f | 0x2028 | 0x2029 | 0x3164
        | 0xfe00..=0xfe0f | 0xfeff | 0xffa0 | 0xfff0..=0xfffb
        | 0x110bd | 0x110cd | 0x13430..=0x1343f | 0x1bca0..=0x1bca3
        | 0x1d173..=0x1d17a | 0x16fe4 | 0xe0000..=0xe0fff
        | 0x0300..=0x0344 | 0x0346..=0x036f | 0x0483..=0x0489
        | 0x0591..=0x05bd | 0x05bf | 0x05c1 | 0x05c2 | 0x05c4 | 0x05c5
        | 0x05c7 | 0x0610..=0x061a | 0x064b..=0x065f | 0x0670
        | 0x06d6..=0x06dc | 0x06df..=0x06e4 | 0x06e7 | 0x06e8
        | 0x06ea..=0x06ed | 0x1ab0..=0x1aff | 0x1dc0..=0x1dff
        | 0x20d0..=0x20ff | 0x3099 | 0x309a | 0xfe20..=0xfe2f)
}

/// Does the text right after an open angle close the tag?
fn starts_closing_tag(after: &str) -> bool {
    let mut chars = after.chars().peekable();
    let mut slash = false;
    while let Some(&c) = chars.peek() {
        if is_slash(c) {
            slash = true;
        } else if !is_filler(c) {
            break;
        }
        chars.next();
    }
    if !slash {
        return false;
    }
    for want in TAG.chars() {
        while chars.peek().is_some_and(|&c| is_filler(c)) {
            chars.next();
        }
        match chars.next() {
            Some(c) if c.to_ascii_lowercase() == want => {}
            _ => return false,
        }
    }
    // The receiver stops the name at a character that cannot belong to it.
    !chars
        .next()
        .is_some_and(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

/// The address of a socket, as the receiver writes it.
///
/// Everything outside the set the receiver accepts is percent-encoded, byte by
/// byte, in upper case. A path under `/tmp` needs none of it, and a path that
/// does need it stays readable to the receiver's own parser.
#[cfg(unix)]
fn uds_address(socket: &str) -> String {
    let mut out = String::from("uds:");
    for b in socket.bytes() {
        let c = b as char;
        if c.is_ascii_alphanumeric() || matches!(c, ':' | '_' | '/' | '.' | '\\' | '-') {
            out.push(c);
        } else {
            out.push_str(&format!("%{b:02X}"));
        }
    }
    out
}

/// What the first gate did with the message.
///
/// The receipt is the channel for bad news only. A message the inbox accepts
/// gets no receipt at all: the accept branch of the gate counts the message
/// and returns, and it writes nothing back. So silence is the answer for a
/// good send, and `Delivered` belongs to the one case that reports success —
/// a message that was held first and that the user then released.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The inbox took the message and wrote no receipt, which is what it does
    /// for every message it accepts. The prompt is in that session's queue.
    Accepted,
    /// The message waits for the user of that session to approve it by hand.
    Held(Option<String>),
    /// A message that was held, and that the user then approved.
    Delivered,
    /// The user of that session declined the message.
    Denied(Option<String>),
    /// The hold ended without an answer.
    Expired(Option<String>),
    /// A setting or a policy in that session refuses peer messages.
    Refused(Option<String>),
    /// The inbox threw the message away: a rate limit, a duplicate, a loop, or
    /// a full queue.
    Dropped(Option<String>),
}

impl Outcome {
    /// One line for the person who pressed send.
    pub fn line(&self, pid: u32) -> String {
        match self {
            Outcome::Accepted => format!("Sent to the running session (pid {pid})"),
            Outcome::Delivered => format!("Delivered to the running session (pid {pid})"),
            Outcome::Held(_) => format!(
                "Held for approval in that session (pid {pid}). Its permission mode does not match, so its user has to release the message there."
            ),
            Outcome::Denied(_) => {
                format!("The user of that session declined the message (pid {pid})")
            }
            Outcome::Expired(_) => {
                format!("The message waited for approval in that session and expired (pid {pid})")
            }
            Outcome::Refused(_) => format!(
                "That session does not take peer messages (pid {pid}). Its crossSessionInbound setting refuses them."
            ),
            Outcome::Dropped(_) => {
                format!("That session's inbox dropped the message (pid {pid})")
            }
        }
    }

    /// The word for the run log.
    pub fn log_word(&self) -> &'static str {
        match self {
            Outcome::Accepted => "sent to the running session",
            Outcome::Delivered => "delivered to the running session",
            Outcome::Held(_) => "held for approval in that session",
            Outcome::Denied(_) => "declined by the user of that session",
            Outcome::Expired(_) => "expired while it waited for approval",
            Outcome::Refused(_) => "refused by that session",
            Outcome::Dropped(_) => "dropped by that session's inbox",
        }
    }

    /// The sentence the receiver sent with the receipt, when it sent one.
    pub fn detail(&self) -> Option<&str> {
        match self {
            Outcome::Accepted | Outcome::Delivered => None,
            Outcome::Held(d)
            | Outcome::Denied(d)
            | Outcome::Expired(d)
            | Outcome::Refused(d)
            | Outcome::Dropped(d) => d.as_deref(),
        }
    }

    /// Did the message reach the session's prompt queue?
    pub fn reached_the_queue(&self) -> bool {
        matches!(self, Outcome::Accepted | Outcome::Delivered)
    }
}

/// Read one receipt frame. `None` when the frame is not a receipt for `msg_id`.
#[cfg(unix)]
fn read_receipt(line: &str, msg_id: &str) -> Option<Outcome> {
    let v: Value = serde_json::from_str(line).ok()?;
    if v.get("type").and_then(|t| t.as_str()) != Some("control") {
        return None;
    }
    if v.get("action").and_then(|a| a.as_str()) != Some("peer_message_status") {
        return None;
    }
    let mine = v.get("orig_msg_id").and_then(|m| m.as_str()) == Some(msg_id)
        || v.get("dropped_msg_ids")
            .and_then(|d| d.as_array())
            .is_some_and(|ids| ids.iter().any(|i| i.as_str() == Some(msg_id)));
    if !mine {
        return None;
    }
    let reason = v
        .get("reason")
        .and_then(|r| r.as_str())
        .map(|r| r.to_string());
    let status = v.get("status").and_then(|s| s.as_str())?;
    // A refusal by the model itself arrives as an expired hold that says so.
    let refused = v.get("status_detail").and_then(|d| d.as_str()) == Some("refused");
    Some(match status {
        "held" => Outcome::Held(reason),
        "delivered" => Outcome::Delivered,
        "denied" => Outcome::Denied(reason),
        "expired" if refused => Outcome::Refused(reason),
        "expired" => Outcome::Expired(reason),
        "refused" => Outcome::Refused(reason),
        "dropped" => Outcome::Dropped(reason),
        _ => return None,
    })
}

/// The name kari sends itself under, so a held message names a tool on a
/// machine and not "an unidentified session".
///
/// The receiver strips the format and control characters, trims the result and
/// cuts it at 64 characters, then builds the attribute again and compares. So
/// the name arrives here already clean, or the whole envelope is lost.
#[cfg(unix)]
fn sender_name() -> String {
    let host: String = paths::hostname()
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '"' | '<' | '>'))
        .collect();
    let host = host.trim();
    let name = if host.is_empty() {
        "kari".to_string()
    } else {
        format!("kari on {host}")
    };
    name.chars().take(64).collect::<String>().trim().to_string()
}

/// The id that a receipt names to say which message it answers.
///
/// The receiver keeps the id only when it reads as a dashed UUID, and drops
/// anything else without a word. An id it dropped leaves the receipt with
/// nothing to match, so the shape is not a detail.
#[cfg(unix)]
fn new_msg_id() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// The two lines the socket takes: the auth line, then one `user` frame.
///
/// `session_id` lets the receiver drop a frame meant for a session that no
/// longer owns the pid. `from` is the address the receipt goes to, and names
/// the sender in the receiver's transcript. `priority: next` queues the prompt
/// behind the turn in progress, which is what a person typing into the
/// terminal gets as well. `msgV` is the version of the frame, as Claude Code's
/// own sender stamps it.
pub fn frame(
    token: Option<&str>,
    session_id: &str,
    content: &str,
    from: &str,
    msg_id: &str,
) -> String {
    let mut out = String::new();
    if let Some(t) = token {
        out.push_str(&json!({ "type": "auth", "token": t }).to_string());
        out.push('\n');
    }
    out.push_str(
        &json!({
            "type": "user",
            "message": { "content": content },
            "session_id": session_id,
            "uuid": uuid::Uuid::new_v4().to_string(),
            "msgV": 1,
            "msg_id": msg_id,
            "from": from,
            "priority": "next",
        })
        .to_string(),
    );
    out.push('\n');
    out
}

/// A socket of our own, so a receipt has somewhere to arrive.
#[cfg(unix)]
struct ReceiptInbox {
    listener: std::os::unix::net::UnixListener,
    path: PathBuf,
    address: String,
}

#[cfg(unix)]
impl Drop for ReceiptInbox {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

/// Bind a socket beside the socket we send to.
///
/// The receiver refuses a reply address outside the directory of its own
/// socket, so the inbox has to sit in that directory. The name carries a
/// random tail, which keeps it clear of the `<pid>.sock` name a session of our
/// own pid would hold, and which the receiver's own name check accepts.
/// A failure here costs the receipt and nothing else, so it is not an error.
#[cfg(unix)]
fn bind_receipt_inbox(their_socket: &str) -> Option<ReceiptInbox> {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    let dir = std::path::Path::new(their_socket).parent()?;
    let pid = std::process::id();
    sweep_stale_inboxes(dir, pid);
    let tail = uuid::Uuid::new_v4().simple().to_string();
    let path = dir.join(format!("{pid}-{}.sock", &tail[..8]));
    let listener = UnixListener::bind(&path).ok()?;
    listener.set_nonblocking(true).ok()?;
    // Only this machine's own user has business here.
    let _ = std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600));
    let address = uds_address(path.to_str()?);
    Some(ReceiptInbox {
        listener,
        path,
        address,
    })
}

/// Clear an inbox that an earlier run of this pid left behind. A socket that
/// still answers belongs to a live process, so it stays.
#[cfg(unix)]
fn sweep_stale_inboxes(dir: &std::path::Path, pid: u32) {
    use std::os::unix::net::UnixStream;

    let prefix = format!("{pid}-");
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in rd.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let Some(tail) = name
            .strip_prefix(&prefix)
            .and_then(|t| t.strip_suffix(".sock"))
        else {
            continue;
        };
        if tail.len() != 8 || !tail.chars().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        if UnixStream::connect(&path).is_ok() {
            continue;
        }
        let _ = std::fs::remove_file(&path);
    }
}

/// Is the process on the other end of this connection the session we wrote to?
///
/// The receipt carries no token, so the frame alone proves nothing. The
/// kernel's own answer does: the peer credentials of the connection name the
/// process and the user, and a frame from anything else is thrown away.
#[cfg(any(target_os = "macos", target_os = "ios"))]
fn peer_is(stream: &std::os::unix::net::UnixStream, pid: u32, uid: u32) -> bool {
    use std::os::unix::io::AsRawFd;
    let fd = stream.as_raw_fd();
    let mut peer_pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let got_pid = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERPID,
            &mut peer_pid as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    let mut cred: libc::xucred = unsafe { std::mem::zeroed() };
    let mut clen = std::mem::size_of::<libc::xucred>() as libc::socklen_t;
    let got_uid = unsafe {
        libc::getsockopt(
            fd,
            libc::SOL_LOCAL,
            libc::LOCAL_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut clen,
        )
    };
    got_pid == 0 && got_uid == 0 && peer_pid as u32 == pid && cred.cr_uid == uid
}

/// What `SO_PEERCRED` answers with. The `libc` crate carries this struct for
/// Linux only, and the phone needs it as well, so kari names the three fields
/// itself. The order and the widths are the kernel's, and both platforms
/// share them.
#[cfg(any(target_os = "linux", target_os = "android"))]
#[repr(C)]
struct Ucred {
    pid: libc::pid_t,
    uid: libc::uid_t,
    gid: libc::gid_t,
}

#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_is(stream: &std::os::unix::net::UnixStream, pid: u32, uid: u32) -> bool {
    use std::os::unix::io::AsRawFd;
    let mut cred = Ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<Ucred>() as libc::socklen_t;
    let got = unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            &mut cred as *mut _ as *mut libc::c_void,
            &mut len,
        )
    };
    got == 0 && cred.pid as u32 == pid && cred.uid == uid
}

/// A platform whose peer credentials kari cannot read keeps the receipt out.
/// The send is then reported as accepted, which is what it is: the frame went
/// out, and a receipt kari cannot trust says nothing.
#[cfg(all(
    unix,
    not(any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    ))
))]
fn peer_is(_stream: &std::os::unix::net::UnixStream, _pid: u32, _uid: u32) -> bool {
    false
}

/// Wait for the receipt of `msg_id`, until `deadline`.
#[cfg(unix)]
fn wait_for_receipt(
    inbox: &ReceiptInbox,
    msg_id: &str,
    pid: u32,
    deadline: std::time::Instant,
) -> Outcome {
    use std::io::Read;
    use std::time::{Duration, Instant};

    let uid = unsafe { libc::getuid() };
    while Instant::now() < deadline {
        match inbox.listener.accept() {
            Ok((stream, _)) => {
                if !peer_is(&stream, pid, uid) {
                    continue;
                }
                let _ = stream.set_nonblocking(false);
                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                let mut text = String::new();
                let _ = stream.take(64 * 1024).read_to_string(&mut text);
                for line in text.lines().filter(|l| !l.trim().is_empty()) {
                    if let Some(outcome) = read_receipt(line, msg_id) {
                        return outcome;
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(_) => break,
        }
    }
    Outcome::Accepted
}

/// How long to wait for a receipt. The inbox writes one as it decides, so the
/// answer is there in a few milliseconds or it is not coming.
#[cfg(unix)]
const RECEIPT_WAIT_MS: u64 = 750;

/// Deliver `text` to the session `session_id` that runs as `pid`.
///
/// `from_mode` is the class kari runs this card under. `None` claims nothing,
/// which is what kari does when it cannot tell.
#[cfg(unix)]
pub fn send(
    pid: u32,
    session_id: &str,
    text: &str,
    from_mode: Option<ModeClass>,
) -> anyhow::Result<Outcome> {
    use std::io::Write;
    use std::os::unix::net::UnixStream;
    use std::time::{Duration, Instant};

    let inbox = inbox_of(pid)?;
    // Bind before the send: the receipt can arrive while we still write.
    let receipts = bind_receipt_inbox(&inbox.socket);
    let address = receipts.as_ref().map(|r| r.address.clone());
    let msg_id = new_msg_id();
    let name = sender_name();
    let content = envelope(address.as_deref(), Some(&name), from_mode, text);
    let payload = frame(
        inbox.token.as_deref(),
        session_id,
        &content,
        address.as_deref().unwrap_or("kari"),
        &msg_id,
    );
    let mut s = UnixStream::connect(&inbox.socket).map_err(|e| {
        anyhow::anyhow!("the session (pid {pid}) does not answer on its message socket: {e}")
    })?;
    s.set_write_timeout(Some(Duration::from_secs(5)))?;
    s.set_read_timeout(Some(Duration::from_millis(400)))?;
    s.write_all(payload.as_bytes())?;
    s.flush()?;
    // Claude Code's own sender keeps the connection open for a moment before it
    // ends it, so the receiver reads the whole frame before the close arrives.
    // A read with a short timeout does the same wait.
    let mut buf = [0u8; 512];
    let _ = std::io::Read::read(&mut s, &mut buf);
    let _ = s.shutdown(std::net::Shutdown::Both);
    let Some(receipts) = receipts else {
        return Ok(Outcome::Accepted);
    };
    let deadline = Instant::now() + Duration::from_millis(RECEIPT_WAIT_MS);
    Ok(wait_for_receipt(&receipts, &msg_id, pid, deadline))
}

#[cfg(not(unix))]
pub fn send(
    pid: u32,
    _session_id: &str,
    _text: &str,
    _from_mode: Option<ModeClass>,
) -> anyhow::Result<Outcome> {
    let _ = pid;
    anyhow::bail!("this node cannot hand a prompt to a running session; jump in instead")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The receiver's own parser, as its regular expression writes it. An
    /// envelope that this does not read is an envelope the receiver drops.
    fn parser() -> regex::Regex {
        let addr = r"[A-Za-z0-9%:_/.\\\-]+";
        let session = r"[A-Za-z0-9_\-]{1,80}";
        let hop = r"[0-9a-f]{24}(?:,[0-9a-f]{24}){0,31}";
        regex::Regex::new(&format!(
            "^(?s)<{TAG}(?: from=\"({addr})\")?(?: from-session=\"({session})\")?(?: hop-chain=\"({hop})\")?(?: from-name=\"([^\"<>\\n\\r]+)\")?(?: from-mode=\"(bypass|prompting)\")?>\\n([\\s\\S]*)\\n</{TAG}>$"
        ))
        .unwrap()
    }

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
        let f = frame(
            Some("tok"),
            "sid",
            "push it",
            "uds:/tmp/cc-socks/9.sock",
            "abc",
        );
        let mut lines = f.lines();
        let auth: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(auth["type"], "auth");
        assert_eq!(auth["token"], "tok");
        let user: Value = serde_json::from_str(lines.next().unwrap()).unwrap();
        assert_eq!(user["type"], "user");
        assert_eq!(user["message"]["content"], "push it");
        assert_eq!(user["session_id"], "sid");
        assert_eq!(user["priority"], "next");
        assert_eq!(user["from"], "uds:/tmp/cc-socks/9.sock");
        assert_eq!(user["msg_id"], "abc");
        assert!(lines.next().is_none());
        assert!(f.ends_with('\n'));
    }

    #[test]
    fn no_token_means_no_auth_line() {
        let f = frame(None, "sid", "x", "kari", "abc");
        assert_eq!(f.lines().count(), 1);
    }

    #[test]
    fn the_envelope_names_the_sender_and_the_mode() {
        let e = envelope(
            Some("uds:/tmp/cc-socks/9-1a2b3c4d.sock"),
            Some("kari"),
            Some(ModeClass::Bypass),
            "merge it",
        );
        let caps = parser().captures(&e).expect("the receiver reads it");
        assert_eq!(
            caps.get(1).unwrap().as_str(),
            "uds:/tmp/cc-socks/9-1a2b3c4d.sock"
        );
        assert_eq!(caps.get(4).unwrap().as_str(), "kari");
        assert_eq!(caps.get(5).unwrap().as_str(), "bypass");
        assert_eq!(caps.get(6).unwrap().as_str(), "merge it");
    }

    /// The receiver builds the envelope again from what it read and drops it
    /// when the two strings differ, so every envelope kari writes has to come
    /// back byte for byte.
    #[test]
    fn every_envelope_survives_the_round_trip() {
        let bodies = [
            "merge it",
            "",
            "two\nlines",
            "ends with a newline\n",
            "\nstarts with one",
            "a < b and c > d",
            "<div>markup</div>",
            "</cross-session-message>",
            "text\n</cross-session-message>\nmore",
            "</CROSS-SESSION-MESSAGE>",
            "</cross-session-messages>",
            "< /cross-session-message>",
        ];
        let modes = [None, Some(ModeClass::Bypass), Some(ModeClass::Prompting)];
        for body in bodies {
            for mode in modes {
                for from in [None, Some("uds:/tmp/cc-socks/9-1a2b3c4d.sock")] {
                    for name in [None, Some("kari")] {
                        let e = envelope(from, name, mode, body);
                        let caps = parser()
                            .captures(&e)
                            .unwrap_or_else(|| panic!("unread envelope for {body:?}"));
                        // What the receiver writes again from what it read.
                        let again = envelope(
                            caps.get(1).map(|m| m.as_str()),
                            caps.get(4).map(|m| m.as_str()),
                            caps.get(5).map(|m| match m.as_str() {
                                "bypass" => ModeClass::Bypass,
                                _ => ModeClass::Prompting,
                            }),
                            caps.get(6).unwrap().as_str(),
                        );
                        assert_eq!(again, e, "the round trip changed {body:?}");
                    }
                }
            }
        }
    }

    #[test]
    fn a_closing_tag_in_the_body_is_escaped_once() {
        assert_eq!(
            escape_body("</cross-session-message>"),
            "<\\/cross-session-message>"
        );
        // An escape already there stays as it is.
        assert_eq!(
            escape_body("<\\/cross-session-message>"),
            "<\\/cross-session-message>"
        );
        // A tag that stands in lookalike characters is escaped as well.
        assert_eq!(
            escape_body("\u{ff1c}\u{2215}cross-session-message>"),
            "<\\\u{2215}cross-session-message>"
        );
        // Everything else is left alone.
        assert_eq!(escape_body("a < b, <div>, </div>"), "a < b, <div>, </div>");
        assert_eq!(
            escape_body("<cross-session-message>"),
            "<cross-session-message>"
        );
    }

    #[test]
    fn only_bypass_permissions_is_the_bypass_class() {
        assert_eq!(mode_class("bypassPermissions"), Some(ModeClass::Bypass));
        for m in ["default", "acceptEdits", "auto", "dontAsk"] {
            assert_eq!(mode_class(m), Some(ModeClass::Prompting), "{m}");
        }
        // `plan` bypasses prompts only when that session is allowed to, which
        // is not ours to claim, and an unknown word is unknown.
        assert_eq!(mode_class("plan"), None);
        assert_eq!(mode_class(""), None);
        assert_eq!(mode_class("bypass"), None);
    }

    #[cfg(unix)]
    #[test]
    fn an_address_keeps_the_characters_the_receiver_accepts() {
        assert_eq!(
            uds_address("/tmp/cc-socks/9-1a2b3c4d.sock"),
            "uds:/tmp/cc-socks/9-1a2b3c4d.sock"
        );
        assert_eq!(uds_address("/tmp/a b.sock"), "uds:/tmp/a%20b.sock");
    }

    #[cfg(unix)]
    #[test]
    fn a_receipt_says_what_the_inbox_did() {
        let held = r#"{"type":"control","action":"peer_message_status","status":"held","reason":"waiting","orig_msg_id":"m1","from":"uds:/tmp/cc-socks/1.sock"}"#;
        assert_eq!(
            read_receipt(held, "m1"),
            Some(Outcome::Held(Some("waiting".into())))
        );
        // Another message's receipt is not ours.
        assert_eq!(read_receipt(held, "m2"), None);
        let refused = r#"{"type":"control","action":"peer_message_status","status":"expired","status_detail":"refused","orig_msg_id":"m1"}"#;
        assert_eq!(read_receipt(refused, "m1"), Some(Outcome::Refused(None)));
        let expired = r#"{"type":"control","action":"peer_message_status","status":"expired","orig_msg_id":"m1"}"#;
        assert_eq!(read_receipt(expired, "m1"), Some(Outcome::Expired(None)));
        let dropped = r#"{"type":"control","action":"peer_message_status","status":"dropped","drop_reason":"queue-full","dropped_msg_ids":["m1"]}"#;
        assert_eq!(read_receipt(dropped, "m1"), Some(Outcome::Dropped(None)));
        // A frame that is not a receipt says nothing.
        assert_eq!(read_receipt(r#"{"type":"user"}"#, "m1"), None);
        assert_eq!(read_receipt("not json", "m1"), None);
    }

    /// The receiver keeps the id only when it reads as a dashed UUID. An id of
    /// another shape is dropped, and every receipt then arrives with nothing
    /// to match, which is how kari used to lose them all.
    #[cfg(unix)]
    #[test]
    fn the_message_id_is_a_dashed_uuid() {
        let shape = regex::Regex::new(
            r"^(?i)[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$",
        )
        .unwrap();
        let id = new_msg_id();
        assert!(shape.is_match(&id), "{id} is not an id the receiver keeps");
        let f = frame(None, "sid", "x", "kari", &id);
        let user: Value = serde_json::from_str(f.lines().next().unwrap()).unwrap();
        assert_eq!(user["msg_id"], id);
        assert_eq!(user["msgV"], 1);
    }

    /// The name has to survive the receiver's own cleaning, or the envelope is
    /// dropped with the mode inside it.
    #[cfg(unix)]
    #[test]
    fn the_sender_name_says_kari_and_the_machine() {
        let name = sender_name();
        assert!(name.starts_with("kari"), "{name}");
        assert_eq!(name.trim(), name);
        assert!(name.chars().count() <= 64, "{name}");
        assert!(
            !name.chars().any(|c| c.is_control() || "\"<>".contains(c)),
            "{name}"
        );
        // The receiver reads it back out of a whole envelope.
        let e = envelope(None, Some(&name), Some(ModeClass::Bypass), "hello");
        let caps = parser().captures(&e).expect("the receiver reads it");
        assert_eq!(caps.get(4).unwrap().as_str(), name);
    }

    /// The inbox writes no receipt for a message it takes, so an inbox that
    /// stays quiet is the good outcome and not a lost answer.
    #[cfg(unix)]
    #[test]
    fn silence_from_the_inbox_means_accepted() {
        use std::time::{Duration, Instant};

        // A socket path has 104 bytes on macOS, and the per-user temporary
        // directory alone is longer than that, so the test builds a short one.
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let dir = std::path::Path::new("/tmp").join(format!("kari-peer-{}", &tag[..8]));
        std::fs::create_dir_all(&dir).unwrap();
        let their_socket = dir.join("1.sock");
        let inbox =
            bind_receipt_inbox(their_socket.to_str().unwrap()).expect("an inbox of our own");
        assert!(inbox.address.starts_with("uds:"));
        assert!(inbox.path.exists());
        let deadline = Instant::now() + Duration::from_millis(30);
        assert_eq!(
            wait_for_receipt(
                &inbox,
                "a-message-nobody-answers",
                std::process::id(),
                deadline
            ),
            Outcome::Accepted
        );
        let path = inbox.path.clone();
        drop(inbox);
        // The inbox takes its socket with it.
        assert!(!path.exists());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn only_a_message_in_the_queue_counts_as_arrived() {
        assert!(Outcome::Accepted.reached_the_queue());
        assert!(Outcome::Delivered.reached_the_queue());
        assert!(!Outcome::Held(None).reached_the_queue());
        assert!(!Outcome::Refused(None).reached_the_queue());
        assert!(!Outcome::Denied(None).reached_the_queue());
    }

    /// A hand test against a session that runs on this machine:
    /// `KARI_PEER_PID=<pid> KARI_PEER_SID=<session id> cargo test -p kari-core peer -- --ignored`.
    /// The message arrives in that session as a prompt, so pick one of your own.
    /// `KARI_PEER_MODE` names the mode to claim, `bypassPermissions` by default.
    #[test]
    #[ignore]
    fn sends_to_a_running_session() {
        let pid: u32 = std::env::var("KARI_PEER_PID").unwrap().parse().unwrap();
        let sid = std::env::var("KARI_PEER_SID").unwrap();
        let mode = std::env::var("KARI_PEER_MODE").unwrap_or("bypassPermissions".into());
        // The inbox drops a duplicate, so every run carries its own text.
        let tag = uuid::Uuid::new_v4().simple().to_string();
        let outcome = send(
            pid,
            &sid,
            &format!(
                "kari test message {tag}: reply with the single word ok, and do nothing else."
            ),
            mode_class(&mode),
        )
        .unwrap();
        println!("outcome: {outcome:?} -- {}", outcome.line(pid));
    }
}
