//! Install the status line wrapper that records rate limits on every refresh.
//!
//! The same steps as `scripts/install-statusline.sh`, for hosts without the
//! repository: `kari-node statusline install`. The wrapper runs the user's
//! original status line command unchanged after it saved the sample.

use crate::{hooks, paths};
use serde_json::{json, Value};
use std::path::PathBuf;

const MINIMAL: &str =
    r#"jq -r '"[\(.model.display_name)] \(.context_window.used_percentage // 0)% ctx"'"#;

pub fn wrapper_path() -> PathBuf {
    paths::kari_dir().join("statusline.sh")
}

fn original_path() -> PathBuf {
    paths::kari_dir().join("statusline.original")
}

/// The command Claude Code runs for the status line once kari has wrapped it,
/// and the side effect of preparing it.
///
/// Unix writes a `bash` script that saves the sample with `jq` and then runs
/// the original. Windows names the node binary instead: it reads the same
/// payload on stdin, writes the sample, and runs the original itself. That
/// needs neither `bash` nor `jq`, neither of which ships with Windows.
fn wrapper_command() -> anyhow::Result<String> {
    if cfg!(windows) {
        let exe = std::env::current_exe()?;
        return Ok(format!("\"{}\" statusline capture", exe.display()));
    }
    Ok(wrapper_path().to_string_lossy().into_owned())
}

fn current_command(settings: &Value) -> Option<String> {
    settings
        .get("statusLine")
        .and_then(|s| s.get("command"))
        .and_then(|c| c.as_str())
        .map(|s| s.to_string())
        .filter(|s| !s.is_empty())
}

/// True when settings.json points at kari's wrapper.
pub fn installed() -> bool {
    let Ok(v) = hooks::read_settings() else {
        return false;
    };
    let Ok(ours) = wrapper_command() else {
        return false;
    };
    current_command(&v).as_deref() == Some(ours.as_str())
}

/// Wrap the current status line command. Returns one line for the user.
pub fn install() -> anyhow::Result<String> {
    let mut v = hooks::read_settings()?;
    if !v.is_object() {
        anyhow::bail!("settings.json is not a JSON object");
    }
    let wrapper_str = wrapper_command()?;
    let current = current_command(&v);
    if current.as_deref() == Some(wrapper_str.as_str()) {
        return Ok("status line wrapper already installed".into());
    }
    let original = current.unwrap_or_else(|| MINIMAL.to_string());
    std::fs::create_dir_all(paths::kari_dir())?;
    std::fs::write(original_path(), &original)?;
    // Windows runs the binary named in `wrapper_str`; there is no script file.
    #[cfg(not(windows))]
    {
        let wrapper = wrapper_path();
        std::fs::write(&wrapper, crate::quota::wrapper_script(&original))?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
        }
    }
    let obj = v.as_object_mut().unwrap();
    let sl = obj.entry("statusLine").or_insert_with(|| json!({}));
    if !sl.is_object() {
        *sl = json!({});
    }
    let slo = sl.as_object_mut().unwrap();
    slo.entry("type").or_insert_with(|| json!("command"));
    slo.insert("command".into(), json!(wrapper_str.clone()));
    hooks::write_settings(&v)?;
    Ok(format!(
        "status line wrapper installed as {wrapper_str}; original command saved"
    ))
}

/// Record the sample, then run the command kari wrapped and give it the same
/// bytes on stdin. What that command prints is what Claude Code shows, so this
/// stays quiet: a failure to record must not cost the user their status line.
///
/// This is the Windows half of `quota::wrapper_script`. Returns the original
/// command's stdout.
pub fn capture(input: &str) -> String {
    if let Err(e) = crate::quota::capture(input, chrono::Utc::now()) {
        tracing::debug!("status line sample not recorded: {e}");
    }
    let Ok(original) = std::fs::read_to_string(original_path()) else {
        return String::new();
    };
    let original = original.trim();
    if original.is_empty() {
        return String::new();
    }
    run_shell(original, input)
}

/// Run a command line the way the platform's shell would, with `input` on stdin.
fn run_shell(command: &str, input: &str) -> String {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let mut c = if cfg!(windows) {
        let mut c = Command::new("cmd");
        c.arg("/C").arg(command);
        c
    } else {
        let mut c = Command::new("sh");
        c.arg("-c").arg(command);
        c
    };
    let child = c
        .env("PATH", paths::child_path())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let Ok(mut child) = child else {
        return String::new();
    };
    if let Some(mut si) = child.stdin.take() {
        let _ = si.write_all(input.as_bytes());
    }
    match child.wait_with_output() {
        Ok(o) => String::from_utf8_lossy(&o.stdout).into_owned(),
        Err(_) => String::new(),
    }
}

/// Put the original command back and remove the wrapper.
pub fn uninstall() -> anyhow::Result<String> {
    let orig = match std::fs::read_to_string(original_path()) {
        Ok(o) => o,
        Err(_) => return Ok("nothing to restore".into()),
    };
    let mut v = hooks::read_settings()?;
    if let Some(sl) = v.get_mut("statusLine").and_then(|s| s.as_object_mut()) {
        if orig.trim() == MINIMAL {
            sl.remove("command");
        } else {
            sl.insert("command".into(), json!(orig));
        }
    }
    hooks::write_settings(&v)?;
    let _ = std::fs::remove_file(original_path());
    let _ = std::fs::remove_file(wrapper_path());
    Ok(format!("restored status line command: {}", orig.trim()))
}
