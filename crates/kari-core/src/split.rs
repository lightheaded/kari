//! `SplitHub`: this machine, plus a server.
//!
//! A desktop with a server configured has two sources of board, and it needs
//! both. The server carries the other hosts, which is why it exists. The engine
//! in this process carries the sessions of the machine the window is on, and
//! those must be on the board before any network answers — see "What kari is"
//! in `AGENTS.md`.
//!
//! Version 3 shipped without this type, and the app in server mode was a
//! `RemoteHub` alone. A Mac with a server then showed no cards of its own: a
//! new session started, landed in `~/.claude`, and nothing on the board moved,
//! because the only board came over the network from a host that never reads
//! that directory. The local engine was still open, but only as a store for
//! settings.
//!
//! So the board is a merge, and the routing follows the merge:
//!
//! - Cards of this machine come from the local hub, always, whether or not the
//!   server answers. Cards of every other node come from the server.
//! - An action on this machine's card goes to the local engine. It works with
//!   the network down, because nothing about a session on this Mac needs a
//!   server.
//! - Columns belong to the server while it answers, because there is one set
//!   for every client. A copy is kept in the local store, so the board still
//!   has its columns when the server is away.
//!
//! The server also knows this machine, once a node on it dials in. Its copy is
//! dropped from the merge by node id: one machine is one row, and the local
//! half is the fresher of the two because it is not a cache of anything.

use crate::hub::{Hub, HubEvent, LOCAL};
use crate::hubapi::HubApi;
use crate::model::*;
use crate::remote::RemoteHub;
use crate::Engine;
use std::sync::Arc;
use tokio::sync::broadcast;
use tracing::{info, warn};

pub struct SplitHub {
    /// This machine, and nothing else. Built with `Hub::local_only`, so it
    /// dials no node of its own: the other hosts arrive through the server.
    ///
    /// Named `machine` rather than `local` because a field of that name reads,
    /// at every use, as a host name in the `.local` domain — which is what
    /// `scripts/check-privacy.sh` looks for. The guard is worth more than the
    /// shorter field name.
    machine: Arc<Hub>,
    server: Arc<RemoteHub>,
    /// This machine's node id, which is how the server names it. Cards and
    /// rows the server holds under this id are dropped from the merge.
    local_node_id: String,
    tx: broadcast::Sender<HubEvent>,
}

impl SplitHub {
    pub fn connect(engine: Arc<Engine>, base: &str, token: &str) -> Arc<SplitHub> {
        let local_node_id = engine.node_id();
        let machine = Hub::local_only(Arc::clone(&engine));
        let server = RemoteHub::connect(base, token, engine);
        let (tx, _) = broadcast::channel(256);
        let hub = Arc::new(SplitHub {
            machine,
            server,
            local_node_id,
            tx,
        });
        hub.forward(hub.machine.subscribe(), "kari-split-machine");
        hub.forward(hub.server.subscribe(), "kari-split-server");
        info!(
            "the board is this machine plus the server at {}",
            hub.server.base()
        );
        hub
    }

    /// Republish one half's events as this hub's own, so the UI subscribes
    /// once and cannot tell where a change came from.
    fn forward(&self, mut rx: broadcast::Receiver<HubEvent>, name: &str) {
        let tx = self.tx.clone();
        std::thread::Builder::new()
            .name(name.into())
            .spawn(move || loop {
                match rx.blocking_recv() {
                    Ok(e) => {
                        if tx.send(e).is_err() {
                            break;
                        }
                    }
                    // A dropped event only costs a board refetch, and the next
                    // event triggers one.
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(broadcast::error::RecvError::Closed) => break,
                }
            })
            .expect("spawn");
    }

    /// True for the node this window runs on, under either name it goes by:
    /// `local` in this process, and its node id everywhere else.
    fn is_local(&self, node: &str) -> bool {
        node == LOCAL || node == self.local_node_id
    }

    /// The local hub always calls this machine `local`, whatever the caller
    /// said.
    fn on_local<T>(&self, f: impl FnOnce(&Arc<Hub>) -> T) -> T {
        f(&self.machine)
    }
}

