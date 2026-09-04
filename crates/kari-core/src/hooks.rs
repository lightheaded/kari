//! Claude Code hooks: parse incoming events, fold them into per-session state,
//! and install or remove the hook entries in `~/.claude/settings.json`.
//!
//! Claude Code runs a relay on each event. The relay posts the JSON payload to
//! kari on 127.0.0.1 and always exits 0, so a stopped kari never disturbs a
//! session. On Unix the relay is a small `sh` script kari writes; on Windows,
//! where there is neither `sh` nor a `curl` to count on, it is the node binary
//! itself under `kari-node hooks relay`.

use crate::model::{HookEvent, HookState};
use crate::paths;
use chrono::Utc;
use serde_json::{json, Value};
use std::path::PathBuf;

pub const HOOK_PATH: &str = "/kari/hook";
/// The header that carries the shared secret. The relay script reads the
/// secret from the token file at run time, so a rotated token needs no
/// reinstall.
pub const TOKEN_HEADER: &str = "x-kari-token";
/// The header a hub sends with its id. A column push needs the id of the hub
/// that holds the node's lease.
pub const HUB_HEADER: &str = "x-kari-hub";
/// The subcommand the node binary takes to act as the hook relay on Windows.
const RELAY_SUBCOMMAND: &str = "hooks relay";

/// How an entry in settings.json is recognised as kari's. Unix registers the
/// relay script by path; Windows registers the node binary with a subcommand,
/// so both shapes have to be matched. `uninstall` strips everything this
/// returns true for, so the name must appear as well: a foreign hook that
/// happens to mention the subcommand stays where it is.
fn is_kari_command(c: &str) -> bool {
    c.contains("kari") && (c.contains("hook.sh") || c.contains(RELAY_SUBCOMMAND))
}

/// Post one hook payload to the node on this host, the way the `sh` relay does.
///
/// Never returns an error to the caller: Claude Code runs this on every event,
/// and a stopped node must not disturb a session. The return value is what to
/// print on stdout, which is empty except for a held permission that kari
/// decided.
pub fn relay(payload: &str, port: u16) -> String {
    let held = is_held_event(payload);
    let timeout = if held { HELD_TIMEOUT_SECS } else { 3 };
    let token = std::fs::read_to_string(paths::hook_token_file())
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    let client = match reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .build()
    {
        Ok(c) => c,
        Err(_) => return String::new(),
    };
    let res = client
        .post(format!("http://127.0.0.1:{port}{HOOK_PATH}"))
        .header("content-type", "application/json")
        .header(TOKEN_HEADER, token)
        .body(payload.to_string())
        .send();
    // Only a held event has an answer worth printing. Anything else, including
    // every failure, leaves Claude Code to carry on as if kari were not there.
    match res {
        Ok(r) if held => r.text().unwrap_or_default(),
        _ => String::new(),
    }
}

/// True when this payload is the event kari may hold for a remote answer. Read
/// from the text rather than from a parsed value, so a payload kari cannot
/// parse still reaches the node with the right timeout.
fn is_held_event(payload: &str) -> bool {
    serde_json::from_str::<Value>(payload)
        .ok()
        .and_then(|v| {
            v.get("hook_event_name")
                .and_then(|e| e.as_str())
                .map(|e| e == HELD_EVENT)
        })
        .unwrap_or_else(|| payload.contains(&format!("\"{HELD_EVENT}\"")))
}

/// Events kari registers. PostToolUse clears a pending permission, so it needs every tool.
const EVENTS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("SessionEnd", None),
    ("UserPromptSubmit", None),
    ("Stop", None),
    ("Notification", None),
    ("PreToolUse", Some("AskUserQuestion|ExitPlanMode")),
    ("PostToolUse", Some("*")),
    ("PermissionRequest", None),
];

/// The hook Claude Code runs before a permission dialog. kari can hold it for
/// a remote answer (Away mode), so its timeout is long. The relay prints
/// kari's decision, or nothing, and Claude Code shows the dialog then.
pub const HELD_EVENT: &str = "PermissionRequest";
/// Seconds the relay waits on a held event. Above the longest hold a node offers.
pub const HELD_TIMEOUT_SECS: u64 = 660;

