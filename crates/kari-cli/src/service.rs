//! Install the node as a service of the user, so a host watches itself.
//!
//! kari must hold the state of a session whether or not a window is open, and
//! the window is not always open: a laptop gets closed, and the app is quit.
//! Until now the answer was "a node is installed by whatever manages its host",
//! which is true of a server in a container and false of a laptop. On a laptop,
//! the thing that manages the host is kari.
//!
//! So the node writes its own service file and asks the supervisor of the user
//! session to keep it running: `launchd` on macOS, `systemd --user` on Linux.
//! Both restart it when it exits, which the node depends on twice — once for an
//! update it installed over itself, and once for the moment a window opens and
//! the node steps down (see `kari_core::owner`).
//!
//! It is a service of the *user*, never of the system. The node reads
//! `~/.claude`, runs Claude Code and holds that user's login, so it belongs to
//! the session that owns those, and it must not run before the user logs in.

use std::path::{Path, PathBuf};
#[cfg(any(target_os = "macos", target_os = "linux"))]
use std::process::Command;

/// The name `launchd` knows the node by. The systemd unit is named by its file.
#[cfg(target_os = "macos")]
pub const LABEL: &str = "io.github.lightheaded.kari-node";

/// What the service will run, beyond `serve`.
pub struct Options {
    /// Link to this server, as `kari-node serve --server` would.
    pub server: Option<String>,
    /// Let the node replace its own binary.
    pub auto_update: bool,
}

#[cfg(target_os = "macos")]
fn log_file() -> PathBuf {
    kari_core::paths::kari_dir().join("node.log")
}

/// The binary the service must run.
///
/// The path of the running program, made absolute. It goes into a file that a
/// supervisor reads later, so a relative path or a symlink into a build
/// directory would break the service the next time the host starts.
fn program() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(std::fs::canonicalize(&exe).unwrap_or(exe))
}

fn args(opt: &Options) -> Vec<String> {
    let mut v = vec!["serve".to_string()];
    if let Some(s) = &opt.server {
        v.push("--server".into());
        v.push(s.clone());
    }
    if opt.auto_update {
        v.push("--auto-update".into());
    }
    v
}

/// Install the service and start it.
pub fn install(opt: Options) -> anyhow::Result<String> {
    let program = program()?;
    if !program.exists() {
        anyhow::bail!("{} is not there any more", program.display());
    }
    let out = install_for_platform(&program, &opt)?;
    Ok(out)
}

// ------------------------------------------------------------------- macOS

#[cfg(target_os = "macos")]
fn plist_path() -> PathBuf {
    kari_core::paths::home()
        .join("Library/LaunchAgents")
        .join(format!("{LABEL}.plist"))
}

#[cfg(target_os = "macos")]
fn install_for_platform(program: &Path, opt: &Options) -> anyhow::Result<String> {
    let mut items = vec![program.display().to_string()];
    items.extend(args(opt));
    let program_args: String = items
        .iter()
        .map(|a| format!("    <string>{}</string>\n", xml(a)))
        .collect();
    let log = log_file();
    if let Some(d) = log.parent() {
        std::fs::create_dir_all(d)?;
    }

    // KeepAlive, because the node exits on purpose in two cases and must come
    // back in both: after it installs an update over itself, and after a window
    // opens and it steps down. ThrottleInterval keeps that second case from
    // becoming a restart loop while the window stays open — it comes back, sees
    // the claim, and waits.
    let plist = format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>{LABEL}</string>
  <key>ProgramArguments</key>
  <array>
{program_args}  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <true/>
  <key>ThrottleInterval</key>
  <integer>10</integer>
  <key>ProcessType</key>
  <string>Background</string>
  <key>StandardOutPath</key>
  <string>{log}</string>
  <key>StandardErrorPath</key>
  <string>{log}</string>
</dict>
</plist>
"#,
        log = xml(&log.display().to_string()),
    );

    let path = plist_path();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&path, plist)?;

    // Out first, so a changed plist is the one that runs. A service that was
    // not loaded makes this fail, and that is not an error.
    let target = format!("gui/{}", unsafe { libc::getuid() });
    let _ = launchctl(&["bootout", &format!("{target}/{LABEL}")]);
    launchctl(&["bootstrap", &target, &path.display().to_string()])?;
    launchctl(&["enable", &format!("{target}/{LABEL}")])?;
    Ok(format!(
        "the node runs at login and now: {}. Its log is {}",
        path.display(),
        log.display()
    ))
}

