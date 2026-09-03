//! Well-known locations. Claude Code and herdr write here; kari reads.

use std::path::PathBuf;

pub fn home() -> PathBuf {
    dirs::home_dir().unwrap_or_else(|| PathBuf::from("/"))
}

pub fn claude_dir() -> PathBuf {
    if let Ok(p) = std::env::var("CLAUDE_CONFIG_DIR") {
        return PathBuf::from(p);
    }
    home().join(".claude")
}

pub fn claude_sessions_dir() -> PathBuf {
    claude_dir().join("sessions")
}

pub fn claude_projects_dir() -> PathBuf {
    claude_dir().join("projects")
}

pub fn claude_jobs_dir() -> PathBuf {
    claude_dir().join("jobs")
}

static KARI_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// Put the kari directory somewhere else, once, before the first lookup. A
/// second call after a lookup has no effect. For a device without a home
/// directory, such as a phone.
pub fn set_kari_dir(dir: &std::path::Path) {
    let _ = KARI_DIR.set(dir.to_path_buf());
}

pub fn kari_dir() -> PathBuf {
    if let Some(d) = KARI_DIR.get() {
        return d.clone();
    }
    let p = dirs::config_dir()
        .map(|c| c.join("kari"))
        .unwrap_or_else(|| home().join(".config/kari"));
    // On macOS dirs::config_dir is ~/Library/Application Support; prefer ~/.config like herdr.
    let dot = home().join(".config/kari");
    if cfg!(target_os = "macos") {
        dot
    } else {
        p
    }
}

/// Work directory of kari's own `claude -p` runs, such as the summarizer.
/// Sessions that start here are kari's, not the user's.
pub fn internal_cwd_prefix() -> String {
    kari_dir().join("summaries").to_string_lossy().into_owned()
}

/// True for a session that kari itself started for internal work.
pub fn is_internal_cwd(cwd: &str) -> bool {
    let prefix = internal_cwd_prefix();
    cwd == prefix || cwd.starts_with(&format!("{prefix}/"))
}

pub fn kari_db() -> PathBuf {
    kari_dir().join("kari.db")
}

/// The shared secret between the hook relay script and the receiver.
pub fn hook_token_file() -> PathBuf {
    kari_dir().join("hook-token")
}

pub fn rate_limits_file() -> PathBuf {
    kari_dir().join("rate-limits.json")
}

pub fn herdr_socket() -> PathBuf {
    if let Ok(p) = std::env::var("HERDR_SOCKET_PATH") {
        return PathBuf::from(p);
    }
    home().join(".config/herdr/herdr.sock")
}

/// PATH for child processes. GUI apps inherit a short PATH from launchd.
pub fn child_path() -> String {
    let mut parts: Vec<String> = vec![
        home().join(".local/bin").to_string_lossy().into_owned(),
        home().join(".cargo/bin").to_string_lossy().into_owned(),
        "/opt/homebrew/bin".into(),
        "/usr/local/bin".into(),
        "/usr/bin".into(),
        "/bin".into(),
        "/usr/sbin".into(),
        "/sbin".into(),
    ];
    if let Ok(p) = std::env::var("PATH") {
        for seg in p.split(':') {
            if !parts.iter().any(|x| x == seg) {
                parts.push(seg.to_string());
            }
        }
    }
    parts.join(":")
}

/// The machine's host name, without a domain. Falls back to "kari".
pub fn hostname() -> String {
    let from_cmd = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty());
    from_cmd
        .or_else(|| std::fs::read_to_string("/etc/hostname").ok())
        .map(|s| s.trim().split('.').next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            if cfg!(target_os = "android") {
                "phone".into()
            } else {
                "kari".into()
            }
        })
}

pub fn which(bin: &str) -> Option<PathBuf> {
    for dir in child_path().split(':') {
        let p = PathBuf::from(dir).join(bin);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Claude Code escapes a cwd into a project slug: every non-alphanumeric byte becomes '-'.
pub fn project_slug(cwd: &str) -> String {
    cwd.chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect()
}

pub fn project_display_name(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}
