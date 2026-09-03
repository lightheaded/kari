//! Install the status line wrapper that records rate limits on every refresh.
//!
//! The same steps as `scripts/install-statusline.sh`, for hosts without the
//! repository: `kari-node statusline install`. The wrapper runs the user's
//! original status line command unchanged after it saved the sample.

use crate::{hooks, paths, quota};
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
    current_command(&v).as_deref() == Some(wrapper_path().to_string_lossy().as_ref())
}

/// Wrap the current status line command. Returns one line for the user.
pub fn install() -> anyhow::Result<String> {
    let mut v = hooks::read_settings()?;
    if !v.is_object() {
        anyhow::bail!("settings.json is not a JSON object");
    }
    let wrapper = wrapper_path();
    let wrapper_str = wrapper.to_string_lossy().into_owned();
    let current = current_command(&v);
    if current.as_deref() == Some(wrapper_str.as_str()) {
        return Ok("status line wrapper already installed".into());
    }
    let original = current.unwrap_or_else(|| MINIMAL.to_string());
    std::fs::create_dir_all(paths::kari_dir())?;
    std::fs::write(original_path(), &original)?;
    std::fs::write(&wrapper, quota::wrapper_script(&original))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&wrapper, std::fs::Permissions::from_mode(0o755))?;
    }
    let obj = v.as_object_mut().unwrap();
    let sl = obj.entry("statusLine").or_insert_with(|| json!({}));
    if !sl.is_object() {
        *sl = json!({});
    }
    let slo = sl.as_object_mut().unwrap();
    slo.entry("type").or_insert_with(|| json!("command"));
    slo.insert("command".into(), json!(wrapper_str));
    hooks::write_settings(&v)?;
    Ok(format!(
        "status line wrapper installed at {}; original command saved",
        wrapper.display()
    ))
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