/// The stdout of the relay that settles a permission prompt.
pub fn decision_json(behavior: &str) -> serde_json::Value {
    json!({
        "hookSpecificOutput": {
            "hookEventName": HELD_EVENT,
            "decision": { "behavior": behavior }
        }
    })
}

pub fn parse(v: &Value) -> Option<HookEvent> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(|x| x.to_string());
    let session_id = s("session_id")?;
    let event = s("hook_event_name")?;
    Some(HookEvent {
        at: Utc::now(),
        session_id,
        event,
        cwd: s("cwd"),
        transcript_path: s("transcript_path"),
        notification_type: s("notification_type"),
        message: s("message"),
        tool_name: s("tool_name"),
        permission_mode: s("permission_mode"),
    })
}

/// A Claude Code session id: 8 to 64 characters from `[A-Za-z0-9_-]`.
/// The id reaches shell commands later, so anything else is refused at the door.
pub fn valid_session_id(s: &str) -> bool {
    (8..=64).contains(&s.len())
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// Fold one event into the session's hook state.
pub fn apply(st: &mut HookState, e: &HookEvent) {
    st.last_event = Some(e.event.clone());
    st.last_at = Some(e.at);
    st.events_seen += 1;
    match e.event.as_str() {
        "SessionStart" => {
            st.started_at = Some(e.at);
            st.ended_at = None;
            st.permission_pending_since = None;
            st.permission_message = None;
            st.turn_active = false;
        }
        "SessionEnd" => {
            st.ended_at = Some(e.at);
            st.permission_pending_since = None;
            st.turn_active = false;
        }
        "UserPromptSubmit" => {
            st.turn_active = true;
            st.idle_since = None;
            st.permission_pending_since = None;
            st.permission_message = None;
        }
        "Stop" => {
            st.turn_active = false;
            st.idle_since = Some(e.at);
            st.permission_pending_since = None;
            st.permission_message = None;
        }
        "PostToolUse" => {
            // The tool ran, so any permission prompt was answered.
            st.permission_pending_since = None;
            st.permission_message = None;
            st.turn_active = true;
        }
        "PreToolUse" => {
            st.turn_active = true;
        }
        // Claude Code asks for permission. The hook fires before the dialog.
        "PermissionRequest" => {
            st.permission_pending_since = Some(e.at);
            st.permission_message = e.tool_name.clone();
        }
        "Notification" => match e.notification_type.as_deref().unwrap_or("") {
            "permission_prompt" | "agent_needs_input" => {
                st.permission_pending_since = Some(e.at);
                st.permission_message = e.message.clone();
            }
            "idle_prompt" => {
                st.idle_since = Some(e.at);
                st.turn_active = false;
            }
            _ => {}
        },
        _ => {}
    }
}

pub fn script_path() -> PathBuf {
    paths::kari_dir().join("hook.sh")
}

/// The command line Claude Code runs on each event, and the side effect of
/// preparing it.
///
/// Unix writes a small `sh` script and names it. Windows has no `sh` and no
/// `curl` it can count on, so it names the node binary itself with a
/// subcommand: one process, no shell, and nothing to keep executable.
pub fn relay_command(port: u16) -> anyhow::Result<String> {
    if cfg!(windows) {
        let exe = std::env::current_exe()?;
        return Ok(format!(
            "\"{}\" {RELAY_SUBCOMMAND} --port {port}",
            exe.display()
        ));
    }
    let sp = script_path();
    std::fs::create_dir_all(paths::kari_dir())?;
    std::fs::write(&sp, script(port))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sp, std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(sp.to_string_lossy().into_owned())
}

pub fn script(port: u16) -> String {
    let token_file = paths::hook_token_file().to_string_lossy().into_owned();
    let url = format!("http://127.0.0.1:{port}{HOOK_PATH}");
    let hdr = format!("-H 'content-type: application/json' -H \"{TOKEN_HEADER}: $tok\"");
    format!(
        "#!/bin/sh\n\
         # kari hook relay. Posts the Claude Code hook payload to kari and never fails.\n\
         # A {HELD_EVENT} may be held by kari for a remote answer; its decision is printed.\n\
         tok=$(cat '{token_file}' 2>/dev/null)\n\
         payload=$(cat)\n\
         case \"$payload\" in\n\
           *'\"hook_event_name\":\"{HELD_EVENT}\"'*|*'\"hook_event_name\": \"{HELD_EVENT}\"'*)\n\
             printf '%s' \"$payload\" | curl -s -m {HELD_TIMEOUT_SECS} -X POST {hdr} --data-binary @- '{url}' 2>/dev/null\n\
             ;;\n\
           *)\n\
             printf '%s' \"$payload\" | curl -s -m 3 -X POST {hdr} --data-binary @- '{url}' >/dev/null 2>&1\n\
             ;;\n\
         esac\n\
         exit 0\n"
    )
}

/// The shared secret. Created on first use with mode 0600. Any process that
/// runs as the same user can read it, so the token keeps out other users,
/// sandboxed apps and web pages, not the user's own processes.
pub fn token() -> anyhow::Result<String> {
    let p = paths::hook_token_file();
    if let Ok(t) = std::fs::read_to_string(&p) {
        let t = t.trim().to_string();
        if t.len() >= 32 {
            return Ok(t);
        }
    }
    // Two v4 UUIDs, hyphens removed: 256 bits from the platform's own random
    // source. Windows has no /dev/urandom, and `uuid` already reaches the right
    // generator on every platform kari runs on.
    let t: String = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    std::fs::create_dir_all(paths::kari_dir())?;
    std::fs::write(&p, &t)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(t)
}

/// Rewrite the relay script when an installed copy predates the token header
/// or the held event. kari owns the script, so this needs no click. The hook
/// entry for the held event needs a reinstall, which `installed_events` reports.
pub fn refresh_script(port: u16) -> anyhow::Result<()> {
    let sp = script_path();
    let Ok(current) = std::fs::read_to_string(&sp) else {
        return Ok(());
    };
    if current.contains(TOKEN_HEADER) && current.contains(HELD_EVENT) {
        return Ok(());
    }
    std::fs::write(&sp, script(port))?;
    Ok(())
}

/// True when settings.json registers kari for the held event. An older install
/// lacks it, and Away mode then cannot hold anything.
pub fn held_event_installed() -> bool {
    let Ok(v) = read_settings() else { return false };
    v.get("hooks")
        .and_then(|h| h.get(HELD_EVENT))
        .and_then(|a| a.as_array())
        .is_some_and(|a| a.iter().any(is_kari_group))
}

fn settings_path() -> PathBuf {
    paths::claude_dir().join("settings.json")
}

pub(crate) fn read_settings() -> anyhow::Result<Value> {
    let p = settings_path();
    if !p.exists() {
        return Ok(json!({}));
    }
    let text = std::fs::read_to_string(&p)?;
    if text.trim().is_empty() {
        return Ok(json!({}));
    }
    Ok(serde_json::from_str(&text)?)
}

pub(crate) fn write_settings(v: &Value) -> anyhow::Result<()> {
    let p = settings_path();
    if let Some(dir) = p.parent() {
        std::fs::create_dir_all(dir)?;
    }
    if p.exists() {
        let stamp = Utc::now().format("%Y%m%d-%H%M%S");
        let backup = paths::kari_dir().join(format!("claude-settings-{stamp}.json.bak"));
        std::fs::create_dir_all(paths::kari_dir())?;
        std::fs::copy(&p, &backup)?;
        // The settings file can hold API keys. Keep the backup private to the user.
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&backup, std::fs::Permissions::from_mode(0o600))?;
        }
    }
    let tmp = p.with_extension("json.kari-tmp");
    std::fs::write(&tmp, serde_json::to_string_pretty(v)? + "\n")?;
    std::fs::rename(&tmp, &p)?;
    Ok(())
}

