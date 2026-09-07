//! Start a server, link a real node to it, and read the node's board back
//! through the link.
//!
//! Run: `cargo run -p kari-core --example link_server`
//!
//! Needs both binaries: `cargo build -p kari-cli`.
//!
//! Nothing here touches your own kari. The server and the node each get their
//! own home directory, so they make their own databases and their own tokens
//! and both are deleted when the example ends. The point it proves is the one
//! the link exists for: the node opens the connection, and the server calls the
//! node's ordinary API back down it.

use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

struct Proc {
    child: Child,
    dir: std::path::PathBuf,
}

impl Drop for Proc {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn get(url: &str, token: &str) -> anyhow::Result<serde_json::Value> {
    let resp = reqwest::blocking::Client::builder()
        .timeout(Duration::from_secs(20))
        .build()?
        .get(url)
        .header(kari_core::hooks::TOKEN_HEADER, token)
        .send()?;
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    if !status.is_success() {
        anyhow::bail!("{status}: {text}");
    }
    Ok(serde_json::from_str(&text)?)
}

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=info")
        .init();

    let server_bin = std::path::Path::new("target/debug/kari-server");
    let node_bin = std::path::Path::new("target/debug/kari-node");
    for b in [server_bin, node_bin] {
        if !b.exists() {
            anyhow::bail!("build them first: cargo build -p kari-cli");
        }
    }

    let port = kari_core::tunnel::free_port()?;
    let node_port = kari_core::tunnel::free_port()?;

    // The server, with its own home so it makes its own token.
    let server_home = std::env::temp_dir().join(format!("kari-link-example-server-{port}"));
    std::fs::create_dir_all(&server_home)?;
    let server = Proc {
        child: Command::new(server_bin)
            .args(["serve", "--listen", &format!("127.0.0.1:{port}")])
            .env("HOME", &server_home)
            .env("XDG_CONFIG_HOME", server_home.join(".config"))
            .stdout(Stdio::null())
            .spawn()?,
        dir: server_home.clone(),
    };
    println!("server     http://127.0.0.1:{port}");

    // Its token, which the node must present. On a real host this is copied by
    // hand, or by whatever manages the node's configuration.
    let token_path = server_home.join(".config/kari/server-token");
    let deadline = Instant::now() + Duration::from_secs(20);
    let token = loop {
        if let Ok(t) = std::fs::read_to_string(&token_path) {
            if t.trim().len() >= 32 {
                break t.trim().to_string();
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the server never wrote its token");
        }
        std::thread::sleep(Duration::from_millis(100));
    };
    println!("token      {}…", &token[..8]);

    // The node, which dials the server rather than waiting to be dialled.
    let node_home = std::env::temp_dir().join(format!("kari-link-example-node-{node_port}"));
    std::fs::create_dir_all(node_home.join(".claude/projects"))?;
    let node = Proc {
        child: Command::new(node_bin)
            .args([
                "serve",
                "--listen",
                &format!("127.0.0.1:{node_port}"),
                "--name",
                "example-node",
                "--summaries",
                "false",
                "--server",
                &format!("http://127.0.0.1:{port}"),
            ])
            .env("HOME", &node_home)
            .env("XDG_CONFIG_HOME", node_home.join(".config"))
            .env("KARI_SERVER_TOKEN", &token)
            .stdout(Stdio::null())
            .spawn()?,
        dir: node_home.clone(),
    };

    // Wait for the roster to name it.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        if let Ok(v) = get(&format!("http://127.0.0.1:{port}/kari/v1/nodes"), &token) {
            if v.as_array()
                .is_some_and(|a| a.iter().any(|n| n["online"].as_bool().unwrap_or(false)))
            {
                println!("\nroster\n{v:#}");
                break;
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node never linked");
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    // The board, fetched by the server over the link the node opened.
    let board = get(&format!("http://127.0.0.1:{port}/kari/v1/board"), &token)?;
    let boards = board["boards"].as_array().cloned().unwrap_or_default();
    println!("\nboards fetched over the link: {}", boards.len());
    for b in &boards {
        let cards = b["board"]["cards"].as_array().map(|a| a.len()).unwrap_or(0);
        let cols = b["board"]["columns"]
            .as_array()
            .map(|a| a.len())
            .unwrap_or(0);
        println!(
            "  {}: {cols} columns, {cards} cards{}",
            b["node_name"].as_str().unwrap_or("?"),
            b["error"]
                .as_str()
                .map(|e| format!("  error: {e}"))
                .unwrap_or_default()
        );
    }
    if boards.is_empty() {
        anyhow::bail!("the server got no board over the link");
    }

    drop(node);
    drop(server);
    println!("\ndone");
    Ok(())
}
