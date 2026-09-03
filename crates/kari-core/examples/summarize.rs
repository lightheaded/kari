//! Summarize one session with Haiku. Costs one call.
//! Run: cargo run -p kari-core --example summarize [-- <session-id-prefix>]

use kari_core::{summary, Engine};

fn main() -> anyhow::Result<()> {
    let engine = Engine::open()?;
    engine.refresh_all();
    let board = engine.board();
    let prefix = std::env::args().nth(1);
    let mut cards: Vec<_> = board
        .cards
        .iter()
        .filter(|c| c.session.as_ref().is_some_and(|s| s.turns > 0))
        .collect();
    cards.sort_by_key(|c| std::cmp::Reverse(c.last_activity_at));
    let cv = cards
        .into_iter()
        .find(|c| {
            prefix.as_ref().is_none_or(|p| {
                c.card
                    .session_id
                    .as_deref()
                    .unwrap_or("")
                    .starts_with(p.as_str())
            })
        })
        .ok_or_else(|| anyhow::anyhow!("no session matches"))?;
    let facts = cv.session.as_ref().unwrap();
    println!(
        "session {} · {}",
        facts.session_id.chars().take(8).collect::<String>(),
        cv.title
    );
    let excerpt = summary::excerpt(std::path::Path::new(&facts.transcript_path), 30)?;
    println!(
        "excerpt: {} bytes, {} lines",
        excerpt.len(),
        excerpt.lines().count()
    );
    let t = std::time::Instant::now();
    let s = summary::generate(facts, &engine.settings().summary_model)?;
    println!("took {:?}", t.elapsed());
    println!("{}", serde_json::to_string_pretty(&s)?);
    Ok(())
}