fn is_kari_group(group: &Value) -> bool {
    group
        .get("hooks")
        .and_then(|h| h.as_array())
        .map(|arr| {
            !arr.is_empty()
                && arr.iter().all(|h| {
                    h.get("command")
                        .and_then(|c| c.as_str())
                        .is_some_and(is_kari_command)
                })
        })
        .unwrap_or(false)
}

fn strip_kari(hooks: &mut Value) {
    if let Some(map) = hooks.as_object_mut() {
        for (_, groups) in map.iter_mut() {
            if let Some(arr) = groups.as_array_mut() {
                arr.retain(|g| !is_kari_group(g));
            }
        }
        map.retain(|_, v| v.as_array().is_none_or(|a| !a.is_empty()));
    }
}

/// True when settings.json carries kari's hook entries.
pub fn installed() -> bool {
    let Ok(v) = read_settings() else { return false };
    v.get("hooks")
        .and_then(|h| h.as_object())
        .map(|m| {
            m.values().any(|groups| {
                groups
                    .as_array()
                    .is_some_and(|a| a.iter().any(is_kari_group))
            })
        })
        .unwrap_or(false)
}

/// Prepare the relay and register it for every event kari listens to.
/// Returns the command line that went into settings.json.
pub fn install(port: u16) -> anyhow::Result<String> {
    let cmd = relay_command(port)?;
    let mut v = read_settings()?;
    if !v.is_object() {
        anyhow::bail!("settings.json is not a JSON object");
    }
    let hooks = v
        .as_object_mut()
        .unwrap()
        .entry("hooks")
        .or_insert_with(|| json!({}));
    if !hooks.is_object() {
        *hooks = json!({});
    }
    strip_kari(hooks);
    for (event, matcher) in EVENTS {
        let timeout = if *event == HELD_EVENT {
            HELD_TIMEOUT_SECS
        } else {
            5
        };
        let mut group = json!({ "hooks": [ { "type": "command", "command": cmd.clone(), "timeout": timeout } ] });
        if let Some(m) = matcher {
            group["matcher"] = json!(m);
        }
        let arr = hooks
            .as_object_mut()
            .unwrap()
            .entry(*event)
            .or_insert_with(|| json!([]));
        if !arr.is_array() {
            *arr = json!([]);
        }
        arr.as_array_mut().unwrap().push(group);
    }
    write_settings(&v)?;
    Ok(cmd)
}

