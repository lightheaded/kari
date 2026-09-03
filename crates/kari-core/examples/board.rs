//! Smoke test: scan real local data and print the board as text.
//! Run: cargo run -p kari-core --example board

use kari_core::Engine;
use std::collections::BTreeMap;

fn main() -> anyhow::Result<()> {
    tracing_subscriber_init();
    let engine = Engine::open()?;
    let t = std::time::Instant::now();
    engine.refresh_all();
    eprintln!("refresh took {:?}", t.elapsed());
    let board = engine.board();
    if std::env::args().any(|a| a == "--json") {
        println!("{}", serde_json::to_string_pretty(&board)?);
        return Ok(());
    }
    let mut by_col: BTreeMap<(i32, String), Vec<String>> = BTreeMap::new();
    for col in &board.columns {
        by_col.entry((col.order, col.name.clone())).or_default();
    }
    for c in &board.cards {
        let col = board.columns.iter().find(|k| k.id == c.column_id).unwrap();
        let live = c
            .live
            .as_ref()
            .map(|l| l.status.clone().unwrap_or_default())
            .unwrap_or_else(|| "-".into());
        let herdr = c
            .herdr
            .as_ref()
            .map(|h| h.pane_id.clone())
            .unwrap_or_else(|| "-".into());
        let tok = c
            .session
            .as_ref()
            .map(|s| s.tokens.weighted() / 1e6)
            .unwrap_or(0.0);
        let proj = c.project_name.clone().unwrap_or_default();
        by_col
            .entry((col.order, col.name.clone()))
            .or_default()
            .push(format!(
                "  [{:?}] {:<52} {:<22} live={:<5} herdr={:<6} {:>6.2}M  {}{}",
                c.state,
                kari_core::truncate(&c.title, 50),
                kari_core::truncate(&proj, 20),
                live,
                herdr,
                tok,
                c.reason,
                if c.locked { " (locked)" } else { "" }
            ));
    }
    for ((_, name), rows) in by_col {
        println!("== {name} ({})", rows.len());
        for r in rows {
            println!("{r}");
        }
    }
    println!("quota: {:?}", board.quota);
    println!("herdr connected: {}", board.herdr_connected);
    Ok(())
}

fn tracing_subscriber_init() {}
