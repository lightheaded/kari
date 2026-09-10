//! An SSH port forward to a remote node's loopback API, and the one SSH call
//! that pairs a node: reading its token.

use crate::paths;
use std::process::{Child, Command, Stdio};

/// A running `ssh -N -L` child. Dropping it ends the forward.
pub struct Tunnel {
    child: Child,
    pub local_port: u16,
}

fn ssh_bin() -> std::path::PathBuf {
    paths::which("ssh").unwrap_or_else(|| "/usr/bin/ssh".into())
}

/// The 1Password agent socket. macOS holds a group container under App Data
/// protection, so the first look at this path raises a system prompt: "kari
/// would like to access data from other apps". One prompt is the price of the
/// fallback. A prompt every reconnect is not, which is why the answer is kept
/// for the life of the process.
static AGENT_SOCK: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// A GUI app inherits no agent socket from a shell. When none is set, use the
/// 1Password agent socket if it exists. A plain key file needs no agent.
///
/// `KARI_SSH_AUTH_SOCK` names the socket directly. Set it to skip the search,
/// and set it to an empty value to turn the fallback off: a node that
/// authenticates with a key file, or an `IdentityAgent` line in
/// `~/.ssh/config`, needs neither the search nor the prompt.
fn agent_sock() -> Option<String> {
    if let Ok(s) = std::env::var("KARI_SSH_AUTH_SOCK") {
        return Some(s).filter(|s| !s.is_empty());
    }
    if let Ok(s) = std::env::var("SSH_AUTH_SOCK") {
        if !s.is_empty() {
            return Some(s);
        }
    }
    AGENT_SOCK
        .get_or_init(|| {
            let op = paths::home()
                .join("Library/Group Containers/2BUA8C4S2C.com.1password/t/agent.sock");
            op.exists().then(|| op.to_string_lossy().into_owned())
        })
        .clone()
}

fn ssh_command() -> Command {
    let mut c = Command::new(ssh_bin());
    crate::proc::quiet(&mut c);
    c.env("PATH", paths::child_path());
    if let Some(s) = agent_sock() {
        c.env("SSH_AUTH_SOCK", s);
    }
    c.args([
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "StrictHostKeyChecking=accept-new",
    ]);
    c
}

/// A free TCP port on loopback.
pub fn free_port() -> anyhow::Result<u16> {
    let l = std::net::TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

impl Tunnel {
    /// Start the forward. Returns before the connection is up; poll the node's
    /// health to know when it is.
    pub fn open(ssh_host: &str, remote_port: u16) -> anyhow::Result<Tunnel> {
        if cfg!(target_os = "android") {
            anyhow::bail!(
                "SSH forwards are not available on this device; give the node an address instead"
            );
        }
        let local_port = free_port()?;
        let child = ssh_command()
            .args([
                "-N",
                "-o",
                "ExitOnForwardFailure=yes",
                "-o",
                "ServerAliveInterval=15",
                "-o",
                "ServerAliveCountMax=3",
                "-L",
                &format!("127.0.0.1:{local_port}:127.0.0.1:{remote_port}"),
                "--",
                ssh_host,
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        Ok(Tunnel { child, local_port })
    }

    /// False once ssh exited. The error text, when there is one, is in `exit_message`.
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }

    /// What ssh said on the way out, for the node's status line.
    pub fn exit_message(&mut self) -> Option<String> {
        use std::io::Read;
        let mut s = String::new();
        if let Some(mut e) = self.child.stderr.take() {
            let _ = e.read_to_string(&mut s);
        }
        let s = s.trim().lines().last().unwrap_or("").to_string();
        if s.is_empty() {
            None
        } else {
            Some(s)
        }
    }
}

impl Drop for Tunnel {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// Read the node's hook token over SSH. That is the whole pairing step: SSH is
/// the authentication, the token then guards the loopback port on the node.
pub fn read_remote_token(ssh_host: &str) -> anyhow::Result<String> {
    if cfg!(target_os = "android") {
        anyhow::bail!("SSH is not available on this device; pair with a code instead");
    }
    let out = ssh_command()
        .args(["--", ssh_host, "cat ~/.config/kari/hook-token"])
        .stdin(Stdio::null())
        .output()?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        anyhow::bail!(
            "ssh {ssh_host} failed: {}",
            if err.is_empty() {
                "no token file; has kari-node run there once?".into()
            } else {
                err
            }
        );
    }
    let tok = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if tok.len() < 32 {
        anyhow::bail!("the node's token file is empty; start kari-node there once");
    }
    Ok(tok)
}
