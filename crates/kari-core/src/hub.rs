//! The hub: one board over many nodes.
//!
//! The local node is the engine in this process. Every remote node is a
//! `kari-node` daemon reached through an SSH port forward, a direct address on
//! a private network, or a loopback port for tests. The hub keeps one thread
//! per remote node that holds the connection, follows the node's event stream
//! and caches its board. Card actions go to the node that owns the card.
//!
//! Columns live in the hub's own store. Only the primary hub pushes them, and
//! each node decides who the primary is: it keeps a lease, and refuses a push
//! from any other hub. "Make this device primary" takes the lease on every
//! node and adopts the columns found there, so a switch changes no columns.
//! A hub without a local engine, such as a phone, shows only remote nodes.

use crate::client::{ApiClient, EventItem};
use crate::model::*;
use crate::{keychain, launcher, paths, tunnel::Tunnel, Engine, Event};
use chrono::{DateTime, Utc};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use std::time::{Duration, Instant};
use tokio::sync::broadcast;
use tracing::{info, warn};

pub const LOCAL: &str = "local";
const INTENT_KEY: &str = "primary_intent";
/// Seconds between two renewals of a lease this hub holds.
const RENEW_EVERY: u64 = 60;

#[derive(Debug, Clone)]
pub enum HubEvent {
    BoardChanged {
        node_id: String,
    },
    Notice {
        node_id: String,
        node_name: String,
        title: String,
        body: String,
        card_id: Option<String>,
    },
}

#[derive(Default)]
struct RemoteState {
    online: bool,
    identity: Option<NodeIdentity>,
    last_seen: Option<DateTime<Utc>>,
    error: Option<String>,
    board: Option<BoardView>,
    client: Option<ApiClient>,
    tunnel: Option<Tunnel>,
    paired: bool,
    /// The node's column lease as it last reported it.
    lease: Option<Lease>,
}

struct Remote {
    rec: NodeRecord,
    state: Arc<Mutex<RemoteState>>,
    stop: Arc<AtomicBool>,
}

pub struct Hub {
    engine: Arc<Engine>,
    remotes: RwLock<Vec<Remote>>,
    tx: broadcast::Sender<HubEvent>,
    /// This hub's id and name, as the nodes record them in their lease.
    hub_id: String,
    hub_name: String,
    /// True when the engine in this process is a node on the board. A phone
    /// runs no Claude Code, so its engine is only the hub's store.
    with_local: bool,
    /// This hub wants to be primary. Set by `claim_primary`, cleared when a
    /// node reports another holder. Persisted, so a restart keeps the role.
    primary: AtomicBool,
}

impl Hub {
    /// A hub whose engine is also a node on the board: the desktop app.
    pub fn new(local: Arc<Engine>) -> Arc<Hub> {
        Self::build(local, true)
    }

    /// A hub whose engine is only its store: a device without Claude Code.
    pub fn without_local(store: Arc<Engine>) -> Arc<Hub> {
        Self::build(store, false)
    }

    fn build(local: Arc<Engine>, with_local: bool) -> Arc<Hub> {
        let (tx, _) = broadcast::channel(256);
        let hub_id = local.node_id();
        let hub_name = local.node_name();
        let hub = Arc::new(Hub {
            engine: Arc::clone(&local),
            remotes: RwLock::new(vec![]),
            tx,
            hub_id,
            hub_name,
            with_local,
            primary: AtomicBool::new(false),
        });
        hub.restore_intent();
        // Local engine events become hub events with the local node id.
        let h = Arc::clone(&hub);
        let mut rx = local.subscribe();
        std::thread::Builder::new()
            .name("kari-hub-local".into())
            .spawn(move || loop {
                match rx.blocking_recv() {
                    Ok(Event::BoardChanged) => h.emit(HubEvent::BoardChanged {
                        node_id: LOCAL.into(),
                    }),
                    // Another hub took the local node's lease.
                    Ok(Event::LeaseChanged) => {
                        if let Some(l) = h.engine.lease() {
                            if l.hub_id != h.hub_id {
                                h.lose_primary(&l.hub_name);
                            }
                        }
                        h.emit(HubEvent::BoardChanged {
                            node_id: LOCAL.into(),
                        });
                    }
                    Ok(Event::Notice {
                        title,
                        body,
                        card_id,
                    }) => h.emit(HubEvent::Notice {
                        node_id: LOCAL.into(),
                        node_name: h.engine.node_name(),
                        title,
                        body,
                        card_id,
                    }),
                    Err(broadcast::error::RecvError::Lagged(_)) => continue,
                    Err(_) => break,
                }
            })
            .expect("spawn");
        for rec in local.list_nodes() {
            hub.spawn_remote(rec);
        }
        hub.start_lease_keeper();
        hub
    }

    pub fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    pub fn hub_id(&self) -> &str {
        &self.hub_id
    }

    // ------------------------------------------------------------ primary lease

    /// True when this hub pushes columns.
    pub fn is_primary(&self) -> bool {
        self.primary.load(Ordering::Relaxed)
    }

    fn set_intent(&self, on: bool) {
        self.primary.store(on, Ordering::Relaxed);
        let _ = self.engine.kv_set(INTENT_KEY, if on { "1" } else { "0" });
    }

    fn claim(&self, take: bool) -> LeaseClaim {
        LeaseClaim {
            hub_id: self.hub_id.clone(),
            hub_name: self.hub_name.clone(),
            take,
        }
    }

    /// On start: a desktop whose local lease is free is primary, as every
    /// kari before the lease was. A stored intent wins over that default.
    fn restore_intent(&self) {
        let stored = self.engine.kv_get(INTENT_KEY);
        let want = match stored.as_deref() {
            Some("1") => true,
            Some(_) => false,
            None => self.with_local,
        };
        if want && self.with_local {
            match self.engine.claim_lease(self.claim(false)) {
                Ok(_) => self.primary.store(true, Ordering::Relaxed),
                Err(e) => {
                    info!("not primary on start: {e}");
                    self.set_intent(false);
                }
            }
        } else {
            self.primary.store(want, Ordering::Relaxed);
        }
    }

