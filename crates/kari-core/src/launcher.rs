//! Open sessions in a terminal, focus herdr panes, start background jobs.

use crate::model::HerdrAgent;
use crate::paths;
use std::process::{Command, Stdio};

pub fn sh_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

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

fn applescript_string(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

/// Open a new terminal window in `cwd` and run `command` there.
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
    // herdr runs inside a terminal. Bring the configured one to front, best effort.
    let app = match terminal_app {
        "iTerm2" => "iTerm",
        other => other,
    };
    let _ = osascript(&format!(
        "tell application {} to activate",
        applescript_string(app)
    ));
    Ok(())
}

pub struct BgStart {
    pub job_id: String,
    pub raw: String,
}

/// `claude --bg [--resume <id>] [--model <model>] --permission-mode <mode> --name <name> "<prompt>"` in `cwd`.
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
    cmd.arg(prompt);
    let out = cmd.stdin(Stdio::null()).output()?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        anyhow::bail!("claude --bg failed: {stderr} {stdout}");
    }
    // "backgrounded · 7c5dcf5d · flaky-test-fix"
    let job_id = stdout
        .lines()
        .find_map(|l| {
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
        .ok_or_else(|| anyhow::anyhow!("could not read job id from: {stdout}"))?;
    Ok(BgStart {
        job_id,
        raw: stdout,
    })
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
