//! Prove that a file put on a card reaches a run, over the real node API.
//!
//! Run: `cargo run -p kari-core --example attachments`
//!
//! The example opens an engine in a temporary directory, so it touches nothing
//! of yours. It then serves that engine on a free loopback port and does what
//! a client does: add a card, put a file on it, read the file back, see the
//! file on the board, read the prompt a run would get, and take the file off
//! again. Last it proves the sweep: an archived card loses its files.

use kari_core::client::ApiClient;
use kari_core::model::{with_attachments, CardPatch, NewAttachment, NewTask};
use kari_core::{api, attach, paths, tunnel, Engine};
use std::sync::Arc;

/// A one-pixel PNG. Small enough to print, real enough to have a content type.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4E, 0x47, 0x0D, 0x0A, 0x1A, 0x0A, 0x00, 0x00, 0x00, 0x0D, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1F, 0x15, 0xC4,
    0x89,
];

fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter("kari_core=warn")
        .init();

    let dir = std::env::temp_dir().join(format!("kari-attachments-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir)?;
    let engine = Engine::open_at(&dir)?;
    println!("engine      a temporary one at {}", dir.display());

    let port = tunnel::free_port()?;
    let token = kari_core::hooks::token()?;
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

    // A card to hang the file on.
    let card = client.add_task(&NewTask {
        title: "Make the empty state read better".into(),
        project_cwd: None,
        run_prompt: Some("The screenshot shows the state to fix.".into()),
        auto_run: false,
        priority: 0,
        notes: None,
        model: None,
        column_id: None,
    })?;
    println!("card        {}", card.id);

    // The name carries a space and a slash on purpose. Both must be gone: the
    // name is the last segment of a URL that travels raw inside a link frame.
    let written = client.add_attachment(
        &card.id,
        &NewAttachment {
            name: "shots/Screen shot.png".into(),
            data_b64: attach::b64_encode(PNG),
        },
    )?;
    println!(
        "attached    {} ({}, {} bytes)\n            at {}",
        written.name, written.mime, written.bytes, written.path
    );
    anyhow::ensure!(
        !written.name.contains(' ') && !written.name.contains('/'),
        "the stored name still needs encoding: {}",
        written.name
    );
    anyhow::ensure!(written.mime == "image/png", "wrong content type");

    // A second file of the same name gets a number rather than overwriting.
    let second = client.add_attachment(
        &card.id,
        &NewAttachment {
            name: "shots/Screen shot.png".into(),
            data_b64: attach::b64_encode(PNG),
        },
    )?;
    anyhow::ensure!(
        second.name != written.name,
        "the second file overwrote the first"
    );
    println!("second      {} kept the first one", second.name);

    // The bytes come back whole.
    let got = client.attachment(&card.id, &written.name)?;
    anyhow::ensure!(
        attach::b64_decode(&got.data_b64)? == PNG,
        "the bytes changed on the way"
    );
    println!("read back   {} bytes, unchanged", PNG.len());

    // The board carries the list, so a card shows its paperclip with no extra call.
    let board = client.board()?;
    let view = board
        .cards
        .iter()
        .find(|c| c.card.id == card.id)
        .ok_or_else(|| anyhow::anyhow!("the card left the board"))?;
    anyhow::ensure!(view.attachments.len() == 2, "the board lost an attachment");
    println!(
        "board       {} attachments on the card",
        view.attachments.len()
    );

    // The prompt a run receives names every path. This is the whole feature:
    // Claude Code opens a file only when the prompt names it.
    let prompt = with_attachments("The screenshot shows the state to fix.", &view.attachments);
    println!("\nthe prompt a run would get\n---\n{prompt}\n---\n");
    for a in &view.attachments {
        anyhow::ensure!(prompt.contains(&a.path), "the prompt lost {}", a.name);
        anyhow::ensure!(
            std::path::Path::new(&a.path).is_file(),
            "the prompt names a path that is not a file: {}",
            a.path
        );
    }

    // One file off, the rest stay.
    client.delete_attachment(&card.id, &written.name)?;
    anyhow::ensure!(
        attach::list(&card.id).len() == 1,
        "the delete took too much"
    );
    println!("deleted     one file, one left");

    // The sweep: an archived card loses its files at once.
    client.patch_card(
        &card.id,
        &CardPatch {
            archived: Some(true),
            ..Default::default()
        },
    )?;
    engine.sweep_attachments();
    anyhow::ensure!(
        attach::list(&card.id).is_empty(),
        "an archived card kept its files"
    );
    anyhow::ensure!(
        !paths::card_attachments_dir(&card.id).exists(),
        "the directory of an archived card is still there"
    );
    println!("swept       the archived card lost its files");

    std::fs::remove_dir_all(&dir).ok();
    println!("\nall of it held.");
    Ok(())
}