#[cfg(target_os = "macos")]
fn launchctl(args: &[&str]) -> anyhow::Result<()> {
    let out = Command::new("launchctl").args(args).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "launchctl {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn uninstall() -> anyhow::Result<String> {
    let path = plist_path();
    let target = format!("gui/{}/{LABEL}", unsafe { libc::getuid() });
    let _ = launchctl(&["bootout", &target]);
    match std::fs::remove_file(&path) {
        Ok(()) => Ok(format!(
            "the node no longer runs at login: {} is removed",
            path.display()
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok("the node was not installed as a service".into())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(target_os = "macos")]
pub fn status() -> anyhow::Result<String> {
    let path = plist_path();
    if !path.exists() {
        return Ok("not installed".into());
    }
    let out = Command::new("launchctl")
        .args([
            "print",
            &format!("gui/{}/{LABEL}", unsafe { libc::getuid() }),
        ])
        .output()?;
    if !out.status.success() {
        return Ok(format!("installed at {} but not loaded", path.display()));
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let pid = text
        .lines()
        .find_map(|l| l.trim().strip_prefix("pid = "))
        .unwrap_or("none");
    Ok(format!(
        "installed at {}, loaded, pid {pid}",
        path.display()
    ))
}

/// Escape the three characters that would end the element early. A path can
/// hold an ampersand, and one unescaped makes the whole file unreadable to
/// `launchd`, which then says nothing at all.
#[cfg(target_os = "macos")]
fn xml(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

// ------------------------------------------------------------------- Linux

#[cfg(target_os = "linux")]
fn unit_path() -> PathBuf {
    let base = match std::env::var("XDG_CONFIG_HOME") {
        Ok(p) if !p.trim().is_empty() => PathBuf::from(p),
        _ => kari_core::paths::home().join(".config"),
    };
    base.join("systemd/user").join("kari-node.service")
}

#[cfg(target_os = "linux")]
fn install_for_platform(program: &Path, opt: &Options) -> anyhow::Result<String> {
    let mut items = vec![program.display().to_string()];
    items.extend(args(opt));
    // Restart=always, not on-failure: the node exits cleanly both after it
    // updates itself and when a window takes the engine, and on-failure treats
    // a clean exit as the end of the job.
    let unit = format!(
        "[Unit]\n\
         Description=kari node\n\
         After=default.target\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={cmd}\n\
         Restart=always\n\
         RestartSec=10\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        cmd = items.join(" "),
    );
    let path = unit_path();
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    std::fs::write(&path, unit)?;
    systemctl(&["daemon-reload"])?;
    systemctl(&["enable", "--now", "kari-node.service"])?;
    Ok(format!(
        "the node runs at login and now: {}. Read its log with `journalctl --user -u kari-node`",
        path.display()
    ))
}

#[cfg(target_os = "linux")]
fn systemctl(args: &[&str]) -> anyhow::Result<()> {
    let mut all = vec!["--user"];
    all.extend_from_slice(args);
    let out = Command::new("systemctl").args(&all).output()?;
    if !out.status.success() {
        anyhow::bail!(
            "systemctl {}: {}",
            all.join(" "),
            String::from_utf8_lossy(&out.stderr).trim()
        );
    }
    Ok(())
}

#[cfg(target_os = "linux")]
pub fn uninstall() -> anyhow::Result<String> {
    let path = unit_path();
    let _ = systemctl(&["disable", "--now", "kari-node.service"]);
    match std::fs::remove_file(&path) {
        Ok(()) => {
            let _ = systemctl(&["daemon-reload"]);
            Ok(format!(
                "the node no longer runs at login: {} is removed",
                path.display()
            ))
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            Ok("the node was not installed as a service".into())
        }
        Err(e) => Err(e.into()),
    }
}

#[cfg(target_os = "linux")]
pub fn status() -> anyhow::Result<String> {
    let path = unit_path();
    if !path.exists() {
        return Ok("not installed".into());
    }
    let out = Command::new("systemctl")
        .args(["--user", "is-active", "kari-node.service"])
        .output()?;
    let state = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(format!("installed at {}, {state}", path.display()))
}

// ----------------------------------------------------------------- Windows

// Windows has no per-user supervisor with the shape of the two above. A
// scheduled task with an "at log on" trigger is the nearest thing, and it needs
// its own XML and its own thinking about the restart the node depends on. Until
// that is written, say so plainly rather than write something that looks
// installed and is not.

#[cfg(windows)]
fn install_for_platform(program: &Path, opt: &Options) -> anyhow::Result<String> {
    // The command is in the message, because the person now has to do by hand
    // what the other two platforms do for them, and the flags they passed are
    // part of it.
    anyhow::bail!(
        "kari does not install a Windows service yet. Run this from a scheduled task \
         with an at-log-on trigger, and set it to restart when it exits:\n  {} {}",
        program.display(),
        args(opt).join(" ")
    )
}

#[cfg(windows)]
pub fn uninstall() -> anyhow::Result<String> {
    anyhow::bail!("kari does not install a Windows service yet")
}

#[cfg(windows)]
pub fn status() -> anyhow::Result<String> {
    Ok("not supported on Windows yet".into())
}
