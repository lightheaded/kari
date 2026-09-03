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

pub fn kari_dir() -> PathBuf {
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

pub fn kari_db() -> PathBuf {
    kari_dir().join("kari.db")
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
