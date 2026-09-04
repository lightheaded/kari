//! Does `Hub::board()` answer while a node is unreachable?
//!
//! The phone shows one screen until the first board arrives. This example
//! builds a hub whose only node has an address that answers nothing, then asks
//! for the board once a second and prints how long each call took.
//!
//! Run: cargo run -p kari-core --example board_while_offline

use kari_core::hub::Hub;
use kari_core::model::NewNode;
use kari_core::Engine;
use std::sync::Arc;
use std::time::{Duration, Instant};

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=info")
        .init();
    let dir = std::env::temp_dir().join(format!("kari-offline-{}", std::process::id()));
    std::fs::create_dir_all(&dir)?;
    let engine = Engine::open_at(&dir)?;
    let hub = Hub::without_local(Arc::clone(&engine));

    // An address in a private range that routes nowhere here. Built from
    // octets, because this repository holds no address literal. The token is a
    // stand-in: the connection never gets far enough to use it.
    let nowhere = std::net::SocketAddr::new(
        std::net::IpAddr::V4(std::net::Ipv4Addr::new(10, 99, 99, 99)),
        47311,
    );
    hub.add_node(NewNode {
        name: "ghost".into(),
        address: Some(nowhere.to_string()),
        token: Some("00000000".into()),
        ..Default::default()
    })?;

    for i in 1..=20 {
        let t = Instant::now();
        let b = hub.board();
        let ms = t.elapsed().as_millis();
        println!(
            "{i:2}: board in {ms:>6} ms, {} node(s), {} card(s){}",
            b.nodes.len(),
            b.cards.len(),
            if ms > 1000 { "   <-- SLOW" } else { "" }
        );
        std::thread::sleep(Duration::from_secs(1));
    }
    println!("done; the store was in {}", dir.display());
    Ok(())
}
