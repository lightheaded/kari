//! Set the default permission mode of a real node, and read it back from the
//! board the node serves.
//!
//! Run: `cargo run -p kari-core --example default_mode`
//!
//! A card that names no mode runs under the default of the node that runs it.
//! The desktop Settings writes that default to every node, and the drawer reads
//! it back from the board of the card's node. This example checks both legs
//! against a `kari-node serve` child with its own home directory, so it touches
//! neither your store nor your keychain. It needs the node binary:
//!
//!   cargo build -p kari-cli

use kari_core::client::ApiClient;
use kari_core::tunnel;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Node {
    child: Child,
    dir: std::path::PathBuf,
}

impl Drop for Node {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn main() -> anyhow::Result<()> {
    let bin = std::path::Path::new("target/debug/kari-node");
    if !bin.exists() {
        anyhow::bail!("build the node first: cargo build -p kari-cli");
    }
    let port = tunnel::free_port()?;
    let home = std::env::temp_dir().join(format!("kari-mode-example-{port}"));
    std::fs::create_dir_all(home.join(".claude/projects"))?;
    let child = Command::new(bin)
        .args([
            "serve",
            "--listen",
            &format!("127.0.0.1:{port}"),
            "--name",
            "mode-node",
            "--summaries",
            "false",
        ])
        .env("HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let _node = Node {
        child,
        dir: home.clone(),
    };

    let token_file = home.join(".config/kari/hook-token");
    let deadline = Instant::now() + Duration::from_secs(30);
    let client = loop {
        if let Ok(t) = std::fs::read_to_string(&token_file) {
            let c = ApiClient::at(&format!("http://127.0.0.1:{port}"), t.trim());
            if t.trim().len() >= 32 && c.settings().is_ok() {
                break c;
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node did not answer");
        }
        std::thread::sleep(Duration::from_millis(200));
    };

    let before = client.board()?.default_permission_mode;
    println!("before     the node runs a card with no mode under {before}");
    anyhow::ensure!(before == "auto", "a new node starts in {before}, not auto");

    client.set_default_permission_mode("bypassPermissions")?;
    let settings = client.settings()?;
    anyhow::ensure!(
        settings.default_permission_mode == "bypassPermissions",
        "the node kept {}",
        settings.default_permission_mode
    );
    let after = client.board()?.default_permission_mode;
    anyhow::ensure!(
        after == "bypassPermissions",
        "the board still says {after}"
    );
    println!("after      the node settings and its board both say {after}");
    Ok(())
}
