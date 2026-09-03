//! Exercise the planner against the real board without starting anything.
//! `cargo run -p kari-core --example plan`
//!
//! The example adds two temporary task cards, asks for a plan, prints it, then
//! dismisses the plan and deletes the cards.

use kari_core::model::*;

fn main() -> anyhow::Result<()> {
    // With --accept the example also starts the plan, follows it, and stops it.
    let accept = std::env::args().any(|a| a == "--accept");
    let engine = kari_core::Engine::open()?;
    engine.refresh_all();
    engine.refresh_calibration();

    let board = engine.board();
    // A directory can be given, so a probe never runs inside a real project.
    let cwd = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());
    println!(
        "quota: {:?}",
        board
            .quota
            .as_ref()
            .and_then(|q| q.five_hour.as_ref())
            .map(|w| w.used_percentage)
    );
    println!(
        "weekly: {:?}",
        board
            .quota
            .as_ref()
            .and_then(|q| q.seven_day.as_ref())
            .map(|w| w.used_percentage)
    );
    println!(
        "calibration: {:.3} pct/Mtok ({})",
        board.calibration.pct_per_mtok, board.calibration.source
    );

    let mut temp: Vec<String> = vec![];
    for (title, prio) in [("kari planner probe A", 5), ("kari planner probe B", 0)] {
        let c = engine.add_task(NewTask {
            title: title.into(),
            project_cwd: Some(cwd.clone()),
            run_prompt: Some("Print the word ok and stop.".into()),
            auto_run: true,
            priority: prio,
            notes: Some("temporary card from the plan example".into()),
            model: None,
        })?;
        temp.push(c.id);
    }

    let result = engine.propose_now();
    match &result {
        Ok(p) => {
            println!(
                "\nproposal {} trigger={:?}",
                p.id.chars().take(8).collect::<String>(),
                p.trigger
            );
            println!("reason: {}", p.reason);
            println!(
                "budget {:.1}% · plan {:.1}% · window {:.1}% -> {:.1}% · skipped {}",
                p.budget_pct, p.total_pct, p.used_pct_before, p.used_pct_after, p.skipped
            );
            for i in &p.items {
                println!(
                    "  {} {:>6.2}%  {}  ({}){}",
                    if i.fits { "+" } else { "-" },
                    i.estimate.pct_five_hour,
                    i.title,
                    i.estimate.source,
                    i.skip_reason
                        .as_deref()
                        .map(|r| format!("  did not fit: {r}"))
                        .unwrap_or_default()
                );
            }
        }
        Err(e) => println!("\nno plan: {e}"),
    }

    if accept {
        if let Ok(p) = &result {
            let started = engine.accept_proposal(&p.id, None, false)?;
            println!("\nstarted {started} job(s)");
            for i in 0..40 {
                std::thread::sleep(std::time::Duration::from_secs(3));
                engine.scan_jobs();
                let board = engine.board();
                let mine: Vec<String> = p
                    .items
                    .iter()
                    .filter_map(|it| board.cards.iter().find(|c| c.card.id == it.card_id))
                    .map(|c| format!("{:?}/{:?}", c.state, c.card.last_job_state))
                    .collect();
                println!("[{:>3}s] {}", i * 3, mine.join("  "));
                let all_done = p.items.iter().all(|it| {
                    board
                        .cards
                        .iter()
                        .find(|c| c.card.id == it.card_id)
                        .is_none_or(|c| {
                            matches!(
                                c.card.last_job_state.as_deref(),
                                Some("done") | Some("failed") | Some("stopped")
                            )
                        })
                });
                if all_done {
                    break;
                }
            }
            let stopped = engine.stop_proposal(&p.id)?;
            println!("stop_proposal stopped {stopped} still-running job(s)");
        }
    }

    // Clean up whatever the example created.
    if let Ok(p) = &result {
        engine.dismiss_proposal(&p.id)?;
        println!("\nproposal dismissed");
    }
    for id in &temp {
        engine.delete_card(id)?;
    }
    println!("{} temporary cards deleted", temp.len());
    Ok(())
}