/// Fold the server's account rows into this machine's.
///
/// Two nodes on one Claude Code login share one budget, and after the merge
/// they must share one row: the whole point of grouping by account is that a
/// meter per machine reads as two budgets where there is one. The freshest
/// sample wins inside a row, as it does in `account::group`.
fn merge_accounts(mut mine: Vec<AccountQuota>, theirs: Vec<AccountQuota>) -> Vec<AccountQuota> {
    for row in theirs {
        match mine.iter_mut().find(|g| g.key == row.key) {
            Some(g) => {
                g.node_ids.extend(row.node_ids);
                g.node_names.extend(row.node_names);
                let newer = match (&g.quota, &row.quota) {
                    (_, None) => false,
                    (None, Some(_)) => true,
                    (Some(have), Some(new)) => new.at > have.at,
                };
                if newer {
                    g.quota = row.quota;
                    g.calibration = row.calibration;
                }
                // An alias the user set on this device wins: it is the label
                // they are looking at.
                if g.alias.is_none() {
                    if let Some(a) = row.alias {
                        g.label = a.clone();
                        g.alias = Some(a);
                    }
                }
            }
            None => mine.push(row),
        }
    }
    mine
}

impl HubApi for SplitHub {
    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    fn board(&self) -> HubBoard {
        // This machine first, and unconditionally. Everything below only adds.
        let mut b = self.machine.board();
        let sb = match self.server.try_board() {
            Ok(sb) => sb,
            Err(e) => {
                // Not an error state for the board: the cards of this machine
                // are on it, which is what the person in front of it needs.
                warn!("board from {}: {e}", self.server.base());
                return b;
            }
        };

        // The columns are the server's while it answers, so every client shows
        // the same board. A local card can carry a column this set does not
        // have, exactly as a remote card can, and maps the same way.
        if !sb.columns.is_empty() {
            if sb.columns != b.columns {
                if let Err(e) = self.machine.local_engine().set_columns(sb.columns.clone()) {
                    warn!("the server's columns are not cached locally: {e}");
                }
                for c in b.cards.iter_mut() {
                    c.view.column_id = Hub::map_column(&sb.columns, &c.view);
                }
            }
            b.columns = sb.columns;
        }

        let mine = self.local_node_id.as_str();
        b.nodes
            .extend(sb.nodes.into_iter().filter(|n| n.id != mine));
        b.cards
            .extend(sb.cards.into_iter().filter(|c| c.node_id != mine));
        b.quotas
            .extend(sb.quotas.into_iter().filter(|q| q.node_id != mine));
        b.queues
            .extend(sb.queues.into_iter().filter(|q| q.node_id != mine));
        b.proposals
            .extend(sb.proposals.into_iter().filter(|p| p.node_id != mine));
        b.accounts = merge_accounts(
            b.accounts,
            sb.accounts
                .into_iter()
                .filter(|a| a.node_ids.iter().any(|n| n != mine))
                .collect(),
        );

        // The hub is the server's, and it says so. The rest of these describe
        // the machine the window is on, so they stay local: a server runs no
        // Claude Code, has no hooks and scans nothing.
        b.hub_id = sb.hub_id;
        b.hub_name = sb.hub_name;
        b.primary = sb.primary;
        b
    }

    fn nodes(&self) -> Vec<NodeStatus> {
        let mut out = self.machine.nodes();
        let mine = self.local_node_id.as_str();
        out.extend(self.server.nodes().into_iter().filter(|n| n.id != mine));
        out
    }

    fn refresh_all(&self) {
        self.machine.refresh_all();
        self.server.refresh_all();
    }

    fn columns(&self) -> Vec<Column> {
        let c = self.server.columns();
        if c.is_empty() {
            // The server is away. The copy in the local store is the set it
            // last published, which is closer than nothing.
            return self.machine.columns();
        }
        c
    }

    fn set_columns(&self, cols: Vec<Column>) -> anyhow::Result<()> {
        // To the server, which owns them for every client. The local copy
        // follows on the next board, from the server's own answer.
        self.server.set_columns(cols)
    }

    fn reset_columns(&self) -> anyhow::Result<()> {
        self.server.reset_columns()
    }

    fn add_task(&self, node: &str, t: NewTask) -> anyhow::Result<Card> {
        if self.is_local(node) {
            return self.on_local(|h| h.add_task(LOCAL, t));
        }
        self.server.add_task(node, t)
    }

    fn patch_card(&self, node: &str, card: &str, p: CardPatch) -> anyhow::Result<Card> {
        if self.is_local(node) {
            return self.on_local(|h| h.patch_card(LOCAL, card, p));
        }
        self.server.patch_card(node, card, p)
    }

