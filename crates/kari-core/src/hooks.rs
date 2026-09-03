//! Claude Code hooks: parse incoming events, fold them into per-session state,
//! and install or remove the hook entries in `~/.claude/settings.json`.
//!
//! Claude Code runs a small shell script on each event. The script posts the
//! JSON payload to kari on 127.0.0.1 and always exits 0, so a stopped kari
//! never disturbs a session.

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
const MARKER: &str = "kari/hook.sh";

/// Events kari registers. PostToolUse clears a pending permission, so it needs every tool.
const EVENTS: &[(&str, Option<&str>)] = &[
    ("SessionStart", None),
    ("SessionEnd", None),
    ("UserPromptSubmit", None),
    ("Stop", None),
    ("Notification", None),
    ("PreToolUse", Some("AskUserQuestion|ExitPlanMode")),
    ("PostToolUse", Some("*")),
];

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

pub fn script(port: u16) -> String {
    let token_file = paths::hook_token_file().to_string_lossy().into_owned();
    format!(
        "#!/bin/sh\n# kari hook relay. Posts the Claude Code hook payload to kari and never fails.\n\
         tok=$(cat '{token_file}' 2>/dev/null)\n\
         curl -s -m 3 -X POST -H 'content-type: application/json' -H \"{TOKEN_HEADER}: $tok\" --data-binary @- \\\n  \
         'http://127.0.0.1:{port}{HOOK_PATH}' >/dev/null 2>&1\nexit 0\n"
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
    let mut bytes = [0u8; 32];
    std::fs::File::open("/dev/urandom")
        .and_then(|mut f| std::io::Read::read_exact(&mut f, &mut bytes))?;
    let t: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
    std::fs::create_dir_all(paths::kari_dir())?;
    std::fs::write(&p, &t)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(t)
}

/// Rewrite the relay script when an installed copy predates the token header.
/// kari owns the script, so this needs no click.
pub fn refresh_script(port: u16) -> anyhow::Result<()> {
    let sp = script_path();
    let Ok(current) = std::fs::read_to_string(&sp) else {
        return Ok(());
    };
    if current.contains(TOKEN_HEADER) {
        return Ok(());
    }
    std::fs::write(&sp, script(port))?;
    Ok(())
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
                        .is_some_and(|c| c.contains(MARKER))
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

/// Write the relay script and register it for every event kari listens to.
pub fn install(port: u16) -> anyhow::Result<PathBuf> {
    let sp = script_path();
    std::fs::create_dir_all(paths::kari_dir())?;
    std::fs::write(&sp, script(port))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&sp, std::fs::Permissions::from_mode(0o755))?;
    }
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
    let cmd = sp.to_string_lossy().into_owned();
    for (event, matcher) in EVENTS {
        let mut group = json!({ "hooks": [ { "type": "command", "command": cmd, "timeout": 5 } ] });
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
    Ok(sp)
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
