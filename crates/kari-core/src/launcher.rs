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

/// The commands above as argument lists.
///
/// macOS hands a terminal one line and lets a shell parse it, which is what
/// `sh_quote` is for. Windows has no `sh` in the middle: the program is
/// started with an argument list and the quoting is done once, by the caller
/// that builds the process. The two forms are built side by side so that a
/// change to one is a change to the other.
fn push_model(argv: &mut Vec<String>, model: Option<&str>) {
    if let Some(m) = model.map(str::trim).filter(|m| !m.is_empty()) {
        argv.push("--model".into());
        argv.push(m.into());
    }
}

pub fn resume_argv(session_id: &str, model: Option<&str>) -> Vec<String> {
    let mut argv = vec!["claude".into(), "--resume".into(), session_id.into()];
    push_model(&mut argv, model);
    argv
}

pub fn new_argv(model: Option<&str>) -> Vec<String> {
    let mut argv = vec!["claude".to_string()];
    push_model(&mut argv, model);
    argv
}

pub fn attach_argv(job_id: &str) -> Vec<String> {
    vec!["claude".into(), "attach".into(), job_id.into()]
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

/// A jump onto another machine, as an argument list. Only the local half
/// changes: the remote half stays one `sh`-quoted line, because there really
/// is a shell at the far end to parse it.
fn ssh_argv_for(ssh_host: &str, remote: String) -> Vec<String> {
    vec![
        "ssh".into(),
        "-t".into(),
        ssh_host.into(),
        "--".into(),
        "sh".into(),
        "-lc".into(),
        remote,
    ]
}

pub fn ssh_argv(ssh_host: &str, cwd: &str, command: &str) -> Vec<String> {
    ssh_argv_for(ssh_host, format!("cd {} && {}", sh_quote(cwd), command))
}

pub fn ssh_shell_argv(ssh_host: &str, cwd: &str) -> Vec<String> {
    ssh_argv_for(
        ssh_host,
        format!("cd {} && exec \"$SHELL\" -l", sh_quote(cwd)),
    )
}

pub fn herdr_remote_argv(ssh_host: &str) -> anyhow::Result<Vec<String>> {
    let herdr = paths::which("herdr").ok_or_else(|| {
        anyhow::anyhow!(
            "herdr is not on PATH here, so kari cannot attach to the pane on {ssh_host}"
        )
    })?;
    Ok(vec![
        herdr.to_string_lossy().into_owned(),
        "--remote".into(),
        ssh_host.into(),
    ])
}

/// Open a terminal window in `cwd` and run `argv` there, on Windows.
///
/// The program is started with a console of its own, and nothing is asked to
/// parse a command line: `claude` is a real executable on Windows, so there is
/// no shell in the way and no second round of quoting to get wrong.
///
/// This ignores the terminal setting, which names macOS applications. Windows
/// routes a new console to whatever "Default terminal application" is set to,
/// which on Windows 11 is Windows Terminal — so the choice is already the
/// user's, made in one place for every program rather than here.
#[cfg(windows)]
pub fn open_in_terminal_argv(cwd: &str, argv: &[String]) -> anyhow::Result<()> {
    use std::os::windows::process::CommandExt;
    /// CREATE_NEW_CONSOLE. kari is a GUI process with no console of its own,
    /// so a child started without this has nowhere to draw and exits at once.
    const CREATE_NEW_CONSOLE: u32 = 0x0000_0010;

    let (program, args) = argv
        .split_first()
        .ok_or_else(|| anyhow::anyhow!("nothing to run"))?;
    let exe = paths::which(program).unwrap_or_else(|| std::path::PathBuf::from(program));
    // Claude Code from npm is `claude.cmd`, a batch file, and CreateProcess
    // cannot start one of those: it needs `cmd.exe` to read it. The native
    // installer leaves a real `claude.exe`, which starts directly.
    let batch = exe
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("cmd") || e.eq_ignore_ascii_case("bat"));
    let mut cmd = if batch {
        let mut c = Command::new("cmd.exe");
        c.arg("/c").arg(&exe);
        c
    } else {
        Command::new(&exe)
    };
    cmd.args(args)
        .current_dir(cwd)
        .env("PATH", paths::child_path())
        .creation_flags(CREATE_NEW_CONSOLE)
        .stdin(Stdio::null())
        .spawn()?;
    Ok(())
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
    crate::proc::quiet(&mut cmd);
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
    let mut cmd = Command::new(claude);
    crate::proc::quiet(&mut cmd);
    let out = cmd
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
