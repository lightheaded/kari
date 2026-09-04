//! Start one tiny background job and follow it to the end.
//! `cargo run -p kari-core --example jobrun -- <cwd> [model]`
//!
//! The example adds a temporary card, starts it, prints every state change and
//! the run log, then deletes the card.

use kari_core::model::*;

fn main() -> anyhow::Result<()> {
    let cwd = std::env::args()
        .nth(1)
        .unwrap_or_else(|| std::env::temp_dir().to_string_lossy().into_owned());
    let model = std::env::args().nth(2);
    let engine = kari_core::Engine::open()?;
    engine.refresh_all();

    let card = engine.add_task(NewTask {
        title: "kari job probe".into(),
        project_cwd: Some(cwd.clone()),
        run_prompt: Some("Reply with the single word ok. Use no tools.".into()),
        auto_run: false,
        priority: 0,
        notes: Some("temporary card from the jobrun example".into()),
        model: model.clone(),
        column_id: None,
    })?;
    println!("model: {model:?}");
    println!(
        "card {} in {cwd}",
        card.id.chars().take(8).collect::<String>()
    );

    let job = engine.start_card(&card.id, None)?;
    println!("job {job}");

    let mut last = String::new();
    for i in 0..60 {
        std::thread::sleep(std::time::Duration::from_secs(3));
        engine.scan_jobs();
        let board = engine.board();
        let Some(cv) = board.cards.iter().find(|c| c.card.id == card.id) else {
            continue;
        };
        let state = format!(
            "{:?} / job {:?} / remembered {:?} / {}",
            cv.state,
            cv.bg_job.as_ref().and_then(|j| j.state.clone()),
            cv.card.last_job_state,
            cv.reason
        );
        if state != last {
            println!("[{:>3}s] {state}", i * 3);
            last = state;
        }
        let terminal = matches!(
            cv.card.last_job_state.as_deref(),
            Some("done") | Some("failed") | Some("stopped")
        );
        if terminal {
            break;
        }
    }

    println!("\nrun log:");
    for l in engine.job_log(&card.id, 20) {
        println!("  {} {:?} {:?}", l.at.to_rfc3339(), l.state, l.detail);
    }

    engine.delete_card(&card.id)?;
    println!("card deleted");
    Ok(())
}
