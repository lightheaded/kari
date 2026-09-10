//! Writes the hub holds for a node that is away.
//!
//! A node keeps the cards of its own machine, so every card write goes to the
//! node that owns the card. When that node is offline the write has nowhere to
//! go, and kari used to refuse it. A person then loses the note they typed
//! because a laptop is shut.
//!
//! So the hub queues the write instead. The queue is in the hub's own store,
//! one row per write, and it survives a restart of the hub. The board shows
//! the queued writes over the node's last known board, which is how a card
//! added to a sleeping laptop appears at once. When the node answers again,
//! the hub sends the queue in order and the node's own board replaces the
//! drawing.
//!
//! Only writes that ask the node to remember something are queued: add a card,
//! change a card, delete a card. An action that needs the machine itself —
//! run, stop, jump in, summarize — is still refused with the node's name,
//! because a queued one would run at a time nobody asked for.

use crate::model::*;
use crate::{infer, paths};
use chrono::Utc;

/// One write the hub holds for a node.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PendingWrite {
    /// Put this card on the node exactly as it is here.
    ///
    /// A task added while the node was away and the undo of a delete are the
    /// same write. The card carries the id the hub gave it, so the node adopts
    /// that id rather than inventing a second one, and every later write from
    /// this hub still names a card that exists.
    Put {
        card: Box<Card>,
    },
    Patch {
        card_id: String,
        patch: CardPatch,
    },
    Delete {
        card_id: String,
    },
}

impl PendingWrite {
    pub fn card_id(&self) -> &str {
        match self {
            PendingWrite::Put { card } => &card.id,
            PendingWrite::Patch { card_id, .. } => card_id,
            PendingWrite::Delete { card_id } => card_id,
        }
    }
}

/// A write in the queue, with the row that holds it.
#[derive(Debug, Clone)]
pub struct Queued {
    pub seq: i64,
    pub write: PendingWrite,
    /// What the node said the last time this write was sent. A write that
    /// fails stays at the head of the queue, so the ones behind it keep their
    /// order, and the board shows the reason on the node.
    pub last_error: Option<String>,
}

/// What the queue must do to take one more write.
///
/// Two writes on one card fold into one: a card added and then edited five
/// times is one card to put on the node, not six calls. The queue therefore
/// holds at most one write per card, and a card added and then deleted leaves
/// nothing behind, because the node never saw it.
#[derive(Debug, Default)]
pub struct Fold {
    /// Rows to delete.
    pub drop: Vec<i64>,
    /// The write to append. None when the new write cancels what was queued.
    pub write: Option<PendingWrite>,
}

/// Fold one new write into the writes already queued for the same node.
pub fn fold(queued: &[Queued], w: PendingWrite) -> Fold {
    let id = w.card_id().to_string();
    let mine: Vec<&Queued> = queued.iter().filter(|q| q.write.card_id() == id).collect();
    let drop: Vec<i64> = mine.iter().map(|q| q.seq).collect();
    let put = mine.iter().find_map(|q| match &q.write {
        PendingWrite::Put { card } => Some(card.as_ref().clone()),
        _ => None,
    });
    match w {
        PendingWrite::Put { card } => Fold {
            drop,
            write: Some(PendingWrite::Put { card }),
        },
        PendingWrite::Patch { card_id, patch } => {
            // The card is not on the node yet, so the edit belongs in the card
            // that goes there. This also lets a person repair a card whose
            // write the node refused: the next attempt sends the fixed card.
            if let Some(mut card) = put {
                card.apply_patch(&patch);
                card.updated_at = Utc::now();
                return Fold {
                    drop,
                    write: Some(PendingWrite::Put {
                        card: Box::new(card),
                    }),
                };
            }
            let mut merged = CardPatch::default();
            for q in &mine {
                if let PendingWrite::Patch { patch, .. } = &q.write {
                    merged.merge(patch.clone());
                }
            }
            merged.merge(patch);
            Fold {
                drop,
                write: Some(PendingWrite::Patch {
                    card_id,
                    patch: merged,
                }),
            }
        }
        PendingWrite::Delete { card_id } => Fold {
            drop,
            // A card the node never saw is dropped from the queue and nothing
            // is sent. Anything else has to be deleted where it lives.
            write: match put {
                Some(_) => None,
                None => Some(PendingWrite::Delete { card_id }),
            },
        },
    }
}

