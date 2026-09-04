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
    // Compare as paths, not as strings: Windows separates with a backslash, so
    // a string prefix test would miss every internal session there.
    std::path::Path::new(cwd).starts_with(&prefix)
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

/// The character that separates entries in PATH: ';' on Windows, ':' elsewhere.
pub const PATH_SEP: char = if cfg!(windows) { ';' } else { ':' };

/// PATH for child processes. GUI apps inherit a short PATH from launchd, and a
/// Windows service inherits the machine PATH rather than the user's, so the
/// directories the Claude Code installer uses are named here in both cases.
pub fn child_path() -> String {
    let mut parts: Vec<String> = if cfg!(windows) {
        let mut v = vec![
            home().join(".local/bin").to_string_lossy().into_owned(),
            home().join(".cargo/bin").to_string_lossy().into_owned(),
        ];
        // npm puts `claude.cmd` here when Claude Code came from npm rather
        // than from the native installer.
        if let Ok(appdata) = std::env::var("APPDATA") {
            v.push(
                std::path::Path::new(&appdata)
                    .join("npm")
                    .to_string_lossy()
                    .into_owned(),
            );
        }
        v
    } else {
        vec![
            home().join(".local/bin").to_string_lossy().into_owned(),
            home().join(".cargo/bin").to_string_lossy().into_owned(),
            "/opt/homebrew/bin".into(),
            "/usr/local/bin".into(),
            "/usr/bin".into(),
            "/bin".into(),
            "/usr/sbin".into(),
            "/sbin".into(),
        ]
    };
    if let Ok(p) = std::env::var("PATH") {
        for seg in p.split(PATH_SEP) {
            if !parts.iter().any(|x| x == seg) {
                parts.push(seg.to_string());
            }
        }
    }
    parts.join(&PATH_SEP.to_string())
}

/// The machine's host name, without a domain. Falls back to "kari".
pub fn hostname() -> String {
    // Windows has no `hostname -s` and no /etc/hostname, but every session
    // carries the name in the environment.
    #[cfg(windows)]
    let first = std::env::var("COMPUTERNAME").ok();
    #[cfg(not(windows))]
    let first = std::process::Command::new("hostname")
        .arg("-s")
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    first
        .filter(|s: &String| !s.trim().is_empty())
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

/// Extensions to try after the bare name. Windows stores the executable bit in
/// the suffix, so `which("claude")` there must find `claude.exe` or `claude.cmd`.
#[cfg(windows)]
const EXE_SUFFIXES: &[&str] = &["", ".exe", ".cmd", ".bat", ".com"];
#[cfg(not(windows))]
const EXE_SUFFIXES: &[&str] = &[""];

pub fn which(bin: &str) -> Option<PathBuf> {
    for dir in child_path().split(PATH_SEP) {
        for suffix in EXE_SUFFIXES {
            let p = PathBuf::from(dir).join(format!("{bin}{suffix}"));
            if p.is_file() {
                return Some(p);
            }
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

/// True for a path that a child process can start in: absolute, and a
/// directory that exists now. A display name such as "kari" fails this test.
pub fn is_usable_cwd(cwd: &str) -> bool {
    let p = std::path::Path::new(cwd);
    p.is_absolute() && p.is_dir()
}

pub fn project_display_name(cwd: &str) -> String {
    std::path::Path::new(cwd)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| cwd.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_display_name_is_not_a_working_directory() {
        // Only paths this test controls. A build sandbox sets HOME to a
        // directory that does not exist, so home() is not safe to assert on.
        // "/" is not absolute on Windows, where a root needs a drive letter.
        let root = if cfg!(windows) { "C:\\" } else { "/" };
        assert!(is_usable_cwd(root));
        // The bug this guards: a project name reached a card as its directory.
        assert!(!is_usable_cwd("kari"));
        assert!(!is_usable_cwd(""));
        assert!(!is_usable_cwd("/no/such/path/for/kari/tests"));
    }

    #[test]
    fn an_internal_cwd_is_recognised_on_every_platform() {
        // The prefix test used to be a string compare, which no separator but
        // "/" could satisfy. A summary session must be spotted as kari's own.
        let inside = kari_dir().join("summaries").join("one");
        assert!(is_internal_cwd(&inside.to_string_lossy()));
        assert!(is_internal_cwd(&internal_cwd_prefix()));
        // A sibling directory whose name merely starts the same is not inside.
        let sibling = kari_dir().join("summaries-elsewhere");
        assert!(!is_internal_cwd(&sibling.to_string_lossy()));
    }
}