    fn move_card(&self, node: &str, card: &str, column: &str) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.move_card(LOCAL, card, column));
        }
        self.server.move_card(node, card, column)
    }

    fn delete_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.delete_card(LOCAL, card));
        }
        self.server.delete_card(node, card)
    }

    fn restore_card(&self, node: &str, card: Card) -> anyhow::Result<Card> {
        if self.is_local(node) {
            return self.on_local(|h| h.restore_card(LOCAL, card));
        }
        self.server.restore_card(node, card)
    }

    fn reorder_cards(
        &self,
        node: &str,
        ranked: Vec<String>,
        unranked: Vec<String>,
    ) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.reorder_cards(LOCAL, ranked, unranked));
        }
        self.server.reorder_cards(node, ranked, unranked)
    }

    /// A card moves between two stores, and this hub reaches one of them
    /// directly and the other through the server. Neither half can do the pair
    /// on its own, so the two ends are done here: write on the target, then
    /// delete from the source.
    fn move_card_to_node(&self, from: &str, card: &str, to: &str) -> anyhow::Result<Card> {
        if self.is_local(from) && self.is_local(to) {
            return self.on_local(|h| h.move_card_to_node(LOCAL, card, LOCAL));
        }
        if !self.is_local(from) && !self.is_local(to) {
            return self.server.move_card_to_node(from, card, to);
        }

        let source = self.board();
        let hc = source
            .cards
            .iter()
            .find(|c| c.view.card.id == card && self.same_node(&c.node_id, from))
            .ok_or_else(|| anyhow::anyhow!("no card {card} on that node"))?;
        if hc.view.card.kind != CardKind::Task {
            anyhow::bail!("only a task card moves between nodes");
        }
        let mut moved = hc.view.card.clone();
        // A new id, because the card is written on a store that does not hold
        // it. An empty id was written here before, and `restore_card` keeps
        // the id it is given, so the target node ended up with a card whose id
        // was the empty string: a second move then wrote over the first.
        moved.id = uuid::Uuid::new_v4().to_string();
        // Read the files before anything is written. A file that cannot be
        // read stops the move rather than arriving after the card is gone.
        let mut files: Vec<NewAttachment> = vec![];
        for a in &hc.view.attachments {
            let got = HubApi::attachment(self, from, card, &a.name).map_err(|e| {
                anyhow::anyhow!(
                    "the attachment {} could not be read: {e}. The card did not move.",
                    a.name
                )
            })?;
            files.push(NewAttachment {
                name: a.name.clone(),
                data_b64: got.data_b64,
            });
        }

        let written = if self.is_local(to) {
            self.on_local(|h| h.restore_card(LOCAL, moved))?
        } else {
            self.server.restore_card(to, moved)?
        };
        for f in files {
            let name = f.name.clone();
            if let Err(e) = HubApi::add_attachment(self, to, &written.id, f) {
                anyhow::bail!(
                    "the card was created on the target node but the attachment {name} did not go with it: {e}. Delete the new card and try again."
                );
            }
        }
        // Delete second: a card that exists twice is recoverable by hand, and
        // one deleted before the write lands is gone.
        let deleted = if self.is_local(from) {
            self.on_local(|h| h.delete_card(LOCAL, card))
        } else {
            self.server.delete_card(from, card)
        };
        if let Err(e) = deleted {
            warn!("card {card} was copied to {to} but not deleted from {from}: {e}");
        }
        Ok(written)
    }

    fn add_attachment(
        &self,
        node: &str,
        card: &str,
        a: NewAttachment,
    ) -> anyhow::Result<Attachment> {
        if self.is_local(node) {
            return self.on_local(|h| h.add_attachment(LOCAL, card, a));
        }
        self.server.add_attachment(node, card, a)
    }

    fn attachment(&self, node: &str, card: &str, name: &str) -> anyhow::Result<AttachmentData> {
        if self.is_local(node) {
            return self.on_local(|h| h.attachment(LOCAL, card, name));
        }
        self.server.attachment(node, card, name)
    }

    fn delete_attachment(&self, node: &str, card: &str, name: &str) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.delete_attachment(LOCAL, card, name));
        }
        self.server.delete_attachment(node, card, name)
    }

    fn start_card(&self, node: &str, card: &str, prompt: Option<String>) -> anyhow::Result<String> {
        if self.is_local(node) {
            return self.on_local(|h| h.start_card(LOCAL, card, prompt));
        }
        self.server.start_card(node, card, prompt)
    }

    fn stop_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.stop_card(LOCAL, card));
        }
        self.server.stop_card(node, card)
    }

    fn send_prompt(&self, node: &str, card: &str, text: &str) -> anyhow::Result<String> {
        if self.is_local(node) {
            return self.on_local(|h| h.send_prompt(LOCAL, card, text));
        }
        self.server.send_prompt(node, card, text)
    }

    fn conversation(&self, node: &str, card: &str, limit: usize) -> anyhow::Result<Conversation> {
        if self.is_local(node) {
            return self.on_local(|h| h.conversation(LOCAL, card, limit));
        }
        self.server.conversation(node, card, limit)
    }

    fn schedule_card(&self, node: &str, card: &str, req: ScheduleRequest) -> anyhow::Result<Card> {
        if self.is_local(node) {
            return self.on_local(|h| h.schedule_card(LOCAL, card, req));
        }
        self.server.schedule_card(node, card, req)
    }

    fn cancel_schedule(&self, node: &str, card: &str) -> anyhow::Result<Card> {
        if self.is_local(node) {
            return self.on_local(|h| h.cancel_schedule(LOCAL, card));
        }
        self.server.cancel_schedule(node, card)
    }

    /// The kill switch. This machine first, and its count stands even when the
    /// server cannot be reached: stopping the jobs in front of the user must
    /// not depend on a network.
    fn stop_all(&self) -> anyhow::Result<usize> {
        let n = self.machine.stop_all()?;
        match self.server.stop_all() {
            Ok(m) => Ok(n + m),
            Err(e) => {
                warn!("stop all on {}: {e}", self.server.base());
                Ok(n)
            }
        }
    }

    fn summarize_card(&self, node: &str, card: &str) -> anyhow::Result<Summary> {
        if self.is_local(node) {
            return self.on_local(|h| h.summarize_card(LOCAL, card));
        }
        self.server.summarize_card(node, card)
    }

    fn job_log(&self, node: &str, card: &str, limit: usize) -> Vec<JobLogEntry> {
        if self.is_local(node) {
            return self.on_local(|h| h.job_log(LOCAL, card, limit));
        }
        self.server.job_log(node, card, limit)
    }

    fn jump_in(&self, node: &str, card: &str) -> anyhow::Result<String> {
        if self.is_local(node) {
            return self.on_local(|h| h.jump_in(LOCAL, card));
        }
        self.server.jump_in(node, card)
    }

    fn answer_permission(&self, node: &str, id: &str, behavior: &str) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.answer_permission(LOCAL, id, behavior));
        }
        self.server.answer_permission(node, id, behavior)
    }

    fn set_automation_mode(&self, node: &str, mode: AutomationMode) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.set_automation_mode(LOCAL, mode));
        }
        self.server.set_automation_mode(node, mode)
    }

    fn set_automation_mode_all(&self, mode: AutomationMode) -> Vec<String> {
        let mut refused = self.machine.set_automation_mode_all(mode);
        refused.extend(self.server.set_automation_mode_all(mode));
        refused
    }

    fn set_away_mode(&self, node: &str, on: bool) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.set_away_mode(LOCAL, on));
        }
        self.server.set_away_mode(node, on)
    }

    fn propose_now(&self, node: &str) -> anyhow::Result<Proposal> {
        if self.is_local(node) {
            return self.on_local(|h| h.propose_now(LOCAL));
        }
        self.server.propose_now(node)
    }

    fn proposal(&self, node: &str) -> Option<Proposal> {
        if self.is_local(node) {
            return self.on_local(|h| h.proposal(LOCAL));
        }
        self.server.proposal(node)
    }

    fn proposal_history(&self, node: &str, limit: usize) -> Vec<Proposal> {
        if self.is_local(node) {
            return self.on_local(|h| h.proposal_history(LOCAL, limit));
        }
        self.server.proposal_history(node, limit)
    }

    fn accept_proposal(
        &self,
        node: &str,
        id: &str,
        card_ids: Option<Vec<String>>,
    ) -> anyhow::Result<usize> {
        if self.is_local(node) {
            return self.on_local(|h| h.accept_proposal(LOCAL, id, card_ids));
        }
        self.server.accept_proposal(node, id, card_ids)
    }

    fn snooze_proposal(&self, node: &str, id: &str, minutes: i64) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.snooze_proposal(LOCAL, id, minutes));
        }
        self.server.snooze_proposal(node, id, minutes)
    }

    fn dismiss_proposal(&self, node: &str, id: &str) -> anyhow::Result<()> {
        if self.is_local(node) {
            return self.on_local(|h| h.dismiss_proposal(LOCAL, id));
        }
        self.server.dismiss_proposal(node, id)
    }

    fn stop_proposal(&self, node: &str, id: &str) -> anyhow::Result<usize> {
        if self.is_local(node) {
            return self.on_local(|h| h.stop_proposal(LOCAL, id));
        }
        self.server.stop_proposal(node, id)
    }

    fn projects(&self, node: &str) -> Vec<Project> {
        if self.is_local(node) {
            return self.on_local(|h| h.projects(LOCAL));
        }
        self.server.projects(node)
    }

    fn quota_history(&self, node: &str, limit: usize) -> Vec<QuotaSample> {
        if self.is_local(node) {
            return self.on_local(|h| h.quota_history(LOCAL, limit));
        }
        self.server.quota_history(node, limit)
    }

    // The roster is the server's: its nodes dial it, and there is nothing here
    // to add or pair. The refusal carries the server's address, so the message
    // says where to go.

    fn add_node(&self, n: NewNode) -> anyhow::Result<NodeStatus> {
        self.server.add_node(n)
    }

    fn update_node(&self, id: &str, p: NodePatch) -> anyhow::Result<NodeStatus> {
        self.server.update_node(id, p)
    }

    fn remove_node(&self, id: &str) -> anyhow::Result<()> {
        self.server.remove_node(id)
    }

    fn pair_node(&self, id: &str) -> anyhow::Result<String> {
        self.server.pair_node(id)
    }

    fn pairing_code(&self) -> anyhow::Result<String> {
        self.server.pairing_code()
    }

    fn is_primary(&self) -> bool {
        self.server.is_primary()
    }

    fn claim_primary(&self) -> anyhow::Result<String> {
        self.server.claim_primary()
    }

    fn is_remote(&self) -> bool {
        true
    }

    fn local_engine(&self) -> &Arc<Engine> {
        self.machine.local_engine()
    }
}

