//! The seam between the UI and whatever is answering it.
//!
//! Until now the desktop app held a `Hub` directly: an in-process object that
//! keeps the board, owns the columns and dials every node itself. That works
//! while the app is the only client. It stops working once the clients
//! outnumber the desks, because each client is then its own hub and a node is
//! only as online as *that* client's path to it — a laptop asleep, a laptop on
//! someone else's network, a phone on mobile data.
//!
//! The answer is a server that holds the one hub, with the nodes dialling out
//! to it (see `link`). But the single-machine setup must keep working exactly
//! as it does, with no server to run and nothing to enrol. So the hub becomes a
//! trait with two implementations: `Hub` in this process, and `RemoteHub`
//! talking to a server. The Tauri layer and the React UI hold the trait, so
//! neither knows which one it has.
//!
//! Everything here is blocking, matching `Hub`'s existing shape. The Tauri
//! commands already push hub calls onto a blocking thread; making the trait
//! async would have rewritten every one of them for no gain.

use crate::model::*;
use crate::{hub::HubEvent, Engine};
use std::sync::Arc;
use tokio::sync::broadcast;

/// What the UI asks of a hub.
///
/// One method per Tauri command, and deliberately no more: this is the list of
/// things the UI can do, so a method appearing here is a claim that a client of
/// a server can do it too.
pub trait HubApi: Send + Sync + 'static {
    // --- the board -------------------------------------------------------

    /// Board changes and notices. A `RemoteHub` republishes the server's
    /// stream into a channel of its own, so a subscriber cannot tell.
    fn subscribe(&self) -> broadcast::Receiver<HubEvent>;

    fn board(&self) -> HubBoard;
    fn nodes(&self) -> Vec<NodeStatus>;
    /// Re-read every node now. Advisory: the board arrives by event either way.
    fn refresh_all(&self);

    // --- columns ---------------------------------------------------------
    //
    // The columns belong to the hub, not to a node, so with a server there is
    // one set of them rather than one per client.

    fn columns(&self) -> Vec<Column>;
    fn set_columns(&self, cols: Vec<Column>) -> anyhow::Result<()>;
    fn reset_columns(&self) -> anyhow::Result<()>;

    // --- cards -----------------------------------------------------------
    //
    // A card is `(node id, card id)` and every action routes by node. With a
    // server the routing is the server's job; the arguments do not change.

    fn add_task(&self, node: &str, t: NewTask) -> anyhow::Result<Card>;
    fn patch_card(&self, node: &str, card: &str, p: CardPatch) -> anyhow::Result<Card>;
    fn move_card(&self, node: &str, card: &str, column: &str) -> anyhow::Result<()>;
    fn delete_card(&self, node: &str, card: &str) -> anyhow::Result<()>;
    fn restore_card(&self, node: &str, card: Card) -> anyhow::Result<Card>;
    fn reorder_cards(
        &self,
        node: &str,
        ranked: Vec<String>,
        unranked: Vec<String>,
    ) -> anyhow::Result<()>;
    /// Move a task card to another node. Each node keeps its own store, so the
    /// card is written on the target and deleted from the source: the card
    /// that comes back has a new id.
    fn move_card_to_node(&self, from: &str, card: &str, to: &str) -> anyhow::Result<Card>;

    // --- running a card --------------------------------------------------

    fn start_card(&self, node: &str, card: &str, prompt: Option<String>) -> anyhow::Result<String>;
    fn stop_card(&self, node: &str, card: &str) -> anyhow::Result<()>;
    /// Give the card its next prompt: into the running session when there is
    /// one, else as a background job. The string says which.
    fn send_prompt(&self, node: &str, card: &str, text: &str) -> anyhow::Result<String>;
    /// The prompts and replies of the card's session, the last `limit` of them.
    fn conversation(&self, node: &str, card: &str, limit: usize) -> anyhow::Result<Conversation>;
    fn stop_all(&self) -> anyhow::Result<usize>;
    fn summarize_card(&self, node: &str, card: &str) -> anyhow::Result<Summary>;
    fn job_log(&self, node: &str, card: &str, limit: usize) -> Vec<JobLogEntry>;
    /// The command to run in a terminal to resume the card's session. The
    /// string comes back to the caller, who runs it on *their* machine.
    fn jump_in(&self, node: &str, card: &str) -> anyhow::Result<String>;
    fn answer_permission(&self, node: &str, id: &str, behavior: &str) -> anyhow::Result<()>;

    // --- automation ------------------------------------------------------

    fn set_automation_mode(&self, node: &str, mode: AutomationMode) -> anyhow::Result<()>;
    /// Best effort across every node. Returns the ids that refused.
    fn set_automation_mode_all(&self, mode: AutomationMode) -> Vec<String>;
    fn set_away_mode(&self, node: &str, on: bool) -> anyhow::Result<()>;

    // --- the planner -----------------------------------------------------

    fn propose_now(&self, node: &str) -> anyhow::Result<Proposal>;
    fn proposal(&self, node: &str) -> Option<Proposal>;
    fn proposal_history(&self, node: &str, limit: usize) -> Vec<Proposal>;
    fn accept_proposal(
        &self,
        node: &str,
        id: &str,
        card_ids: Option<Vec<String>>,
    ) -> anyhow::Result<usize>;
    fn snooze_proposal(&self, node: &str, id: &str, minutes: i64) -> anyhow::Result<()>;
    fn dismiss_proposal(&self, node: &str, id: &str) -> anyhow::Result<()>;
    fn stop_proposal(&self, node: &str, id: &str) -> anyhow::Result<usize>;

    // --- a node's own data ------------------------------------------------

    fn projects(&self, node: &str) -> Vec<Project>;
    fn quota_history(&self, node: &str, limit: usize) -> Vec<QuotaSample>;

    // --- the roster -------------------------------------------------------

    fn add_node(&self, n: NewNode) -> anyhow::Result<NodeStatus>;
    fn update_node(&self, id: &str, p: NodePatch) -> anyhow::Result<NodeStatus>;
    fn remove_node(&self, id: &str) -> anyhow::Result<()>;
    fn pair_node(&self, id: &str) -> anyhow::Result<String>;
    fn pairing_code(&self) -> anyhow::Result<String>;

    // --- the lease --------------------------------------------------------
    //
    // Only meaningful when several hubs can push columns to one node, which is
    // exactly the arrangement a server replaces. A `RemoteHub` is never
    // primary and cannot claim: there is one hub, and it is the server's.

    fn is_primary(&self) -> bool;
    fn claim_primary(&self) -> anyhow::Result<String>;

    // --- this machine -----------------------------------------------------

    /// True when this hub is a client of a server rather than the hub itself.
    ///
    /// The UI needs to know, because the controls that arbitrate between
    /// several hubs — the primary lease — mean nothing when there is one hub
    /// and it is somewhere else.
    fn is_remote(&self) -> bool {
        false
    }

    /// This device's own store.
    ///
    /// Settings, the account aliases, the calibration and the hook
    /// installation are properties of the machine the app runs on, not of the
    /// board, so they never travel to a server. Every hub has one of these,
    /// including a client of a server and including a phone: what differs is
    /// whether that engine is also a *node* on the board, which is the
    /// distinction `Hub::new` and `Hub::without_local` already draw.
    fn local_engine(&self) -> &Arc<Engine>;
}