    /// A node reported another holder. Step back and tell the user once.
    fn lose_primary(&self, holder: &str) {
        if !self.is_primary() {
            return;
        }
        self.set_intent(false);
        warn!("lease lost to {holder}");
        self.emit(HubEvent::Notice {
            node_id: LOCAL.into(),
            node_name: self.hub_name.clone(),
            title: format!("{holder} is primary now"),
            body:
                "This device follows. Columns are read-only here until you make it primary again."
                    .into(),
            card_id: None,
        });
    }

    /// Become the hub that pushes columns: take the lease on the local node and
    /// on every online remote node, then adopt the columns the previous primary
    /// left on the nodes. Offline nodes get the claim when they reconnect.
    pub fn claim_primary(&self) -> anyhow::Result<String> {
        self.set_intent(true);
        let mut taken = 0usize;
        let mut failed = vec![];
        if self.with_local {
            match self.engine.claim_lease(self.claim(true)) {
                Ok(_) => taken += 1,
                Err(e) => failed.push(format!("this machine: {e}")),
            }
        }
        // The columns to adopt: from the node whose foreign lease was renewed last.
        let mut adopt: Option<(DateTime<Utc>, ApiClient)> = None;
        let remotes: Vec<(String, Arc<Mutex<RemoteState>>)> = self
            .remotes
            .read()
            .unwrap()
            .iter()
            .map(|r| (r.rec.id.clone(), Arc::clone(&r.state)))
            .collect();
        for (id, state) in remotes {
            let (client, previous) = {
                let st = state.lock().unwrap();
                match (&st.client, st.online) {
                    (Some(c), true) => (c.clone(), st.lease.clone()),
                    _ => continue,
                }
            };
            match client.claim_lease(&self.claim(true)) {
                Ok(l) => {
                    taken += 1;
                    state.lock().unwrap().lease = Some(l);
                    if let Some(p) = previous.filter(|p| p.hub_id != self.hub_id) {
                        if adopt.as_ref().is_none_or(|(at, _)| p.renewed_at > *at) {
                            adopt = Some((p.renewed_at, client.clone()));
                        }
                    }
                }
                Err(e) => failed.push(format!("{id}: {e}")),
            }
        }
        if let Some((_, client)) = adopt {
            match client.columns() {
                Ok(cols) if !cols.is_empty() => {
                    if let Err(e) = self.engine.set_columns(cols) {
                        warn!("columns not adopted: {e}");
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("columns not read from the previous primary: {e}"),
            }
        }
        self.push_columns();
        self.emit(HubEvent::BoardChanged {
            node_id: LOCAL.into(),
        });
        if failed.is_empty() {
            Ok(format!("primary on {taken} node(s)"))
        } else {
            Ok(format!(
                "primary on {taken} node(s); not yet on {}",
                failed.join(", ")
            ))
        }
    }

    /// Renew the local lease while this hub is primary. Remote leases renew in
    /// their node threads.
    fn start_lease_keeper(self: &Arc<Self>) {
        if !self.with_local {
            return;
        }
        let h = Arc::clone(self);
        std::thread::Builder::new()
            .name("kari-hub-lease".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_secs(RENEW_EVERY));
                if h.is_primary() {
                    if let Err(e) = h.engine.claim_lease(h.claim(false)) {
                        let holder = h
                            .engine
                            .lease()
                            .map(|l| l.hub_name)
                            .unwrap_or_else(|| "another hub".into());
                        warn!("local lease renewal refused: {e}");
                        h.lose_primary(&holder);
                    }
                }
            })
            .expect("spawn");
    }

    /// Whether this hub may push columns to a node with the given lease.
    fn lease_ours(&self, lease: &Option<Lease>) -> bool {
        match lease {
            None => true,
            Some(l) => l.hub_id == self.hub_id || l.expired(Utc::now()),
        }
    }

    /// The engine of this machine, for the parts of the app that are local
    /// only: columns, settings, hooks, calibration.
    pub fn engine(&self) -> &Arc<Engine> {
        &self.engine
    }

    fn emit(&self, e: HubEvent) {
        let _ = self.tx.send(e);
    }

    // ------------------------------------------------------------ node threads

    fn spawn_remote(self: &Arc<Self>, rec: NodeRecord) {
        let state = Arc::new(Mutex::new(RemoteState {
            paired: keychain::load_token(&rec.id).is_some(),
            ..Default::default()
        }));
        // An offline node still shows its last board.
        if let Some((board, at)) = self.engine.node_cache(&rec.id) {
            let mut st = state.lock().unwrap();
            st.board = Some(board);
            st.last_seen = Some(at);
        }
        let stop = Arc::new(AtomicBool::new(false));
        if rec.enabled {
            let h = Arc::clone(self);
            let (r, s, f) = (rec.clone(), Arc::clone(&state), Arc::clone(&stop));
            std::thread::Builder::new()
                .name(format!("kari-node-{}", rec.id))
                .spawn(move || h.run_remote(r, s, f))
                .expect("spawn");
        }
        self.remotes
            .write()
            .unwrap()
            .push(Remote { rec, state, stop });
    }

    fn stop_remote(&self, id: &str) -> Option<NodeRecord> {
        let mut rs = self.remotes.write().unwrap();
        let pos = rs.iter().position(|r| r.rec.id == id)?;
        let r = rs.remove(pos);
        r.stop.store(true, Ordering::Relaxed);
        let mut st = r.state.lock().unwrap();
        st.tunnel.take();
        st.client.take();
        st.online = false;
        Some(r.rec)
    }

    fn set_error(state: &Mutex<RemoteState>, msg: impl Into<String>) {
        let mut st = state.lock().unwrap();
        st.online = false;
        st.client = None;
        st.tunnel = None;
        st.error = Some(msg.into());
    }

