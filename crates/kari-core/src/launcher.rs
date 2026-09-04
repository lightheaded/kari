//! Open sessions in a terminal, focus herdr panes, start background jobs.

use crate::model::HerdrAgent;
use crate::paths;
use std::process::{Command, Stdio};

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(target_os = "macos")]
fn osascript(script: &str) -> anyhow::Result<()> {
    let out = Command::new("/usr/bin/osascript")
        .arg("-e")
        .arg(script)
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        anyhow::bail!("osascript failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Open a new terminal window in `cwd` and run `command` there.
#[cfg(not(target_os = "macos"))]
pub fn open_in_terminal(terminal_app: &str, cwd: &str, command: &str) -> anyhow::Result<()> {
    let _ = (terminal_app, cwd, command);
    anyhow::bail!("this node has no terminal driver; jump in from the desktop app")
}

/// Open a new terminal window in `cwd` and run `command` there.
#[cfg(target_os = "macos")]
pub fn open_in_terminal(terminal_app: &str, cwd: &str, command: &str) -> anyhow::Result<()> {
    let shell_line = format!("cd {} && {}", sh_quote(cwd), command);
    let line = applescript_string(&shell_line);
    match terminal_app {
        "iTerm" | "iTerm2" => osascript(&format!(
            "tell application \"iTerm\"\n activate\n set w to (create window with default profile)\n tell current session of w to write text {line}\nend tell"
        )),
        "Ghostty" => osascript(&format!(
            "tell application \"Ghostty\" to activate\ndelay 0.3\ntell application \"System Events\" to keystroke \"n\" using command down\ndelay 0.4\ntell application \"System Events\" to keystroke {line}\ntell application \"System Events\" to key code 36"
        )),
        _ => osascript(&format!("tell application \"Terminal\"\n activate\n do script {line}\nend tell")),
    }
}

pub fn resume_command(session_id: &str, model: Option<&str>) -> String {
    format!(
        "claude --resume {}{}",
        sh_quote(session_id),
        model_flag(model)
    )
}

/// A new session in the current directory.
pub fn new_command(model: Option<&str>) -> String {
    format!("claude{}", model_flag(model))
}

fn model_flag(model: Option<&str>) -> String {
    match model.map(str::trim).filter(|m| !m.is_empty()) {
        Some(m) => format!(" --model {}", sh_quote(m)),
        None => String::new(),
    }
}

pub fn attach_command(job_id: &str) -> String {
    format!("claude attach {}", sh_quote(job_id))
}

pub fn focus_herdr(agent: &HerdrAgent, terminal_app: &str) -> anyhow::Result<()> {
    crate::herdr::focus(agent)?;
    raise_terminal(terminal_app);
    Ok(())
}

/// Bring the configured terminal to the front, best effort. herdr runs inside it.
#[cfg(target_os = "macos")]
pub fn raise_terminal(terminal_app: &str) {
    let app = match terminal_app {
        "iTerm2" => "iTerm",
        other => other,
    };
    let _ = osascript(&format!(
        "tell application {} to activate",
        applescript_string(app)
    ));
}

#[cfg(not(target_os = "macos"))]
pub fn raise_terminal(_terminal_app: &str) {}

/// The command that runs a node's jump plan from another machine: a login
/// shell over SSH, so the remote PATH holds `claude`.
pub fn ssh_command(ssh_host: &str, cwd: &str, command: &str) -> String {
    let remote = format!("cd {} && {}", sh_quote(cwd), command);
    format!(
        "ssh -t {} -- sh -lc {}",
        sh_quote(ssh_host),
        sh_quote(&remote)
    )
}

/// Attach to a remote herdr server over SSH. The node already focused the
/// pane, so the attached client opens on it.
pub fn herdr_remote_command(ssh_host: &str) -> anyhow::Result<String> {
    let herdr = paths::which("herdr").ok_or_else(|| {
        anyhow::anyhow!(
            "herdr is not on PATH here, so kari cannot attach to the pane on {ssh_host}"
        )
    })?;
    Ok(format!(
        "{} --remote {}",
        sh_quote(&herdr.to_string_lossy()),
        sh_quote(ssh_host)
    ))
}

/// A login shell on a remote node, in `cwd`. The last resort when a node
/// returns no command and no pane.
pub fn ssh_shell_command(ssh_host: &str, cwd: &str) -> String {
    let remote = format!("cd {} && exec \"$SHELL\" -l", sh_quote(cwd));
    format!(
        "ssh -t {} -- sh -lc {}",
        sh_quote(ssh_host),
        sh_quote(&remote)
    )
}

pub struct BgStart {
    pub job_id: String,
    pub raw: String,
}

/// `claude --bg [--resume <id>] [--model <model>] --permission-mode <mode> --name <name> -- "<prompt>"` in `cwd`.
pub fn start_background(
    cwd: &str,
    prompt: &str,
    name: Option<&str>,
    permission_mode: &str,
    resume: Option<&str>,
    model: Option<&str>,
) -> anyhow::Result<BgStart> {
    let claude =
        paths::which("claude").ok_or_else(|| anyhow::anyhow!("claude not found on PATH"))?;
    let mut cmd = Command::new(claude);
    cmd.current_dir(cwd)
        .env("PATH", paths::child_path())
        .arg("--bg");
    if let Some(r) = resume {
        cmd.args(["--resume", r]);
    }
    cmd.args(["--permission-mode", permission_mode]);
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        cmd.args(["--model", m]);
    }
    if let Some(n) = name {
        cmd.args(["--name", n]);
    }
    // `--` ends the options. A prompt that starts with `-` stays a prompt.
    cmd.arg("--").arg(prompt);
    // The job id comes from stdout. Colour codes in it would break every later lookup.
    cmd.env("NO_COLOR", "1")
        .env_remove("FORCE_COLOR")
        .env_remove("CLICOLOR_FORCE");
    let out = cmd.stdin(Stdio::null()).output()?;
    let stdout = strip_ansi(&String::from_utf8_lossy(&out.stdout));
    let stderr = strip_ansi(&String::from_utf8_lossy(&out.stderr));
    if !out.status.success() {
        anyhow::bail!("claude --bg failed: {stderr} {stdout}");
    }
    let job_id = parse_job_id(&stdout)
        .ok_or_else(|| anyhow::anyhow!("could not read job id from: {stdout}"))?;
    Ok(BgStart {
        job_id,
        raw: stdout,
    })
}

/// The job id in the `claude --bg` output: "backgrounded · 7c5dcf5d · flaky-test-fix".
fn parse_job_id(stdout: &str) -> Option<String> {
    stdout.lines().find_map(|l| {
        let l = l.trim();
        if !l.starts_with("backgrounded") {
            return None;
        }
        l.split(['·', ' '])
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .nth(1)
            .map(|s| s.to_string())
    })
}

/// Remove ANSI escape sequences: colours, cursor moves, OSC hyperlinks.
pub fn strip_ansi(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if c != '\x1b' {
            out.push(c);
            continue;
        }
        match chars.next() {
            // CSI: ESC [ <params> <final byte 0x40..=0x7e>
            Some('[') => {
                for c in chars.by_ref() {
                    if ('\x40'..='\x7e').contains(&c) {
                        break;
                    }
                }
            }
            // OSC: ESC ] ... BEL or ESC \
            Some(']') => {
                while let Some(c) = chars.next() {
                    if c == '\x07' {
                        break;
                    }
                    if c == '\x1b' && chars.peek() == Some(&'\\') {
                        chars.next();
                        break;
                    }
                }
            }
            // Two-byte escapes such as ESC ( B. A lone ESC at the end is dropped.
            Some(_) | None => {}
        }
    }
    out
}