/// The in-process hub, which is what the app has had all along. Every method
/// forwards; the trait adds no behaviour to the single-machine path.
impl HubApi for crate::hub::Hub {
    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        crate::hub::Hub::subscribe(self)
    }
    fn board(&self) -> HubBoard {
        crate::hub::Hub::board(self)
    }
    fn nodes(&self) -> Vec<NodeStatus> {
        crate::hub::Hub::nodes(self)
    }
    fn refresh_all(&self) {
        crate::hub::Hub::refresh_all(self)
    }

    fn columns(&self) -> Vec<Column> {
        self.engine().columns()
    }
    fn set_columns(&self, cols: Vec<Column>) -> anyhow::Result<()> {
        crate::hub::Hub::set_columns(self, cols)
    }
    fn reset_columns(&self) -> anyhow::Result<()> {
        crate::hub::Hub::reset_columns(self)
    }

    fn add_task(&self, node: &str, t: NewTask) -> anyhow::Result<Card> {
        crate::hub::Hub::add_task(self, node, t)
    }
    fn patch_card(&self, node: &str, card: &str, p: CardPatch) -> anyhow::Result<Card> {
        crate::hub::Hub::patch_card(self, node, card, p)
    }
    fn move_card(&self, node: &str, card: &str, column: &str) -> anyhow::Result<()> {
        crate::hub::Hub::move_card(self, node, card, column)
    }
    fn delete_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        crate::hub::Hub::delete_card(self, node, card)
    }
    fn restore_card(&self, node: &str, card: Card) -> anyhow::Result<Card> {
        crate::hub::Hub::restore_card(self, node, card)
    }
    fn reorder_cards(
        &self,
        node: &str,
        ranked: Vec<String>,
        unranked: Vec<String>,
    ) -> anyhow::Result<()> {
        crate::hub::Hub::reorder_cards(self, node, ranked, unranked)
    }
    fn move_card_to_node(&self, from: &str, card: &str, to: &str) -> anyhow::Result<Card> {
        crate::hub::Hub::move_card_to_node(self, from, card, to)
    }

    fn start_card(&self, node: &str, card: &str, prompt: Option<String>) -> anyhow::Result<String> {
        crate::hub::Hub::start_card(self, node, card, prompt)
    }
    fn stop_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        crate::hub::Hub::stop_card(self, node, card)
    }
    fn send_prompt(&self, node: &str, card: &str, text: &str) -> anyhow::Result<String> {
        crate::hub::Hub::send_prompt(self, node, card, text)
    }
    fn conversation(&self, node: &str, card: &str, limit: usize) -> anyhow::Result<Conversation> {
        crate::hub::Hub::conversation(self, node, card, limit)
    }
    fn stop_all(&self) -> anyhow::Result<usize> {
        crate::hub::Hub::stop_all(self)
    }
    fn summarize_card(&self, node: &str, card: &str) -> anyhow::Result<Summary> {
        crate::hub::Hub::summarize_card(self, node, card)
    }
    fn job_log(&self, node: &str, card: &str, limit: usize) -> Vec<JobLogEntry> {
        crate::hub::Hub::job_log(self, node, card, limit)
    }
    fn jump_in(&self, node: &str, card: &str) -> anyhow::Result<String> {
        crate::hub::Hub::jump_in(self, node, card)
    }
    fn answer_permission(&self, node: &str, id: &str, behavior: &str) -> anyhow::Result<()> {
        crate::hub::Hub::answer_permission(self, node, id, behavior)
    }

    fn set_automation_mode(&self, node: &str, mode: AutomationMode) -> anyhow::Result<()> {
        crate::hub::Hub::set_automation_mode(self, node, mode)
    }
    fn set_automation_mode_all(&self, mode: AutomationMode) -> Vec<String> {
        crate::hub::Hub::set_automation_mode_all(self, mode)
    }
    fn set_away_mode(&self, node: &str, on: bool) -> anyhow::Result<()> {
        crate::hub::Hub::set_away_mode(self, node, on)
    }

    fn propose_now(&self, node: &str) -> anyhow::Result<Proposal> {
        crate::hub::Hub::propose_now(self, node)
    }
    fn proposal(&self, node: &str) -> Option<Proposal> {
        crate::hub::Hub::proposal(self, node)
    }
    fn proposal_history(&self, node: &str, limit: usize) -> Vec<Proposal> {
        crate::hub::Hub::proposal_history(self, node, limit)
    }
    fn accept_proposal(
        &self,
        node: &str,
        id: &str,
        card_ids: Option<Vec<String>>,
    ) -> anyhow::Result<usize> {
        crate::hub::Hub::accept_proposal(self, node, id, card_ids)
    }
    fn snooze_proposal(&self, node: &str, id: &str, minutes: i64) -> anyhow::Result<()> {
        crate::hub::Hub::snooze_proposal(self, node, id, minutes)
    }
    fn dismiss_proposal(&self, node: &str, id: &str) -> anyhow::Result<()> {
        crate::hub::Hub::dismiss_proposal(self, node, id)
    }
    fn stop_proposal(&self, node: &str, id: &str) -> anyhow::Result<usize> {
        crate::hub::Hub::stop_proposal(self, node, id)
    }

    fn projects(&self, node: &str) -> Vec<Project> {
        crate::hub::Hub::projects(self, node)
    }
    fn quota_history(&self, node: &str, limit: usize) -> Vec<QuotaSample> {
        crate::hub::Hub::quota_history(self, node, limit)
    }

    fn add_node(&self, n: NewNode) -> anyhow::Result<NodeStatus> {
        crate::hub::Hub::add_node(self, n)
    }
    fn update_node(&self, id: &str, p: NodePatch) -> anyhow::Result<NodeStatus> {
        crate::hub::Hub::update_node(self, id, p)
    }
    fn remove_node(&self, id: &str) -> anyhow::Result<()> {
        crate::hub::Hub::remove_node(self, id)
    }
    fn pair_node(&self, id: &str) -> anyhow::Result<String> {
        crate::hub::Hub::pair_node(self, id)
    }
    fn pairing_code(&self) -> anyhow::Result<String> {
        crate::hub::Hub::pairing_code(self)
    }

    fn is_primary(&self) -> bool {
        crate::hub::Hub::is_primary(self)
    }
    fn claim_primary(&self) -> anyhow::Result<String> {
        crate::hub::Hub::claim_primary(self)
    }

    fn local_engine(&self) -> &Arc<Engine> {
        self.engine()
    }
}
