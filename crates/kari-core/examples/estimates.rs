//! Print the calibration and the task estimates kari would use right now.
//! `cargo run -p kari-core --example estimates`

fn main() -> anyhow::Result<()> {
    let engine = kari_core::Engine::open()?;
    engine.refresh_calibration();
    let c = engine.calibration();
    println!(
        "calibration: {:.3} pct/Mtok (band {:.3} to {:.3}) source={} pairs={}",
        c.pct_per_mtok, c.low, c.high, c.source, c.samples
    );
    let board = engine.board();
    let mut rows: Vec<_> = board
        .cards
        .iter()
        .filter_map(|cv| {
            cv.estimate
                .as_ref()
                .map(|e| (cv.title.clone(), cv.card.kind, e.clone()))
        })
        .collect();
    rows.sort_by(|a, b| b.2.pct_five_hour.partial_cmp(&a.2.pct_five_hour).unwrap());
    println!("{} cards", rows.len());
    for (title, kind, e) in rows.iter().take(12) {
        println!(
            "  {:>6.2}% ({:>5.2}-{:>5.2}) {:>7.1}M {:<8} {:?} {}",
            e.pct_five_hour,
            e.pct_low,
            e.pct_high,
            e.weighted_tokens / 1e6,
            e.source,
            kind,
            kari_core::truncate(title, 48)
        );
    }
    Ok(())
}