pub fn stop_background(job_id: &str) -> anyhow::Result<()> {
    let claude =
        paths::which("claude").ok_or_else(|| anyhow::anyhow!("claude not found on PATH"))?;
    let out = Command::new(claude)
        .args(["stop", job_id])
        .env("PATH", paths::child_path())
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        anyhow::bail!(
            "claude stop failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}

pub fn slugify(s: &str) -> String {
    let mut out = String::new();
    let mut dash = false;
    for c in s.chars().flat_map(|c| c.to_lowercase()) {
        if c.is_ascii_alphanumeric() {
            out.push(c);
            dash = false;
        } else if !dash && !out.is_empty() {
            out.push('-');
            dash = true;
        }
        if out.len() >= 28 {
            break;
        }
    }
    let out = out.trim_end_matches('-').to_string();
    if out.is_empty() {
        "kari-task".into()
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn job_id_plain() {
        assert_eq!(
            parse_job_id("backgrounded · 7c5dcf5d · flaky-test-fix\n").as_deref(),
            Some("7c5dcf5d")
        );
    }

    #[test]
    fn job_id_coloured() {
        let raw =
            "\x1b[2mbackgrounded\x1b[22m · \x1b[36m4f678c99\x1b[39m · kari-plan-view-overhaul\n";
        assert_eq!(parse_job_id(&strip_ansi(raw)).as_deref(), Some("4f678c99"));
    }

    #[test]
    fn ssh_command_wraps_a_login_shell() {
        let c = ssh_command("box", "/srv/repo", "claude --resume 'abc'");
        assert!(c.starts_with("ssh -t 'box' -- sh -lc '"));
        assert!(c.contains("cd '\\''/srv/repo'\\'' && claude"));
        assert!(c.ends_with("'"));
    }

    #[test]
    fn ssh_shell_command_lands_in_the_project() {
        let c = ssh_shell_command("box", "/srv/repo");
        assert!(c.starts_with("ssh -t 'box' -- sh -lc '"));
        assert!(c.contains(r"cd '\''/srv/repo'\'' && exec"));
    }

    #[test]
    fn herdr_remote_command_names_the_host() {
        // The helper needs herdr on PATH. Skip where it is absent.
        if paths::which("herdr").is_none() {
            return;
        }
        let c = herdr_remote_command("box").unwrap();
        assert!(c.ends_with("--remote 'box'"));
    }

    #[test]
    fn strip_ansi_keeps_text() {
        assert_eq!(strip_ansi("plain"), "plain");
        assert_eq!(strip_ansi("\x1b]8;;https://x\x07link\x1b]8;;\x07"), "link");
        assert_eq!(strip_ansi("a\x1b[1;31mb\x1b[0mc"), "abc");
    }
}
