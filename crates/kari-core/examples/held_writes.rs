//! Write to a node that is not there, then start it and watch the write land.
//!
//! Run: `cargo run -p kari-core --example held_writes`
//!
//! The example starts `kari-node serve` as a child process with its own home
//! directory, adds it to a hub whose store is a temporary directory, and then
//! kills the node. It adds a task and a note while the node is down, prints the
//! board that holds them, starts the node again, and prints the board the node
//! itself answers with. It needs the node binary:
//!
//!   cargo build -p kari-cli
//!
//! macOS asks once for permission when the hub reads the node token from the
//! keychain. The desktop app asks the same question the first time.

use kari_core::hub::Hub;
use kari_core::model::{CardPatch, NewNode, NewTask};
use kari_core::{tunnel, Engine};
use std::process::{Child, Command, Stdio};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Start `kari-node serve` on one port, with one home directory.
fn start(bin: &std::path::Path, home: &std::path::Path, port: u16) -> anyhow::Result<Child> {
    Ok(Command::new(bin)
        .args([
            "serve",
            "--listen",
            &format!("127.0.0.1:{port}"),
            "--name",
            "example-node",
            "--summaries",
            "false",
        ])
        .env("HOME", home)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?)
}

/// Wait until the hub agrees about the node, or give up.
fn wait_online(hub: &Arc<Hub>, id: &str, want: bool) -> anyhow::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        if hub.nodes().iter().any(|n| n.id == id && n.online == want) {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(250));
    }
    anyhow::bail!(
        "the node never went {}",
        if want { "online" } else { "offline" }
    )
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
    let home = std::env::temp_dir().join(format!("kari-held-example-{port}"));
    let hub_dir = home.join("hub");
    std::fs::create_dir_all(home.join(".claude/projects"))?;
    std::fs::create_dir_all(&hub_dir)?;
    println!("node home  {}", home.display());

    let mut child = start(bin, &home, port)?;

    // The hub keeps its own store here, so the example touches nothing of yours.
    let hub = Hub::without_local(Engine::open_at(&hub_dir)?);

    // The node writes its token on the first start. Read it and pair by hand.
    let token_file = home.join(".config/kari/hook-token");
    let deadline = Instant::now() + Duration::from_secs(30);
    let token = loop {
        if let Ok(t) = std::fs::read_to_string(&token_file) {
            break t.trim().to_string();
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            anyhow::bail!("the node wrote no token");
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let status = hub.add_node(NewNode {
        name: "example".into(),
        address: Some(format!("127.0.0.1:{port}")),
        token: Some(token),
        ..Default::default()
    })?;
    let id = status.id.clone();
    wait_online(&hub, &id, true)?;
    println!("online     {} cards on the board", hub.board().cards.len());

    // The machine goes away.
    child.kill()?;
    child.wait()?;
    wait_online(&hub, &id, false)?;
    println!("offline    the node is down");

    // A task and a note, written to a machine that is not there.
    let card = hub.add_task(
        &id,
        NewTask {
            title: "buy milk on the way back".into(),
            project_cwd: None,
            run_prompt: None,
            auto_run: false,
            priority: 0,
            notes: None,
            model: None,
            column_id: None,
        },
    )?;
    hub.patch_card(
        &id,
        &card.id,
        CardPatch {
            notes: Some("the shop shuts at six".into()),
            ..Default::default()
        },
    )?;
    let b = hub.board();
    let held = hub
        .nodes()
        .iter()
        .find(|n| n.id == id)
        .map(|n| n.pending_writes);
    println!(
        "written    {} card(s), {} waiting, pending flag {:?}",
        b.cards.len(),
        held.unwrap_or(0),
        b.cards.first().map(|c| c.pending)
    );

    // The machine comes back.
    let mut child = start(bin, &home, port)?;
    wait_online(&hub, &id, true)?;
    // The flush runs on connect. Give the board a moment to be read again.
    let deadline = Instant::now() + Duration::from_secs(30);
    loop {
        let b = hub.board();
        let mine = b.cards.iter().find(|c| c.view.card.id == card.id);
        if let Some(c) = mine.filter(|c| !c.pending) {
            println!(
                "landed     {:?} on the node, notes {:?}, {} waiting",
                c.view.title,
                c.view.card.notes,
                hub.nodes()
                    .iter()
                    .find(|n| n.id == id)
                    .map(|n| n.pending_writes)
                    .unwrap_or(0)
            );
            break;
        }
        if Instant::now() > deadline {
            let _ = child.kill();
            anyhow::bail!("the card never reached the node");
        }
        std::thread::sleep(Duration::from_millis(250));
    }

    hub.remove_node(&id)?;
    child.kill()?;
    child.wait()?;
    std::fs::remove_dir_all(&home)?;
    println!("done       node removed, token deleted, directory cleaned");
    Ok(())
}