impl SplitHub {
    /// Whether two node names mean the same node, with `local` and this
    /// machine's id counted as one.
    fn same_node(&self, a: &str, b: &str) -> bool {
        a == b || (self.is_local(a) && self.is_local(b))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{TimeZone, Utc};

    fn sample(at: i64) -> QuotaSample {
        QuotaSample {
            at: Utc.timestamp_opt(at, 0).unwrap(),
            five_hour: None,
            seven_day: None,
            source: "test".into(),
        }
    }

    fn row(key: &str, node: &str, at: Option<i64>) -> AccountQuota {
        AccountQuota {
            key: key.into(),
            label: node.into(),
            alias: None,
            account: None,
            node_ids: vec![node.into()],
            node_names: vec![node.into()],
            quota: at.map(sample),
            calibration: None,
        }
    }

    #[test]
    fn two_nodes_on_one_login_stay_one_row() {
        // The reason grouping exists: a meter per machine reads as two budgets
        // where the login holds one.
        let out = merge_accounts(
            vec![row("acc-1", "local", Some(100))],
            vec![row("acc-1", "other", Some(200))],
        );
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].node_names, vec!["local", "other"]);
        // The newest sample describes the shared window best.
        assert_eq!(out[0].quota.as_ref().unwrap().at.timestamp(), 200);
    }

    #[test]
    fn a_second_login_keeps_its_own_row() {
        let out = merge_accounts(
            vec![row("acc-1", "local", Some(100))],
            vec![row("acc-2", "other", Some(200))],
        );
        assert_eq!(out.len(), 2);
    }

    #[test]
    fn an_older_sample_does_not_replace_a_newer_one() {
        let out = merge_accounts(
            vec![row("acc-1", "local", Some(300))],
            vec![row("acc-1", "other", Some(100))],
        );
        assert_eq!(out[0].quota.as_ref().unwrap().at.timestamp(), 300);
    }
}
