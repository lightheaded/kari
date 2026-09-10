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

/// Where the files attached to cards live, one directory per card.
///
/// The directory is the store: there is no table beside it. A row and a file
/// can disagree, and then a card shows an attachment that a run cannot read.
/// One `read_dir` cannot disagree with itself.
pub fn attachments_dir() -> PathBuf {
    kari_dir().join("attachments")
}

/// The directory that holds the files of one card.
pub fn card_attachments_dir(card_id: &str) -> PathBuf {
    attachments_dir().join(safe_file_name(card_id))
}

/// A file name that cannot leave the directory it is written in, and that
/// needs no encoding anywhere kari carries it. Every character outside the
/// safe set becomes `_`, and a name that says nothing after that becomes
/// `file`.
///
/// The name comes from a client, so `../..` and a leading `/` must not survive
/// it. The check is a whitelist rather than a search for the bad shapes,
/// because the bad shapes differ per platform and the whitelist does not.
///
/// A space is outside the set as well, and that is not about the file system.
/// The name is the last segment of a URL on the node API, and that URL travels
/// raw inside a link frame, where `Request::builder().uri()` refuses a space.
/// The name is also one line of the run prompt, and a path with a space in a
/// list of paths is ambiguous to the reader. So `my shot.png` is stored as
/// `my_shot.png`.
pub fn safe_file_name(name: &str) -> String {
    let cleaned: String = name
        .chars()
        .map(|c| match c {
            'a'..='z' | 'A'..='Z' | '0'..='9' | '.' | '-' | '_' => c,
            _ => '_',
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        // A long name is a name a file system refuses. 120 bytes leaves room
        // for the numeric suffix a collision adds.
        cleaned.chars().take(120).collect()
    }
}

/// The shared secret between the hook relay script and the receiver.
pub fn hook_token_file() -> PathBuf {
    kari_dir().join("hook-token")
}

/// The shared secret a node presents when it dials a server, and the one a
/// server checks. Separate from the node's own token: they guard different
/// things and are held by different people.
pub fn server_token_file() -> PathBuf {
    kari_dir().join("server-token")
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
///
/// The order matters, and the first entry is the whole point. A node names
/// itself with this when nobody has named it, and asking a *program* for the
/// answer means the answer depends on `PATH`. Under systemd it does not
/// survive: a unit gets only the `path` its module lists, `hostname` lives in
/// neither coreutils nor systemd, and NixOS writes no `/etc/hostname` when the
/// host name comes from outside (a container taking its name from its host).
/// Every source then fails and the node calls itself "kari" — the app's name,
/// on every machine at once, which is exactly the name that must not be
/// guessable from nothing. The kernel is asked first because it always knows
/// and needs no PATH.
pub fn hostname() -> String {
    // The kernel's own answer. No process, no PATH, always present on Linux.
    #[cfg(target_os = "linux")]
    let kernel = std::fs::read_to_string("/proc/sys/kernel/hostname").ok();
    #[cfg(not(target_os = "linux"))]
    let kernel: Option<String> = None;

    // Windows has no `hostname -s` and no /etc/hostname, but every session
    // carries the name in the environment.
    #[cfg(windows)]
    let spawned = std::env::var("COMPUTERNAME").ok();
    #[cfg(not(windows))]
    let spawned = || {
        std::process::Command::new("hostname")
            .arg("-s")
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
    };
    #[cfg(windows)]
    let spawned = || spawned.clone();

    kernel
        .filter(|s: &String| !s.trim().is_empty())
        .or_else(spawned)
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

/// Read a project directory that a caller sent for a card.
///
/// None and an empty string both mean "no project". Every other value must be
/// a directory on this node: a card that holds a display name, or a path from
/// another machine, can neither run nor open a terminal. The caller passes the
/// error straight to the user, who can then pick another path.
pub fn checked_project_cwd(cwd: Option<&str>) -> anyhow::Result<Option<String>> {
    match cwd.map(str::trim) {
        None | Some("") => Ok(None),
        Some(c) if is_usable_cwd(c) => Ok(Some(c.to_string())),
        Some(c) => anyhow::bail!("{c} is not a directory on this node"),
    }
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

    /// The regression this guards cost a node its name. A systemd unit is
    /// given only the `path` its module lists, `hostname` is in neither
    /// coreutils nor systemd, and a NixOS container that takes its name from
    /// its host has no `/etc/hostname`. Every source failed and the node
    /// called itself "kari" — the app's own name — which on a board of several
    /// such nodes makes them indistinguishable.
    #[test]
    #[cfg(target_os = "linux")]
    fn the_host_name_does_not_depend_on_a_program_being_on_path() {
        let kernel =
            std::fs::read_to_string("/proc/sys/kernel/hostname").expect("linux always has this");
        let kernel = kernel.trim().split('.').next().unwrap_or("").to_string();
        if kernel.is_empty() {
            return; // Nothing to compare against; the fallback chain is right.
        }

        // Whatever PATH holds, the kernel's answer is the one that comes back.
        let restored = std::env::var_os("PATH");
        // SAFETY: single-threaded test, and PATH is put back before it ends.
        unsafe { std::env::set_var("PATH", "") };
        let got = hostname();
        match restored {
            Some(p) => unsafe { std::env::set_var("PATH", p) },
            None => unsafe { std::env::remove_var("PATH") },
        }

        assert_eq!(got, kernel);
        assert_ne!(
            got, "kari",
            "a node fell back to the app name instead of asking the kernel"
        );
    }

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

    #[test]
    fn a_card_takes_only_a_directory_that_is_here() {
        // Nothing at all, and a field the user cleared, both mean no project.
        assert_eq!(checked_project_cwd(None).unwrap(), None);
        assert_eq!(checked_project_cwd(Some("")).unwrap(), None);
        assert_eq!(checked_project_cwd(Some("   ")).unwrap(), None);

        // A directory that is here goes through, without its stray spaces. The
        // test makes its own directory: a build sandbox sets HOME to a path
        // that does not exist, so home() is not safe to assert on.
        let dir = std::env::temp_dir().join(format!("kari-cwd-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let ok = dir.to_string_lossy().into_owned();
        assert_eq!(checked_project_cwd(Some(&ok)).unwrap(), Some(ok.clone()));
        assert_eq!(
            checked_project_cwd(Some(&format!(" {ok} "))).unwrap(),
            Some(ok)
        );
        std::fs::remove_dir_all(&dir).ok();

        // A display name, and a path from another machine, are both refused.
        let err = checked_project_cwd(Some("kari")).unwrap_err().to_string();
        assert!(err.contains("kari"), "{err}");
        assert!(err.contains("not a directory on this node"), "{err}");
        assert!(checked_project_cwd(Some("/no/such/path/on/this/node")).is_err());
    }
}