/// Draw the queued writes over the last board the node sent.
///
/// The board that comes back is what the person asked for: the card they added
/// is on it, the note they wrote is in it, and the card they deleted is gone.
/// It is a drawing, not the node's answer. The node replaces it in full when it
/// comes back and the queue is sent.
pub fn apply(board: &mut BoardView, queued: &[Queued], settings: &Settings) {
    let columns = board.columns.clone();
    for q in queued {
        match &q.write {
            PendingWrite::Put { card } => {
                let card = card.as_ref().clone();
                match board.cards.iter_mut().find(|v| v.card.id == card.id) {
                    Some(v) => {
                        v.card = card;
                        redraw(v, &columns, settings);
                    }
                    None => {
                        if let Some(v) = view_of(card, &columns, settings) {
                            board.cards.push(v);
                        }
                    }
                }
            }
            PendingWrite::Patch { card_id, patch } => {
                if let Some(v) = board.cards.iter_mut().find(|v| &v.card.id == card_id) {
                    v.card.apply_patch(patch);
                    redraw(v, &columns, settings);
                }
            }
            PendingWrite::Delete { card_id } => board.cards.retain(|v| &v.card.id != card_id),
        }
    }
}

/// The card ids the queue touches, so the board can mark them.
pub fn touched(queued: &[Queued]) -> Vec<String> {
    queued
        .iter()
        .filter(|q| !matches!(q.write, PendingWrite::Delete { .. }))
        .map(|q| q.write.card_id().to_string())
        .collect()
}

/// State, column and title again, after a write changed the card under them.
fn redraw(v: &mut CardView, columns: &[Column], settings: &Settings) {
    let inputs = infer::Inputs {
        card: &v.card,
        facts: v.session.as_ref(),
        live: v.live.as_ref(),
        bg: v.bg_job.as_ref(),
        herdr: v.herdr.as_ref(),
        hooks: v.hooks.as_ref(),
        permission: v.permission.as_ref(),
        summary: v.summary.as_ref(),
        now: Utc::now(),
        settings,
    };
    let (state, reason) = infer::derive(&inputs);
    let (column_id, locked, _) = infer::resolve_column(&v.card, state, columns);
    v.state = state;
    v.locked = locked;
    if let Some(id) = column_id {
        v.column_id = id;
    }
    v.reason = reason;
    if let Some(t) = v.card.title.clone() {
        v.title = t;
    }
    v.project_name = v
        .card
        .project_cwd
        .as_deref()
        .map(paths::project_display_name);
}

