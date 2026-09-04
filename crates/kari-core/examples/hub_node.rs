//! Add a real node to the hub, read the merged board, then remove it again.
//!
//! Run: `cargo run -p kari-core --example hub_node`
//!
//! The example starts `kari-node serve` as a child process with its own home
//! directory, so the node has its own database and its own token and touches
//! nothing of yours. It then adds that node to the hub over a plain loopback
//! port instead of an SSH forward, waits for it to come online, prints the
//! board of both nodes, and removes the node again. It needs the node binary:
//!
//!   cargo build -p kari-cli
//!
//! macOS asks once for permission when the hub reads the node token from the
//! keychain. The desktop app asks the same question the first time.

use kari_core::hub::Hub;
use kari_core::{keychain, tunnel, Engine, NewNode};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
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
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=warn")
        .init();
    let bin = std::path::Path::new("target/debug/kari-node");
    if !bin.exists() {
        anyhow::bail!("build the node first: cargo build -p kari-cli");
    }
    let port = tunnel::free_port()?;
    let home = std::env::temp_dir().join(format!("kari-hub-example-{port}"));
    std::fs::create_dir_all(home.join(".claude/projects"))?;
    println!("node home  {}", home.display());
    let child = Command::new(bin)
        .args([
            "serve",
            "--listen",
            &format!("127.0.0.1:{port}"),
            "--name",
            "example-node",
            "--summaries",
            "false",
        ])
        .env("HOME", &home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    let node = Node {
        child,
        dir: home.clone(),
    };

    let engine = Engine::open()?;
    let hub = Hub::new(Arc::clone(&engine));
    println!("hub        local node {}", engine.node_name());

    // A node without an SSH host is paired by hand: read its token and store it.
    let status = hub.add_node(NewNode {
        name: "example".into(),
        ssh_host: None,
        remote_port: port,
        ..Default::default()
    })?;
    println!("added      {} (paired {})", status.name, status.paired);
    let token_file = home.join(".config/kari/hook-token");
    let deadline = Instant::now() + Duration::from_secs(30);
    let token = loop {
        if let Ok(t) = std::fs::read_to_string(&token_file) {
            if t.trim().len() >= 32 {
                break t.trim().to_string();
            }
        }
        if Instant::now() > deadline {
            anyhow::bail!("the node wrote no token file");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    keychain::store_token(&status.id, &token)?;
    println!("paired     token stored in the keychain");

    // The hub retries with a backoff, so the first attempt may have failed
    // before the token was there. Wait for the node to report itself online.
    let deadline = Instant::now() + Duration::from_secs(60);
    loop {
        let n = hub
            .nodes()
            .into_iter()
            .find(|n| n.id == status.id)
            .ok_or_else(|| anyhow::anyhow!("node gone"))?;
        if n.online {
            println!(
                "online     {} v{} api v{}",
                n.name,
                n.version.unwrap_or_default(),
                n.api_version.unwrap_or_default()
            );
            break;
        }
        if Instant::now() > deadline {
            anyhow::bail!("node stayed offline: {}", n.error.unwrap_or_default());
        }
        std::thread::sleep(Duration::from_millis(500));
    }

    let board = hub.board();
    println!(
        "board      {} cards over {} nodes, {} quota bars, {} plan(s)",
        board.cards.len(),
        board.nodes.len(),
        board.quotas.len(),
        board.proposals.len()
    );
    for n in &board.nodes {
        let mine = board.cards.iter().filter(|c| c.node_id == n.id).count();
        println!(
            "  node     {:<14} {:<7} {mine} cards",
            n.name,
            if n.online { "online" } else { "offline" }
        );
    }

    // A task on the remote node, then away again. It never touches this machine.
    let card = hub.add_task(
        &status.id,
        kari_core::NewTask {
            title: "hub example probe".into(),
            project_cwd: Some(home.to_string_lossy().into_owned()),
            run_prompt: None,
            auto_run: false,
            priority: 0,
            notes: None,
            model: None,
            column_id: None,
        },
    )?;
    let after = hub.board();
    let on_node = after
        .cards
        .iter()
        .filter(|c| c.node_id == status.id)
        .count();
    println!("task       added to the node, it now holds {on_node} card(s)");
    hub.delete_card(&status.id, &card.id)?;
    println!("task       removed");

    // The column lease. This hub takes it, a second hub takes it away, this hub
    // notices and steps back, then takes it again.
    println!("primary    {}", hub.claim_primary()?);
    let n = hub.nodes().into_iter().find(|n| n.id == status.id).unwrap();
    anyhow::ensure!(n.primary, "the node does not show this hub as primary");
    let other = kari_core::client::ApiClient::at(&format!("http://127.0.0.1:{port}"), &token)
        .with_hub("phone-example");
    let refused = other.set_columns(&engine.columns());
    anyhow::ensure!(
        refused
            .as_ref()
            .is_err_and(|e| e.to_string().contains("not primary")),
        "the node let a foreign hub push columns: {refused:?}"
    );
    println!("gate       a foreign hub got 409 on a column push");
    other.claim_lease(&kari_core::LeaseClaim {
        hub_id: "phone-example".into(),
        hub_name: "phone".into(),
        take: true,
    })?;
    let deadline = Instant::now() + Duration::from_secs(10);
    while hub.is_primary() {
        if Instant::now() > deadline {
            anyhow::bail!("the hub did not notice that the phone took the lease");
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    println!("lost       the phone took the lease and this hub stepped back");
    let e = hub.set_columns(engine.columns()).unwrap_err().to_string();
    anyhow::ensure!(e.contains("not primary"), "{e}");
    println!("follower   this hub refuses to edit columns: {e}");
    println!("primary    {}", hub.claim_primary()?);
    let lease = other
        .lease()?
        .ok_or_else(|| anyhow::anyhow!("no lease on the node"))?;
    anyhow::ensure!(
        lease.hub_id == engine.node_id(),
        "the node's lease is not this hub's"
    );
    println!("taken back the node's lease names this hub again");

    hub.remove_node(&status.id)?;
    println!("removed    node and its keychain item");
    drop(node);
    println!("stopped    the node process and its home directory is gone");
    Ok(())
}
