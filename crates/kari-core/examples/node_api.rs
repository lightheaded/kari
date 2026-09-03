//! Prove the node API and its client agree, against the real local engine.
//!
//! Run: `cargo run -p kari-core --example node_api`
//!
//! The example serves the engine of this machine on a free loopback port, then
//! asks it the questions the hub asks a remote node: health, board, columns,
//! settings, projects, and one event from the stream. It changes nothing.

use kari_core::client::{ApiClient, EventItem};
use kari_core::{api, tunnel, Engine};
use std::sync::Arc;

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=warn")
        .init();
    let engine = Engine::open()?;
    let port = tunnel::free_port()?;
    let token = kari_core::hooks::token()?;
    println!("serving the local engine on 127.0.0.1:{port}");

    let rt = tokio::runtime::Runtime::new()?;
    let e = Arc::clone(&engine);
    rt.spawn(async move {
        let addr = std::net::SocketAddr::from(([127, 0, 0, 1], port));
        if let Err(e) = api::serve(e, addr, false).await {
            eprintln!("server: {e}");
        }
    });
    std::thread::sleep(std::time::Duration::from_millis(400));

    let client = ApiClient::new(port, &token);
    let id = client.health()?;
    println!(
        "health      node {} ({}) v{} api v{} on {}",
        id.node_name, id.node_id, id.version, id.api_version, id.platform
    );

    let board = client.board()?;
    println!(
        "board       {} cards, {} columns, quota {}, herdr {}",
        board.cards.len(),
        board.columns.len(),
        board.quota.is_some(),
        board.herdr_connected
    );
    println!("columns     {}", client.columns()?.len());
    let s = client.settings()?;
    println!(
        "settings    node_name {:?}, hooks port {}, usage endpoint {}",
        s.node_name, s.hooks_port, s.usage_endpoint_enabled
    );
    println!("projects    {}", client.projects()?.len());

    // A wrong token must be refused, or the node is open to every local process.
    let bad = ApiClient::new(port, "not-the-token");
    match bad.board() {
        Err(e) => println!("bad token   refused: {e}"),
        Ok(_) => anyhow::bail!("a wrong token read the board"),
    }

    // The stream sends a board_changed after a refresh, and keepalives when idle.
    let mut events = client.events()?;
    client.refresh()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
    loop {
        if std::time::Instant::now() > deadline {
            println!("events      no event in 30 s (an idle board sends none)");
            break;
        }
        match events.recv() {
            Some(EventItem::Message(m)) => {
                println!("events      {} {}", m.event, m.data);
                break;
            }
            Some(EventItem::KeepAlive) => println!("events      keepalive"),
            None => {
                println!("events      stream ended");
                break;
            }
        }
    }
    Ok(())
}