/// A card the node has never seen, drawn as the node would draw it.
///
/// It has no session, no process and no job, because it has never run. That
/// makes its state the one a new task derives on its own, and it needs none of
/// the signals the node holds.
fn view_of(card: Card, columns: &[Column], settings: &Settings) -> Option<CardView> {
    let inputs = infer::Inputs {
        card: &card,
        facts: None,
        live: None,
        bg: None,
        herdr: None,
        hooks: None,
        permission: None,
        summary: None,
        now: Utc::now(),
        settings,
    };
    let (state, reason) = infer::derive(&inputs);
    let (column_id, locked, _) = infer::resolve_column(&card, state, columns);
    Some(CardView {
        title: card
            .title
            .clone()
            .unwrap_or_else(|| card.id.chars().take(8).collect()),
        state,
        column_id: column_id?,
        locked,
        project_name: card.project_cwd.as_deref().map(paths::project_display_name),
        session: None,
        live: None,
        bg_job: None,
        herdr: None,
        summary: None,
        hooks: None,
        estimate: None,
        last_activity_at: None,
        reason,
        permission: None,
        // The card is not on its node yet, so no file of it is there either.
        attachments: vec![],
        card,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(id: &str) -> Card {
        Card {
            id: id.into(),
            kind: CardKind::Task,
            title: Some("t".into()),
            session_id: None,
            project_cwd: None,
            priority: 0,
            auto_run: false,
            run_prompt: None,
            permission_mode: None,
            model: None,
            estimate_weighted_tokens: None,
            manual_column: None,
            manual_lock_priority: None,
            tags: vec![],
            notes: None,
            archived: false,
            bg_job_id: None,
            started_by_autopilot: false,
            last_job_state: None,
            last_job_at: None,
            scheduled: None,
            created_at: Utc::now(),
            updated_at: Utc::now(),
            done_at: None,
        }
    }

    fn q(seq: i64, write: PendingWrite) -> Queued {
        Queued {
            seq,
            write,
            last_error: None,
        }
    }

    fn notes(v: &str) -> CardPatch {
        CardPatch {
            notes: Some(v.into()),
            ..Default::default()
        }
    }

    #[test]
    fn a_note_on_a_queued_card_goes_into_that_card() {
        let queued = vec![q(
            1,
            PendingWrite::Put {
                card: Box::new(card("a")),
            },
        )];
        let f = fold(
            &queued,
            PendingWrite::Patch {
                card_id: "a".into(),
                patch: notes("hello"),
            },
        );
        assert_eq!(f.drop, vec![1]);
        match f.write {
            Some(PendingWrite::Put { card }) => assert_eq!(card.notes.as_deref(), Some("hello")),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn two_notes_on_one_card_are_one_write() {
        let queued = vec![q(
            7,
            PendingWrite::Patch {
                card_id: "a".into(),
                patch: notes("first"),
            },
        )];
        let f = fold(
            &queued,
            PendingWrite::Patch {
                card_id: "a".into(),
                patch: CardPatch {
                    priority: Some(3),
                    ..Default::default()
                },
            },
        );
        assert_eq!(f.drop, vec![7]);
        match f.write {
            Some(PendingWrite::Patch { patch, .. }) => {
                assert_eq!(patch.notes.as_deref(), Some("first"));
                assert_eq!(patch.priority, Some(3));
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn a_card_added_and_deleted_leaves_nothing_to_send() {
        let queued = vec![q(
            2,
            PendingWrite::Put {
                card: Box::new(card("a")),
            },
        )];
        let f = fold(
            &queued,
            PendingWrite::Delete {
                card_id: "a".into(),
            },
        );
        assert_eq!(f.drop, vec![2]);
        assert!(f.write.is_none());
    }

    #[test]
    fn deleting_a_card_the_node_holds_is_still_sent() {
        let f = fold(
            &[],
            PendingWrite::Delete {
                card_id: "a".into(),
            },
        );
        assert!(f.drop.is_empty());
        assert!(matches!(f.write, Some(PendingWrite::Delete { .. })));
    }

    #[test]
    fn a_write_on_one_card_leaves_another_card_alone() {
        let queued = vec![q(
            1,
            PendingWrite::Put {
                card: Box::new(card("a")),
            },
        )];
        let f = fold(
            &queued,
            PendingWrite::Put {
                card: Box::new(card("b")),
            },
        );
        assert!(f.drop.is_empty());
    }

    fn board() -> BoardView {
        BoardView {
            columns: vec![Column {
                id: "backlog".into(),
                name: "Backlog".into(),
                order: 0,
                accepts: vec![DerivedState::Backlog],
                wip_limit: None,
                color: None,
                hidden: false,
            }],
            cards: vec![],
            quota: None,
            generated_at: Utc::now(),
            scanning: false,
            herdr_connected: false,
            hooks_installed: false,
            hooks_port: 0,
            calibration: Calibration::default(),
            proposal: None,
            away_mode: false,
            queue: None,
            automation_mode: String::new(),
        }
    }

    #[test]
    fn a_queued_card_lands_on_the_board() {
        let mut board = board();
        let queued = vec![
            q(
                1,
                PendingWrite::Put {
                    card: Box::new(card("a")),
                },
            ),
            q(
                2,
                PendingWrite::Patch {
                    card_id: "a".into(),
                    patch: notes("hello"),
                },
            ),
        ];
        apply(&mut board, &queued, &Settings::default());
        assert_eq!(board.cards.len(), 1);
        assert_eq!(board.cards[0].card.notes.as_deref(), Some("hello"));
        assert_eq!(board.cards[0].column_id, "backlog");
        assert_eq!(touched(&queued), vec!["a".to_string(), "a".to_string()]);
    }

    #[test]
    fn a_queued_delete_takes_the_card_off_the_board() {
        let mut board = board();
        apply(
            &mut board,
            &[q(
                1,
                PendingWrite::Put {
                    card: Box::new(card("a")),
                },
            )],
            &Settings::default(),
        );
        assert_eq!(board.cards.len(), 1);
        apply(
            &mut board,
            &[q(
                1,
                PendingWrite::Delete {
                    card_id: "a".into(),
                },
            )],
            &Settings::default(),
        );
        assert!(board.cards.is_empty());
    }
}
