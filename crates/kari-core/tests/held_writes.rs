//! A person writes to a machine that is not there.
//!
//! These tests open an engine, and `Engine::open_at` fixes the kari directory
//! for the whole process. That is why they live in a test binary of their own,
//! and why they share one hub.

use kari_core::hub::Hub;
use kari_core::model::{CardPatch, NewNode, NewTask};
use kari_core::Engine;
use std::sync::Arc;

/// A hub whose only node is at an address that answers nothing.
fn hub_with_a_node_that_is_away() -> (Arc<Hub>, String) {
    let dir = std::env::temp_dir().join(format!("kari-held-writes-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    let engine = Engine::open_at(&dir).expect("engine");
    let hub = Hub::without_local(engine);
    // Port 9 is discard, and nothing listens on it here, so the node never
    // comes online. No token is given, so the test stores none.
    let node = hub
        .add_node(NewNode {
            name: "away".into(),
            address: Some("127.0.0.1:9".into()),
            ..Default::default()
        })
        .expect("add node");
    (hub, node.id)
}

fn task(title: &str) -> NewTask {
    NewTask {
        title: title.into(),
        project_cwd: None,
        run_prompt: None,
        auto_run: false,
        priority: 0,
        notes: None,
        model: None,
        column_id: None,
    }
}

/// The whole point of the queue: a person writes to a machine that is away,
/// and neither the card nor the note is lost.
#[test]
fn a_task_written_to_a_node_that_is_away_waits_on_the_board() {
    let (hub, node) = hub_with_a_node_that_is_away();
    let card = hub.add_task(&node, task("write it down")).expect("add");

    // On the board at once, marked as waiting, and counted on the node.
    let b = hub.board();
    let hc = b
        .cards
        .iter()
        .find(|c| c.view.card.id == card.id)
        .expect("the card is on the board");
    assert!(hc.pending, "the card waits for the node");
    assert_eq!(hub.nodes()[0].pending_writes, 1);

    // A note on that card is the same one write, not a second.
    let patched = hub
        .patch_card(
            &node,
            &card.id,
            CardPatch {
                notes: Some("the point".into()),
                ..Default::default()
            },
        )
        .expect("patch");
    assert_eq!(patched.notes.as_deref(), Some("the point"));
    assert_eq!(hub.nodes()[0].pending_writes, 1);
    let b = hub.board();
    let hc = b
        .cards
        .iter()
        .find(|c| c.view.card.id == card.id)
        .expect("still on the board");
    assert_eq!(hc.view.card.notes.as_deref(), Some("the point"));

    // An action that needs the machine is still refused, by name.
    let e = hub
        .start_card(&node, &card.id, None)
        .expect_err("a card cannot run on a machine that is not there");
    assert!(e.to_string().contains("offline"), "{e}");

    // Deleting it leaves the node nothing to do: it never saw the card.
    hub.delete_card(&node, &card.id).expect("delete");
    assert_eq!(hub.nodes()[0].pending_writes, 0);
    assert!(hub.board().cards.is_empty());
}
