//! Where the desktop app keeps the token of each remote node.
//!
//! macOS: the login keychain, one generic password per node under the service
//! `kari-node`. Other platforms: a file with mode 0600 under the kari directory.
//! The token never goes into the database.

use crate::paths;
use std::process::{Command, Stdio};

const SERVICE: &str = "kari-node";

fn token_file(node_id: &str) -> std::path::PathBuf {
    paths::kari_dir()
        .join("nodes")
        .join(format!("{node_id}.token"))
}

fn security(args: &[&str]) -> anyhow::Result<std::process::Output> {
    Ok(Command::new("/usr/bin/security")
        .args(args)
        .stdin(Stdio::null())
        .output()?)
}

pub fn store_token(node_id: &str, token: &str) -> anyhow::Result<()> {
    if cfg!(target_os = "macos") {
        // `-U` updates an item that exists. The value passes as an argument,
        // which the process list shows for a moment; `security` reads no stdin.
        let out = security(&[
            "add-generic-password",
            "-U",
            "-s",
            SERVICE,
            "-a",
            node_id,
            "-w",
            token,
        ])?;
        if !out.status.success() {
            anyhow::bail!(
                "keychain write failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            );
        }
        return Ok(());
    }
    let p = token_file(node_id);
    std::fs::create_dir_all(p.parent().unwrap())?;
    std::fs::write(&p, token)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

pub fn load_token(node_id: &str) -> Option<String> {
    if cfg!(target_os = "macos") {
        let out = security(&["find-generic-password", "-s", SERVICE, "-a", node_id, "-w"]).ok()?;
        if !out.status.success() {
            return None;
        }
        let t = String::from_utf8_lossy(&out.stdout).trim().to_string();
        return if t.is_empty() { None } else { Some(t) };
    }
    std::fs::read_to_string(token_file(node_id))
        .ok()
        .map(|t| t.trim().to_string())
        .filter(|t| !t.is_empty())
}

pub fn delete_token(node_id: &str) {
    if cfg!(target_os = "macos") {
        let _ = security(&["delete-generic-password", "-s", SERVICE, "-a", node_id]);
        return;
    }
    let _ = std::fs::remove_file(token_file(node_id));
}