    /// One connection attempt after another, with backoff, until stopped.
    fn run_remote(
        self: Arc<Self>,
        rec: NodeRecord,
        state: Arc<Mutex<RemoteState>>,
        stop: Arc<AtomicBool>,
    ) {
        let mut backoff = 1u64;
        let mut rec = rec;
        let label = rec
            .ssh_host
            .clone()
            .or_else(|| rec.address.clone())
            .unwrap_or_else(|| format!("127.0.0.1:{}", rec.remote_port));
        while !stop.load(Ordering::Relaxed) {
            let started = Instant::now();
            // A learned address, or a changed one, applies to the next attempt.
            if let Some(fresh) = self.record_of(&rec.id) {
                rec = fresh;
            }
            match self.connect_once(&rec, &state, &stop) {
                Ok(()) => {}
                Err(e) => {
                    warn!("node {label}: {e}");
                    Self::set_error(&state, e.to_string());
                }
            }
            self.emit(HubEvent::BoardChanged {
                node_id: rec.id.clone(),
            });
            if stop.load(Ordering::Relaxed) {
                break;
            }
            if started.elapsed() > Duration::from_secs(60) {
                backoff = 1;
            }
            let wait = backoff;
            backoff = (backoff * 2).min(60);
            for _ in 0..wait * 4 {
                if stop.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(250));
            }
        }
    }

    /// Open the forward, wait for health, push columns, then follow the event
    /// stream until it ends. Returns Ok after a clean stream end.
    fn connect_once(
        &self,
        rec: &NodeRecord,
        state: &Arc<Mutex<RemoteState>>,
        stop: &AtomicBool,
    ) -> anyhow::Result<()> {
        let Some(token) = keychain::load_token(&rec.id) else {
            state.lock().unwrap().paired = false;
            anyhow::bail!("not paired: no token for this node; use Pair in Settings");
        };
        state.lock().unwrap().paired = true;
        // Three ways to reach a node: an SSH forward, one of its addresses on a
        // private network, or a loopback port in a test. A direct connection
        // tries every candidate address, so a node that moved is found again.
        let mut probed = None;
        let base = match &rec.ssh_host {
            Some(host) => {
                let t = Tunnel::open(host, rec.remote_port)?;
                let p = t.local_port;
                state.lock().unwrap().tunnel = Some(t);
                format!("http://127.0.0.1:{p}")
            }
            None if !Self::candidates(rec, None).is_empty() => {
                let (addr, id) = self.pick_address(rec, &token, stop)?;
                probed = Some(id);
                format!("http://{addr}")
            }
            None => format!("http://127.0.0.1:{}", rec.remote_port),
        };
        let client = ApiClient::at(&base, &token).with_hub(&self.hub_id);
        // The forward needs a moment. Poll health, and give up when ssh exits.
        let deadline = Instant::now() + Duration::from_secs(20);
        let identity = match probed {
            Some(id) => id,
            None => loop {
                if stop.load(Ordering::Relaxed) {
                    anyhow::bail!("stopped");
                }
                match client.health() {
                    Ok(id) => break id,
                    Err(e) => {
                        let mut st = state.lock().unwrap();
                        if let Some(t) = st.tunnel.as_mut() {
                            if !t.alive() {
                                let msg = t.exit_message().unwrap_or_else(|| "ssh exited".into());
                                drop(st);
                                anyhow::bail!("ssh forward failed: {msg}");
                            }
                        }
                        drop(st);
                        if Instant::now() > deadline {
                            anyhow::bail!("node did not answer health: {e}");
                        }
                        std::thread::sleep(Duration::from_millis(500));
                    }
                }
            },
        };
        // The node says where else it answers. Keep it: this is how the desktop
        // learns a node's private address without a typed one, and how a
        // pairing code can carry it to a phone.
        let working = base
            .strip_prefix("http://")
            .filter(|_| rec.ssh_host.is_none());
        self.remember_addresses(&rec.id, working, &identity.addresses);
        if identity.api_version != API_VERSION {
            anyhow::bail!(
                "node speaks api v{} but this app needs v{API_VERSION}; update kari on one side",
                identity.api_version
            );
        }
        // The token must open a guarded route too, else the pairing is stale.
        let board = client
            .board()
            .map_err(|e| anyhow::anyhow!("token rejected or board failed: {e}"))?;
        // The lease decides who pushes columns. A primary hub renews or takes
        // its claim on connect; a follower only reads.
        let mut lease = client.lease().unwrap_or_default();
        if self.is_primary() {
            match client.claim_lease(&self.claim(false)) {
                Ok(l) => lease = Some(l),
                Err(e) => {
                    let holder = lease
                        .as_ref()
                        .map(|l| l.hub_name.clone())
                        .unwrap_or_else(|| "another hub".into());
                    warn!("node {}: {e}", identity.node_name);
                    self.lose_primary(&holder);
                }
            }
        }
        if self.is_primary() && self.lease_ours(&lease) {
            if let Err(e) = client.set_columns(&self.engine.columns()) {
                warn!("node {}: columns not pushed: {e}", identity.node_name);
            }
        }
        {
            let mut st = state.lock().unwrap();
            st.online = true;
            st.error = None;
            st.identity = Some(identity.clone());
            st.client = Some(client.clone());
            st.last_seen = Some(Utc::now());
            st.board = Some(board.clone());
            st.lease = lease;
        }
        self.engine.save_node_cache(&rec.id, &board);
        info!("node {} online ({})", identity.node_name, identity.version);
        self.emit(HubEvent::BoardChanged {
            node_id: rec.id.clone(),
        });
        let node_name = self.node_name_of(rec, Some(&identity));

        let mut reader = client.events()?;
        let mut last_fetch = Instant::now();
        let mut last_renew = Instant::now();
        let mut pending = false;
        loop {
            if stop.load(Ordering::Relaxed) {
                return Ok(());
            }
            let Some(item) = reader.recv() else {
                anyhow::bail!("event stream ended");
            };
            state.lock().unwrap().last_seen = Some(Utc::now());
            if self.is_primary() && last_renew.elapsed() > Duration::from_secs(RENEW_EVERY) {
                last_renew = Instant::now();
                match client.claim_lease(&self.claim(false)) {
                    Ok(l) => state.lock().unwrap().lease = Some(l),
                    Err(e) => {
                        let holder = client
                            .lease()
                            .ok()
                            .flatten()
                            .map(|l| l.hub_name)
                            .unwrap_or_else(|| "another hub".into());
                        warn!("node {}: renewal refused: {e}", node_name);
                        self.lose_primary(&holder);
                    }
                }
            }
            match item {
                EventItem::Message(m) if m.event == "lease_changed" => {
                    let lease = client.lease().unwrap_or_default();
                    if let Some(l) = lease.as_ref().filter(|l| l.hub_id != self.hub_id) {
                        if !l.expired(Utc::now()) {
                            self.lose_primary(&l.hub_name);
                        }
                    }
                    state.lock().unwrap().lease = lease;
                    self.emit(HubEvent::BoardChanged {
                        node_id: rec.id.clone(),
                    });
                }
                EventItem::Message(m) if m.event == "notice" => {
                    let v: serde_json::Value = serde_json::from_str(&m.data).unwrap_or_default();
                    self.emit(HubEvent::Notice {
                        node_id: rec.id.clone(),
                        node_name: node_name.clone(),
                        title: v["title"].as_str().unwrap_or("kari").to_string(),
                        body: v["body"].as_str().unwrap_or("").to_string(),
                        card_id: v["card_id"].as_str().map(|s| s.to_string()),
                    });
                }
                EventItem::Message(_) => pending = true,
                EventItem::KeepAlive => {
                    // A quiet node still gets a fresh board once a minute.
                    if last_fetch.elapsed() > Duration::from_secs(60) {
                        pending = true;
                    }
                }
            }
            // Coalesce bursts: one fetch per 300 ms at most.
            if pending && last_fetch.elapsed() > Duration::from_millis(300) {
                pending = false;
                last_fetch = Instant::now();
                self.fetch_board(rec, state, &client)?;
            }
        }
    }

    fn fetch_board(
        &self,
        rec: &NodeRecord,
        state: &Mutex<RemoteState>,
        client: &ApiClient,
    ) -> anyhow::Result<()> {
        let board = client.board()?;
        {
            let mut st = state.lock().unwrap();
            st.board = Some(board.clone());
            st.last_seen = Some(Utc::now());
        }
        self.engine.save_node_cache(&rec.id, &board);
        self.emit(HubEvent::BoardChanged {
            node_id: rec.id.clone(),
        });
        Ok(())
    }

    /// Fetch a node's board now, after an action on it. Best effort.
    fn refetch(&self, node_id: &str) {
        let (rec, state, client) = {
            let rs = self.remotes.read().unwrap();
            let Some(r) = rs.iter().find(|r| r.rec.id == node_id) else {
                return;
            };
            let st = r.state.lock().unwrap();
            let Some(c) = st.client.clone() else {
                return;
            };
            (r.rec.clone(), Arc::clone(&r.state), c)
        };
        if let Err(e) = self.fetch_board(&rec, &state, &client) {
            warn!("node refetch: {e}");
        }
    }

    // ------------------------------------------------------------ status

    fn node_name_of(&self, rec: &NodeRecord, identity: Option<&NodeIdentity>) -> String {
        if !rec.name.trim().is_empty() {
            return rec.name.trim().to_string();
        }
        if let Some(h) = &rec.ssh_host {
            return h.clone();
        }
        identity
            .map(|i| i.node_name.clone())
            .unwrap_or_else(|| format!("node {}", &rec.id[..8.min(rec.id.len())]))
    }

    /// Every address worth trying for a node, best first: the one in use, then
    /// the ones it answered on before, then the ones it advertises now.
    fn candidates(rec: &NodeRecord, identity: Option<&NodeIdentity>) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let mut push = |a: &str| {
            let a = a.trim();
            if !a.is_empty() && !out.iter().any(|x| x == a) {
                out.push(a.to_string());
            }
        };
        if let Some(a) = &rec.address {
            push(a);
        }
        for a in &rec.addresses {
            push(a);
        }
        for a in identity.map(|i| i.addresses.as_slice()).unwrap_or(&[]) {
            push(a);
        }
        out
    }

    /// Keep what the node told us about itself: the address that answered, and
    /// the ones it advertises. The next connection starts from the working one,
    /// and a pairing code carries the list to a phone.
    fn remember_addresses(&self, id: &str, working: Option<&str>, advertised: &[String]) {
        let mut rs = self.remotes.write().unwrap();
        let Some(r) = rs.iter_mut().find(|r| r.rec.id == id) else {
            return;
        };
        let before = r.rec.clone();
        if let Some(w) = working {
            r.rec.address = Some(w.to_string());
        }
        let mut list = Self::candidates(&r.rec, None);
        for a in advertised {
            let a = a.trim();
            if !a.is_empty() && !list.iter().any(|x| x == a) {
                list.push(a.to_string());
            }
        }
        r.rec.addresses = list;
        if r.rec == before {
            return;
        }
        let rec = r.rec.clone();
        drop(rs);
        if let Err(e) = self.engine.save_node(&rec) {
            warn!("node {id}: addresses not saved: {e}");
        }
    }

    fn record_of(&self, id: &str) -> Option<NodeRecord> {
        self.remotes
            .read()
            .unwrap()
            .iter()
            .find(|r| r.rec.id == id)
            .map(|r| r.rec.clone())
    }

    /// The first candidate address that answers, with what it answered.
    fn pick_address(
        &self,
        rec: &NodeRecord,
        token: &str,
        stop: &AtomicBool,
    ) -> anyhow::Result<(String, NodeIdentity)> {
        let cands = Self::candidates(rec, None);
        let mut last = String::from("no address to try");
        for addr in &cands {
            if stop.load(Ordering::Relaxed) {
                anyhow::bail!("stopped");
            }
            let client = ApiClient::at(&format!("http://{addr}"), token);
            match client.probe(3) {
                Ok(id) => return Ok((addr.clone(), id)),
                Err(e) => last = format!("{addr}: {e}"),
            }
        }
        anyhow::bail!(
            "no address answered ({} tried); last error {last}",
            cands.len()
        )
    }

    fn local_status(&self) -> NodeStatus {
        let lease = self.engine.lease();
        NodeStatus {
            id: LOCAL.into(),
            name: self.engine.node_name(),
            kind: "local".into(),
            online: true,
            enabled: true,
            paired: true,
            ssh_host: None,
            address: None,
            remote_port: self.engine.settings().hooks_port,
            version: Some(crate::version().into()),
            api_version: Some(API_VERSION),
            remote_node_id: Some(self.engine.node_id()),
            last_seen: Some(Utc::now()),
            error: None,
            primary: lease.as_ref().is_some_and(|l| l.hub_id == self.hub_id),
            lease,
            away_mode: self.engine.settings().away_mode,
            addresses: crate::net::bound_reachable(),
        }
    }

    fn status_of(&self, r: &Remote) -> NodeStatus {
        let st = r.state.lock().unwrap();
        NodeStatus {
            id: r.rec.id.clone(),
            name: self.node_name_of(&r.rec, st.identity.as_ref()),
            kind: "remote".into(),
            online: st.online,
            enabled: r.rec.enabled,
            paired: st.paired,
            ssh_host: r.rec.ssh_host.clone(),
            address: r.rec.address.clone(),
            remote_port: r.rec.remote_port,
            version: st.identity.as_ref().map(|i| i.version.clone()),
            api_version: st.identity.as_ref().map(|i| i.api_version),
            remote_node_id: st.identity.as_ref().map(|i| i.node_id.clone()),
            last_seen: st.last_seen,
            error: if r.rec.enabled {
                st.error.clone()
            } else {
                Some("disabled".into())
            },
            primary: st
                .lease
                .as_ref()
                .is_some_and(|l| l.hub_id == self.hub_id && !l.expired(Utc::now())),
            lease: st.lease.clone(),
            away_mode: st.board.as_ref().is_some_and(|b| b.away_mode),
            addresses: Self::candidates(&r.rec, st.identity.as_ref()),
        }
    }

    pub fn nodes(&self) -> Vec<NodeStatus> {
        let mut out = vec![];
        if self.with_local {
            out.push(self.local_status());
        }
        for r in self.remotes.read().unwrap().iter() {
            out.push(self.status_of(r));
        }
        out
    }

    // ------------------------------------------------------------ board

    /// A remote card keeps its column when the local board has it, else the
    /// column that accepts its state.
    fn map_column(columns: &[Column], view: &CardView) -> String {
        if columns.iter().any(|c| c.id == view.column_id) {
            return view.column_id.clone();
        }
        let by_state = columns
            .iter()
            .filter(|c| !c.hidden)
            .find(|c| c.accepts.contains(&view.state))
            .or_else(|| {
                columns
                    .iter()
                    .find(|c| c.accepts.contains(&DerivedState::Unknown))
            })
            .or_else(|| columns.first());
        by_state.map(|c| c.id.clone()).unwrap_or_default()
    }

    pub fn board(&self) -> HubBoard {
        let columns = self.engine.columns();
        let local_name = self.engine.node_name();
        let mut nodes = vec![];
        let mut cards: Vec<HubCard> = vec![];
        let mut quotas = vec![];
        let mut proposals: Vec<NodeProposal> = vec![];
        // The local board is the whole engine scan; a hub without a local node skips it.
        let lb = if self.with_local {
            Some(self.engine.board())
        } else {
            None
        };
        if let Some(lb) = &lb {
            nodes.push(self.local_status());
            cards.extend(lb.cards.iter().cloned().map(|view| HubCard {
                node_id: LOCAL.into(),
                node_name: local_name.clone(),
                view,
            }));
            quotas.push(NodeQuota {
                node_id: LOCAL.into(),
                node_name: local_name.clone(),
                quota: lb.quota.clone(),
                calibration: lb.calibration.clone(),
            });
            proposals.extend(lb.proposal.clone().map(|p| NodeProposal {
                node_id: LOCAL.into(),
                node_name: local_name.clone(),
                proposal: p,
            }));
        }
        for r in self.remotes.read().unwrap().iter() {
            let status = self.status_of(r);
            let st = r.state.lock().unwrap();
            if r.rec.enabled {
                if let Some(b) = &st.board {
                    for view in &b.cards {
                        let mut v = view.clone();
                        v.column_id = Self::map_column(&columns, &v);
                        cards.push(HubCard {
                            node_id: r.rec.id.clone(),
                            node_name: status.name.clone(),
                            view: v,
                        });
                    }
                    quotas.push(NodeQuota {
                        node_id: r.rec.id.clone(),
                        node_name: status.name.clone(),
                        quota: b.quota.clone(),
                        calibration: b.calibration.clone(),
                    });
                    if let Some(p) = &b.proposal {
                        proposals.push(NodeProposal {
                            node_id: r.rec.id.clone(),
                            node_name: status.name.clone(),
                            proposal: p.clone(),
                        });
                    }
                }
            }
            drop(st);
            nodes.push(status);
        }
        HubBoard {
            columns,
            hub_id: self.hub_id.clone(),
            hub_name: self.hub_name.clone(),
            primary: self.is_primary(),
            nodes,
            cards,
            quotas,
            proposals,
            generated_at: Utc::now(),
            scanning: lb.as_ref().is_some_and(|b| b.scanning),
            herdr_connected: lb.as_ref().is_some_and(|b| b.herdr_connected),
            hooks_installed: lb.as_ref().is_some_and(|b| b.hooks_installed),
            hooks_port: lb
                .as_ref()
                .map(|b| b.hooks_port)
                .unwrap_or_else(|| self.engine.settings().hooks_port),
        }
    }

    pub fn refresh_all(&self) {
        if self.with_local {
            let e = Arc::clone(&self.engine);
            std::thread::spawn(move || e.refresh_all());
        }
        for c in self.online_clients() {
            let _ = c.1.refresh();
        }
    }

    fn online_clients(&self) -> Vec<(String, ApiClient)> {
        self.remotes
            .read()
            .unwrap()
            .iter()
            .filter_map(|r| {
                let st = r.state.lock().unwrap();
                st.client
                    .clone()
                    .filter(|_| st.online)
                    .map(|c| (r.rec.id.clone(), c))
            })
            .collect()
    }

    fn client_of(&self, node_id: &str) -> anyhow::Result<(ApiClient, NodeRecord)> {
        let rs = self.remotes.read().unwrap();
        let Some(r) = rs.iter().find(|r| r.rec.id == node_id) else {
            anyhow::bail!("unknown node {node_id}")
        };
        let st = r.state.lock().unwrap();
        match (&st.client, st.online) {
            (Some(c), true) => Ok((c.clone(), r.rec.clone())),
            _ => anyhow::bail!(
                "node {} is offline{}",
                self.node_name_of(&r.rec, st.identity.as_ref()),
                st.error
                    .as_ref()
                    .map(|e| format!(": {e}"))
                    .unwrap_or_default()
            ),
        }
    }

    /// Run an action on the node that owns a card.
    fn on_node<T>(
        &self,
        node_id: &str,
        local: impl FnOnce(&Arc<Engine>) -> anyhow::Result<T>,
        remote: impl FnOnce(&ApiClient) -> anyhow::Result<T>,
    ) -> anyhow::Result<T> {
        if node_id == LOCAL || node_id.is_empty() {
            if !self.with_local {
                anyhow::bail!("this device runs no Claude Code; pick a node");
            }
            return local(&self.engine);
        }
        let (c, _) = self.client_of(node_id)?;
        let out = remote(&c)?;
        self.refetch(node_id);
        Ok(out)
    }

    // ------------------------------------------------------------ card actions

    pub fn move_card(&self, node: &str, card: &str, column: &str) -> anyhow::Result<()> {
        self.on_node(
            node,
            |e| e.move_card(card, column),
            |c| c.move_card(card, column),
        )
    }

    pub fn add_task(&self, node: &str, t: NewTask) -> anyhow::Result<Card> {
        let t2 = t.clone();
        self.on_node(node, move |e| e.add_task(t), move |c| c.add_task(&t2))
    }

    pub fn patch_card(&self, node: &str, card: &str, p: CardPatch) -> anyhow::Result<Card> {
        let p2 = p.clone();
        self.on_node(
            node,
            move |e| e.patch_card(card, p),
            move |c| c.patch_card(card, &p2),
        )
    }

    pub fn delete_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        self.on_node(node, |e| e.delete_card(card), |c| c.delete_card(card))
    }

    pub fn start_card(
        &self,
        node: &str,
        card: &str,
        prompt: Option<String>,
    ) -> anyhow::Result<String> {
        let p2 = prompt.clone();
        self.on_node(
            node,
            move |e| e.start_card(card, prompt),
            move |c| c.start_card(card, p2),
        )
    }

    pub fn stop_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        self.on_node(node, |e| e.stop_card(card), |c| c.stop_card(card))
    }

    /// Answer a permission prompt a node holds: `allow` or `deny`.
    pub fn answer_permission(&self, node: &str, id: &str, behavior: &str) -> anyhow::Result<()> {
        self.on_node(
            node,
            |e| e.answer_permission(id, behavior),
            |c| c.answer_permission(id, behavior),
        )
    }

    /// Hold permission prompts on a node for a remote answer, or stop holding them.
    pub fn set_away_mode(&self, node: &str, on: bool) -> anyhow::Result<()> {
        self.on_node(node, |e| e.set_away_mode(on), |c| c.set_away_mode(on))
    }

    pub fn summarize_card(&self, node: &str, card: &str) -> anyhow::Result<Summary> {
        self.on_node(node, |e| e.summarize_card(card), |c| c.summarize_card(card))
    }

    pub fn job_log(&self, node: &str, card: &str, limit: usize) -> Vec<JobLogEntry> {
        if node == LOCAL && self.with_local {
            return self.engine.job_log(card, limit);
        }
        self.client_of(node)
            .and_then(|(c, _)| c.job_log(card, limit))
            .unwrap_or_default()
    }

    pub fn projects(&self, node: &str) -> Vec<(String, String)> {
        if node == LOCAL && self.with_local {
            return self.engine.projects();
        }
        self.client_of(node)
            .and_then(|(c, _)| c.projects())
            .unwrap_or_default()
    }

    pub fn quota_history(&self, node: &str, limit: usize) -> Vec<QuotaSample> {
        if node == LOCAL && self.with_local {
            return self.engine.quota_history(limit);
        }
        self.client_of(node)
            .and_then(|(c, _)| c.quota_history(limit))
            .unwrap_or_default()
    }

    /// Jump in where the user sits. A remote card opens a local terminal that
    /// runs the node's plan over SSH.
    pub fn jump_in(&self, node: &str, card: &str) -> anyhow::Result<String> {
        if node == LOCAL && self.with_local {
            return self.engine.jump_in(card);
        }
        let (c, rec) = self.client_of(node)?;
        let plan = c.jump(card)?;
        let Some(host) = rec.ssh_host.as_deref() else {
            anyhow::bail!(
                "node has no SSH host; run there: cd {} && {}",
                plan.cwd,
                plan.command
            )
        };
        let settings = self.engine.settings();
        let home = paths::home().to_string_lossy().into_owned();
        let cmd = if plan.command.is_empty() {
            // The node focused a herdr pane. Land the user in a shell there.
            format!("ssh -t {}", launcher::sh_quote(host))
        } else {
            launcher::ssh_command(host, &plan.cwd, &plan.command)
        };
        launcher::open_in_terminal(&settings.terminal_app, &home, &cmd)?;
        let name = self.node_name_of(&rec, None);
        Ok(format!("{} on {name}", plan.message))
    }

    // ------------------------------------------------------------ proposals

    pub fn propose_now(&self, node: &str) -> anyhow::Result<Proposal> {
        self.on_node(node, |e| e.propose_now(), |c| c.propose_now())
    }

    pub fn proposal(&self, node: &str) -> Option<Proposal> {
        if node == LOCAL && self.with_local {
            return self.engine.proposal();
        }
        self.client_of(node)
            .ok()
            .and_then(|(c, _)| c.proposal().ok().flatten())
    }

    pub fn proposal_history(&self, node: &str, limit: usize) -> Vec<Proposal> {
        if node == LOCAL && self.with_local {
            return self.engine.proposal_history(limit);
        }
        self.client_of(node)
            .and_then(|(c, _)| c.proposal_history(limit))
            .unwrap_or_default()
    }

    pub fn accept_proposal(
        &self,
        node: &str,
        id: &str,
        card_ids: Option<Vec<String>>,
    ) -> anyhow::Result<usize> {
        let ids2 = card_ids.clone();
        self.on_node(
            node,
            move |e| e.accept_proposal(id, card_ids, false),
            move |c| c.accept_proposal(id, ids2),
        )
    }

    pub fn snooze_proposal(&self, node: &str, id: &str, minutes: i64) -> anyhow::Result<()> {
        self.on_node(
            node,
            |e| e.snooze_proposal(id, minutes),
            |c| c.snooze_proposal(id, minutes),
        )
    }

    pub fn dismiss_proposal(&self, node: &str, id: &str) -> anyhow::Result<()> {
        self.on_node(node, |e| e.dismiss_proposal(id), |c| c.dismiss_proposal(id))
    }

    pub fn stop_proposal(&self, node: &str, id: &str) -> anyhow::Result<usize> {
        self.on_node(node, |e| e.stop_proposal(id), |c| c.stop_proposal(id))
    }

    /// The kill switch: every kari-started job on every online node.
    pub fn stop_all(&self) -> anyhow::Result<usize> {
        let mut n = if self.with_local {
            self.engine.stop_all()?
        } else {
            0
        };
        for (id, c) in self.online_clients() {
            match c.stop_all() {
                Ok(k) => n += k,
                Err(e) => warn!("stop all on {id}: {e}"),
            }
            self.refetch(&id);
        }
        Ok(n)
    }

    // ------------------------------------------------------------ columns

    /// Push the hub's columns to every online node whose lease is this hub's.
    /// A node that refuses with 409 has a new primary; this hub steps back.
    fn push_columns(&self) {
        if !self.is_primary() {
            return;
        }
        let cols = self.engine.columns();
        let targets: Vec<(String, ApiClient, Option<Lease>)> = self
            .remotes
            .read()
            .unwrap()
            .iter()
            .filter_map(|r| {
                let st = r.state.lock().unwrap();
                st.client
                    .clone()
                    .filter(|_| st.online)
                    .map(|c| (r.rec.id.clone(), c, st.lease.clone()))
            })
            .collect();
        for (id, c, lease) in targets {
            if !self.lease_ours(&lease) {
                continue;
            }
            if let Err(e) = c.set_columns(&cols) {
                warn!("columns not pushed to {id}: {e}");
                if e.to_string().contains("not primary") {
                    let holder = lease
                        .map(|l| l.hub_name)
                        .unwrap_or_else(|| "another hub".into());
                    self.lose_primary(&holder);
                    return;
                }
            }
            self.refetch(&id);
        }
    }

    /// Who holds the columns, for an error message: the first lease on any
    /// node that names another hub.
    fn holder_name(&self) -> String {
        let foreign = |l: Lease| (l.hub_id != self.hub_id).then_some(l.hub_name);
        if self.with_local {
            if let Some(n) = self.engine.lease().and_then(foreign) {
                return n;
            }
        }
        self.remotes
            .read()
            .unwrap()
            .iter()
            .find_map(|r| r.state.lock().unwrap().lease.clone().and_then(foreign))
            .unwrap_or_else(|| "another hub".into())
    }

    pub fn set_columns(&self, cols: Vec<Column>) -> anyhow::Result<()> {
        if !self.is_primary() {
            anyhow::bail!(
                "not primary: {} holds the columns; use Make this device primary first",
                self.holder_name()
            );
        }
        self.engine.set_columns(cols)?;
        self.push_columns();
        Ok(())
    }

    pub fn reset_columns(&self) -> anyhow::Result<()> {
        if !self.is_primary() {
            anyhow::bail!(
                "not primary: {} holds the columns; use Make this device primary first",
                self.holder_name()
            );
        }
        self.engine.reset_columns()?;
        self.push_columns();
        Ok(())
    }

    // ------------------------------------------------------------ pairing

    /// A pairing code for another hub, such as a phone: every node this hub
    /// knows, with its address and token, plus this machine when it listens on
    /// a second address. Nodes reached over SSH carry no address; the other
    /// hub asks for one. The code holds tokens: show it at home, on demand.
    /// What a phone needs to reach every node this hub knows: the names, the
    /// tokens, and every address the nodes answer on. Version 2 carries a list
    /// per node, so the phone types no address at all.
    pub fn pairing_code(&self) -> anyhow::Result<String> {
        let mut nodes = vec![];
        if self.with_local {
            nodes.push(serde_json::json!({
                "name": self.engine.node_name(),
                "addresses": crate::net::bound_reachable(),
                "token": crate::hooks::token()?,
            }));
        }
        for r in self.remotes.read().unwrap().iter() {
            let Some(token) = keychain::load_token(&r.rec.id) else {
                continue;
            };
            let st = r.state.lock().unwrap();
            nodes.push(serde_json::json!({
                "name": self.node_name_of(&r.rec, st.identity.as_ref()),
                "addresses": Self::candidates(&r.rec, st.identity.as_ref()),
                "token": token,
            }));
        }
        Ok(serde_json::to_string(
            &serde_json::json!({ "kari": 2, "nodes": nodes }),
        )?)
    }

    // ------------------------------------------------------------ node management

    fn status_by_id(&self, id: &str) -> anyhow::Result<NodeStatus> {
        self.nodes()
            .into_iter()
            .find(|n| n.id == id)
            .ok_or_else(|| anyhow::anyhow!("unknown node {id}"))
    }

    /// Add a node and pair it at once when it has an SSH host. A failed pairing
    /// still saves the node; the status shows why.
    pub fn add_node(self: &Arc<Self>, n: NewNode) -> anyhow::Result<NodeStatus> {
        let ssh_host = n
            .ssh_host
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let address = n
            .address
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let mut addresses: Vec<String> = Vec::new();
        for a in address.iter().cloned().chain(n.addresses) {
            let a = a.trim().to_string();
            if !a.is_empty() && !addresses.contains(&a) {
                addresses.push(a);
            }
        }
        let address = address.or_else(|| addresses.first().cloned());
        let rec = NodeRecord {
            id: uuid::Uuid::new_v4().to_string(),
            name: n.name.trim().to_string(),
            ssh_host,
            address,
            addresses,
            remote_port: if n.remote_port == 0 {
                47311
            } else {
                n.remote_port
            },
            enabled: true,
            created_at: Utc::now(),
        };
        self.engine.save_node(&rec)?;
        let token = n
            .token
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty());
        let pair_error = match (token, &rec.ssh_host) {
            // The caller brought the token, for example from a pairing code.
            (Some(tok), _) => keychain::store_token(&rec.id, &tok)
                .err()
                .map(|e| e.to_string()),
            (None, Some(host)) => match crate::tunnel::read_remote_token(host) {
                Ok(tok) => keychain::store_token(&rec.id, &tok)
                    .err()
                    .map(|e| e.to_string()),
                Err(e) => Some(e.to_string()),
            },
            (None, None) => None,
        };
        self.spawn_remote(rec.clone());
        if let Some(e) = pair_error {
            if let Some(r) = self
                .remotes
                .read()
                .unwrap()
                .iter()
                .find(|r| r.rec.id == rec.id)
            {
                r.state.lock().unwrap().error = Some(format!("pairing failed: {e}"));
            }
        }
        self.emit(HubEvent::BoardChanged {
            node_id: rec.id.clone(),
        });
        self.status_by_id(&rec.id)
    }

    /// Change a node. A changed host or port reconnects; a disabled node stops.
    pub fn update_node(self: &Arc<Self>, id: &str, p: NodePatch) -> anyhow::Result<NodeStatus> {
        let Some(mut rec) = self.stop_remote(id) else {
            anyhow::bail!("unknown node {id}")
        };
        if let Some(n) = p.name {
            rec.name = n.trim().to_string();
        }
        if let Some(h) = p.ssh_host {
            rec.ssh_host = h.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        }
        if let Some(a) = p.address {
            rec.address = a.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
            if let Some(a) = &rec.address {
                if !rec.addresses.contains(a) {
                    rec.addresses.insert(0, a.clone());
                }
            }
        }
        if let Some(list) = p.addresses {
            rec.addresses = list
                .into_iter()
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
        }
        if let Some(port) = p.remote_port {
            if port != 0 {
                rec.remote_port = port;
            }
        }
        if let Some(en) = p.enabled {
            rec.enabled = en;
        }
        self.engine.save_node(&rec)?;
        self.spawn_remote(rec);
        self.emit(HubEvent::BoardChanged {
            node_id: id.to_string(),
        });
        self.status_by_id(id)
    }

    pub fn remove_node(&self, id: &str) -> anyhow::Result<()> {
        self.stop_remote(id);
        self.engine.delete_node(id)?;
        keychain::delete_token(id);
        self.emit(HubEvent::BoardChanged {
            node_id: id.to_string(),
        });
        Ok(())
    }

    /// Read the node's token over SSH again and reconnect.
    pub fn pair_node(self: &Arc<Self>, id: &str) -> anyhow::Result<String> {
        let rec = self
            .remotes
            .read()
            .unwrap()
            .iter()
            .find(|r| r.rec.id == id)
            .map(|r| r.rec.clone())
            .ok_or_else(|| anyhow::anyhow!("unknown node {id}"))?;
        let Some(host) = rec.ssh_host.as_deref() else {
            anyhow::bail!("a node without an SSH host is paired with its token: add it again with the token, or scan a pairing code")
        };
        let tok = crate::tunnel::read_remote_token(host)?;
        keychain::store_token(id, &tok)?;
        // Reconnect with the new token.
        if let Some(rec) = self.stop_remote(id) {
            self.spawn_remote(rec);
        }
        Ok(format!("paired with {host}"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_start_with_the_address_in_use() {
        let rec = NodeRecord {
            id: "n1".into(),
            address: Some("a:1".into()),
            addresses: vec!["a:1".into(), "b:2".into()],
            ..Default::default()
        };
        let identity = NodeIdentity {
            ok: true,
            app: "kari".into(),
            version: "0".into(),
            api_version: API_VERSION,
            node_id: "x".into(),
            node_name: "x".into(),
            platform: "linux".into(),
            addresses: vec!["b:2".into(), "c:3".into()],
        };
        // The one in use first, then the known ones, then what the node says.
        // No address twice, so a probe never dials the same address again.
        assert_eq!(
            Hub::candidates(&rec, Some(&identity)),
            vec!["a:1".to_string(), "b:2".into(), "c:3".into()]
        );
    }

    fn col(id: &str, accepts: &[DerivedState]) -> Column {
        Column {
            id: id.into(),
            name: id.into(),
            order: 0,
            accepts: accepts.to_vec(),
            wip_limit: None,
            color: None,
            hidden: false,
        }
    }

    fn view(column_id: &str, state: DerivedState) -> CardView {
        CardView {
            card: Card {
                id: "c".into(),
                kind: CardKind::Task,
                title: None,
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
                last_job_state: None,
                last_job_at: None,
                created_at: Utc::now(),
                updated_at: Utc::now(),
                done_at: None,
            },
            title: "t".into(),
            state,
            column_id: column_id.into(),
            locked: false,
            project_name: None,
            session: None,
            live: None,
            bg_job: None,
            herdr: None,
            summary: None,
            hooks: None,
            estimate: None,
            last_activity_at: None,
            reason: String::new(),
            permission: None,
        }
    }

    #[test]
    fn remote_column_kept_when_local_has_it() {
        let cols = vec![
            col("a", &[DerivedState::Working]),
            col("b", &[DerivedState::Done]),
        ];
        assert_eq!(
            Hub::map_column(&cols, &view("b", DerivedState::Working)),
            "b"
        );
    }

    #[test]
    fn remote_column_mapped_by_state_when_unknown() {
        let cols = vec![
            col("a", &[DerivedState::Working]),
            col("b", &[DerivedState::Done]),
        ];
        assert_eq!(
            Hub::map_column(&cols, &view("zzz", DerivedState::Done)),
            "b"
        );
    }

    #[test]
    fn remote_column_falls_back_to_unknown_then_first() {
        let cols = vec![
            col("a", &[DerivedState::Working]),
            col("u", &[DerivedState::Unknown]),
        ];
        assert_eq!(
            Hub::map_column(&cols, &view("zzz", DerivedState::Stale)),
            "u"
        );
        let cols = vec![col("a", &[DerivedState::Working])];
        assert_eq!(
            Hub::map_column(&cols, &view("zzz", DerivedState::Stale)),
            "a"
        );
    }
}