/// Remove kari's hook entries. Other hooks stay untouched.
pub fn uninstall() -> anyhow::Result<()> {
    let mut v = read_settings()?;
    if let Some(hooks) = v.get_mut("hooks") {
        strip_kari(hooks);
        if hooks.as_object().is_some_and(|m| m.is_empty()) {
            v.as_object_mut().unwrap().remove("hooks");
        }
        write_settings(&v)?;
    }
    let _ = std::fs::remove_file(script_path());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognises_both_shapes_of_its_own_entry() {
        // Unix: the relay script, by path.
        assert!(is_kari_command("/home/you/.config/kari/hook.sh"));
        // Windows: the node binary with the relay subcommand.
        assert!(is_kari_command(
            "\"C:\\Users\\you\\kari\\kari-node.exe\" hooks relay --port 47311"
        ));
    }

    #[test]
    fn leaves_a_foreign_hook_alone() {
        // `uninstall` strips every command this returns true for, so a hook
        // that only looks similar must not match.
        assert!(!is_kari_command("/usr/local/bin/my-own-hook.sh"));
        assert!(!is_kari_command("notify-send 'hooks relay finished'"));
        assert!(!is_kari_command(""));
    }

    #[test]
    fn the_held_event_is_read_from_the_payload() {
        assert!(is_held_event(
            r#"{"session_id":"s","hook_event_name":"PermissionRequest"}"#
        ));
        assert!(!is_held_event(
            r#"{"session_id":"s","hook_event_name":"SessionStart"}"#
        ));
        // A tool named after the event is not the event.
        assert!(!is_held_event(
            r#"{"hook_event_name":"PreToolUse","tool_name":"PermissionRequest"}"#
        ));
    }
}
