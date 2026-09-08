//! `RemoteHub`: a hub that is somewhere else.
//!
//! It implements the same `HubApi` the in-process `Hub` does, by calling the
//! server's `/kari/v1/hub/` routes. The Tauri layer and the React UI hold the
//! trait, so nothing above this file knows which one it has, and the
//! single-machine setup keeps running the code it always ran.
//!
//! Two things are deliberately *not* forwarded:
//!
//! - **The roster.** Adding, pairing and removing nodes is how a hub reaches a
//!   node at an address. A server's nodes dial *it*; there is nothing here to
//!   add. Those methods refuse with a sentence saying where to do it instead.
//! - **This device's own store.** Settings, the account aliases, the
//!   calibration and the hooks belong to the machine the app runs on, so they
//!   never travel. `local_engine` hands back the store this client opened,
//!   exactly as a phone's hub does.

use crate::client::error_of;
use crate::hooks::TOKEN_HEADER;
use crate::hub::HubEvent;
use crate::hubapi::HubApi;
use crate::model::*;
use crate::server::*;
use crate::Engine;
use reqwest::blocking::Client;
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use tracing::{info, warn};

pub struct RemoteHub {
    base: String,
    token: String,
    http: Client,
    /// This device's store. Not the board — the settings, aliases and
    /// calibration that belong to the machine rather than to the hub.
    engine: Arc<Engine>,
    /// The server's events, republished locally so a subscriber cannot tell.
    tx: broadcast::Sender<HubEvent>,
}

impl RemoteHub {
    /// Open a client for the server at `base`, and start following its events.
    ///
    /// Nothing is dialled here: the constructor cannot fail on a server that is
    /// down, because the app must still start and show its last board when the
    /// network is not there. The event follower reconnects on its own.
    pub fn connect(base: &str, token: &str, engine: Arc<Engine>) -> Arc<RemoteHub> {
        // rustls will not pick a provider on its own, and the panic lands on
        // whichever thread happened to dial. See link::ensure_crypto_provider.
        crate::link::ensure_crypto_provider();
        let (tx, _) = broadcast::channel(256);
        let hub = Arc::new(RemoteHub {
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            http: Client::builder()
                .timeout(Duration::from_secs(30))
                .build()
                .expect("http client"),
            engine,
            tx,
        });
        hub.follow_events();
        hub
    }

    /// Where this hub is, for a person to read.
    pub fn base(&self) -> &str {
        &self.base
    }

    /// Ask the server what it is. A client uses this to refuse a server whose
    /// API it does not speak, rather than failing one call at a time later.
    pub fn health(&self) -> anyhow::Result<ServerIdentity> {
        self.get("/kari/health")
    }

    /// The server's board, and the error when there is not one.
    ///
    /// `HubApi::board` has no error channel and answers with an empty board, so
    /// a caller that must tell "the server has no nodes" from "the server did
    /// not answer" needs this instead. `SplitHub` is that caller: it keeps its
    /// own columns when the server is away, and the empty board cannot say
    /// whether it is.
    pub fn try_board(&self) -> anyhow::Result<HubBoard> {
        self.get("/kari/v1/hub/board")
    }

    fn url(&self, path: &str) -> String {
        format!("{}{path}", self.base)
    }

    fn json<T: DeserializeOwned>(&self, r: reqwest::blocking::Response) -> anyhow::Result<T> {
        if !r.status().is_success() {
            return Err(error_of(r));
        }
        // A 204 has no body, and the caller of a `()` method still expects Ok.
        let text = r.text()?;
        if text.trim().is_empty() {
            return Ok(serde_json::from_str("null")?);
        }
        Ok(serde_json::from_str(&text)?)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        let r = self
            .http
            .get(self.url(path))
            .header(TOKEN_HEADER, &self.token)
            .send()?;
        self.json(r)
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        let mut req = self
            .http
            .request(method, self.url(path))
            .header(TOKEN_HEADER, &self.token);
        if let Some(b) = body {
            req = req.json(&b);
        }
        self.json(req.send()?)
    }

    fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        self.send(reqwest::Method::POST, path, body)
    }

    /// A list the server could not give us is empty, not fatal.
    ///
    /// These are the read-only methods whose signature has nowhere to put an
    /// error — the same ones `Hub` answers from a cache when a node is away.
    /// A warning goes to the log so a person can see the difference between
    /// "nothing there" and "could not ask".
    fn or_empty<T: Default>(&self, what: &str, r: anyhow::Result<T>) -> T {
        match r {
            Ok(v) => v,
            Err(e) => {
                warn!("{what} from {}: {e}", self.base);
                T::default()
            }
        }
    }

    /// Follow the server's event stream forever, republishing into `tx`.
    ///
    /// A dropped stream is normal — a laptop sleeps, a network moves — so this
    /// reconnects with the same backoff a node uses to reach a server, and
    /// announces a reconnect as a board change, because whatever happened while
    /// the stream was down was missed.
    fn follow_events(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("kari-remote-events".into())
            .spawn(move || {
                let mut backoff = Duration::from_secs(1);
                loop {
                    match me.stream_once() {
                        Ok(()) => backoff = Duration::from_secs(1),
                        Err(e) => warn!("event stream from {}: {e}", me.base),
                    }
                    if Arc::strong_count(&me) == 1 {
                        return; // Nobody holds the hub any more.
                    }
                    std::thread::sleep(backoff);
                    backoff = (backoff * 2).min(Duration::from_secs(60));
                    // The board moved while we were not listening.
                    let _ = me.tx.send(HubEvent::BoardChanged {
                        node_id: String::new(),
                    });
                }
            })
            .expect("spawn");
    }

    fn stream_once(&self) -> anyhow::Result<()> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(None)
            .tcp_keepalive(Duration::from_secs(30))
            .build()?;
        let resp = http
            .get(self.url("/kari/v1/hub/events"))
            .header(TOKEN_HEADER, &self.token)
            .header("accept", "text/event-stream")
            .send()?;
        if !resp.status().is_success() {
            return Err(error_of(resp));
        }
        info!("following the hub at {}", self.base);
        for line in BufReader::new(resp).lines() {
            let line = line?;
            let Some(data) = line.strip_prefix("data:") else {
                continue; // The event name and the keepalives.
            };
            match serde_json::from_str::<HubEvent>(data.trim()) {
                Ok(e) => {
                    let _ = self.tx.send(e);
                }
                Err(e) => warn!("undecodable hub event: {e}"),
            }
        }
        Ok(())
    }

    /// The one answer for every roster method. A server's nodes arrive by
    /// dialling in, so there is nothing for a client to add or to pair.
    fn no_roster<T>(&self) -> anyhow::Result<T> {
        anyhow::bail!(
            "this board comes from the server at {}. Nodes join it by linking \
             to it, so add or remove one where the node runs, not here",
            self.base
        )
    }
}

fn v<T: serde::Serialize>(t: T) -> Option<serde_json::Value> {
    serde_json::to_value(t).ok()
}

impl HubApi for RemoteHub {
    fn subscribe(&self) -> broadcast::Receiver<HubEvent> {
        self.tx.subscribe()
    }

    fn board(&self) -> HubBoard {
        self.or_empty("board", self.try_board())
    }

    fn nodes(&self) -> Vec<NodeStatus> {
        self.or_empty("nodes", self.get("/kari/v1/hub/nodes"))
    }

    fn refresh_all(&self) {
        let r: anyhow::Result<serde_json::Value> = self.post("/kari/v1/hub/refresh", None);
        self.or_empty("refresh", r);
    }

    fn columns(&self) -> Vec<Column> {
        self.or_empty("columns", self.get("/kari/v1/hub/columns"))
    }

    fn set_columns(&self, cols: Vec<Column>) -> anyhow::Result<()> {
        self.send(reqwest::Method::PUT, "/kari/v1/hub/columns", v(cols))
    }

    fn reset_columns(&self) -> anyhow::Result<()> {
        self.post("/kari/v1/hub/columns/reset", None)
    }

    fn add_task(&self, node: &str, t: NewTask) -> anyhow::Result<Card> {
        self.post(&format!("/kari/v1/hub/nodes/{node}/cards"), v(t))
    }

    fn patch_card(&self, node: &str, card: &str, p: CardPatch) -> anyhow::Result<Card> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}"),
            v(p),
        )
    }

    fn move_card(&self, node: &str, card: &str, column: &str) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}/move"),
            v(ColumnBody {
                column_id: column.to_string(),
            }),
        )
    }

    fn delete_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}"),
            None,
        )
    }

    fn restore_card(&self, node: &str, card: Card) -> anyhow::Result<Card> {
        self.post(&format!("/kari/v1/hub/nodes/{node}/cards/restore"), v(card))
    }

    fn reorder_cards(
        &self,
        node: &str,
        ranked: Vec<String>,
        unranked: Vec<String>,
    ) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/hub/nodes/{node}/cards/reorder"),
            v(ReorderBody { ranked, unranked }),
        )
    }

    fn move_card_to_node(&self, from: &str, card: &str, to: &str) -> anyhow::Result<Card> {
        self.post(
            &format!("/kari/v1/hub/nodes/{from}/cards/{card}/move-to-node"),
            v(ToNodeBody {
                to_node_id: to.to_string(),
            }),
        )
    }

    fn start_card(&self, node: &str, card: &str, prompt: Option<String>) -> anyhow::Result<String> {
        let b: IdBody = self.post(
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}/start"),
            v(PromptBody { prompt }),
        )?;
        Ok(b.id)
    }

    fn stop_card(&self, node: &str, card: &str) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}/stop"),
            None,
        )
    }

    fn stop_all(&self) -> anyhow::Result<usize> {
        let b: CountBody = self.post("/kari/v1/hub/stop-all", None)?;
        Ok(b.count)
    }

    fn summarize_card(&self, node: &str, card: &str) -> anyhow::Result<Summary> {
        self.post(
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}/summarize"),
            None,
        )
    }

    fn job_log(&self, node: &str, card: &str, limit: usize) -> Vec<JobLogEntry> {
        self.or_empty(
            "job log",
            self.get(&format!(
                "/kari/v1/hub/nodes/{node}/cards/{card}/log?limit={limit}"
            )),
        )
    }

    fn jump_in(&self, node: &str, card: &str) -> anyhow::Result<String> {
        // The server answers a jump with the command to run, not with a
        // terminal: it has no desktop, and a node that dialled in cannot be
        // dialled back. `Hub::jump_in` says the client runs it where the person
        // actually is — and when the card is on *this* machine, that is here.
        //
        // Without this, pointing an app at a server takes Jump in away from the
        // machine it works best on: before pairing the local engine opened a
        // terminal, and after pairing the same click only printed a line for
        // the user to retype. A node and an app on one host share a store, so
        // the card id from the server resolves locally.
        //
        // A failure falls through to the server rather than surfacing: the card
        // may have come from a node this device only knows through the server,
        // and the command in the answer is still useful.
        if self.engine.node_id() == node {
            if let Ok(msg) = self.engine.jump_in(card) {
                return Ok(msg);
            }
        }
        let b: IdBody = self.post(
            &format!("/kari/v1/hub/nodes/{node}/cards/{card}/jump"),
            None,
        )?;
        Ok(b.id)
    }

    fn answer_permission(&self, node: &str, id: &str, behavior: &str) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/hub/nodes/{node}/permissions/{id}"),
            v(BehaviorBody {
                behavior: behavior.to_string(),
            }),
        )
    }

    fn set_automation_mode(&self, node: &str, mode: AutomationMode) -> anyhow::Result<()> {
        self.post(&format!("/kari/v1/hub/nodes/{node}/automation"), v(mode))
    }

    fn set_automation_mode_all(&self, mode: AutomationMode) -> Vec<String> {
        self.or_empty("automation", self.post("/kari/v1/hub/automation", v(mode)))
    }

    fn set_away_mode(&self, node: &str, on: bool) -> anyhow::Result<()> {
        self.post(&format!("/kari/v1/hub/nodes/{node}/away"), v(OnBody { on }))
    }

    fn propose_now(&self, node: &str) -> anyhow::Result<Proposal> {
        self.post(&format!("/kari/v1/hub/nodes/{node}/proposal"), None)
    }

    fn proposal(&self, node: &str) -> Option<Proposal> {
        self.or_empty(
            "proposal",
            self.get(&format!("/kari/v1/hub/nodes/{node}/proposal")),
        )
    }

    fn proposal_history(&self, node: &str, limit: usize) -> Vec<Proposal> {
        self.or_empty(
            "proposals",
            self.get(&format!(
                "/kari/v1/hub/nodes/{node}/proposals?limit={limit}"
            )),
        )
    }

    fn accept_proposal(
        &self,
        node: &str,
        id: &str,
        card_ids: Option<Vec<String>>,
    ) -> anyhow::Result<usize> {
        let b: CountBody = self.post(
            &format!("/kari/v1/hub/nodes/{node}/proposals/{id}/accept"),
            v(AcceptBody { card_ids }),
        )?;
        Ok(b.count)
    }

    fn snooze_proposal(&self, node: &str, id: &str, minutes: i64) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/hub/nodes/{node}/proposals/{id}/snooze"),
            v(MinutesBody { minutes }),
        )
    }

    fn dismiss_proposal(&self, node: &str, id: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/kari/v1/hub/nodes/{node}/proposals/{id}"),
            None,
        )
    }

    fn stop_proposal(&self, node: &str, id: &str) -> anyhow::Result<usize> {
        let b: CountBody = self.post(
            &format!("/kari/v1/hub/nodes/{node}/proposals/{id}/stop"),
            None,
        )?;
        Ok(b.count)
    }

    fn projects(&self, node: &str) -> Vec<Project> {
        self.or_empty(
            "projects",
            self.get(&format!("/kari/v1/hub/nodes/{node}/projects")),
        )
    }

    fn quota_history(&self, node: &str, limit: usize) -> Vec<QuotaSample> {
        self.or_empty(
            "quota",
            self.get(&format!("/kari/v1/hub/nodes/{node}/quota?limit={limit}")),
        )
    }

    fn add_node(&self, _n: NewNode) -> anyhow::Result<NodeStatus> {
        self.no_roster()
    }
    fn update_node(&self, _id: &str, _p: NodePatch) -> anyhow::Result<NodeStatus> {
        self.no_roster()
    }
    fn remove_node(&self, _id: &str) -> anyhow::Result<()> {
        self.no_roster()
    }
    fn pair_node(&self, _id: &str) -> anyhow::Result<String> {
        self.no_roster()
    }
    fn pairing_code(&self) -> anyhow::Result<String> {
        self.no_roster()
    }

    /// There is one hub and it is the server's. Nothing here can be primary,
    /// and nothing needs to claim it — which is the point of having a server.
    fn is_primary(&self) -> bool {
        true
    }

    fn claim_primary(&self) -> anyhow::Result<String> {
        anyhow::bail!(
            "the hub is the server at {}, so no device needs to be primary",
            self.base
        )
    }

    fn is_remote(&self) -> bool {
        true
    }

    fn local_engine(&self) -> &Arc<Engine> {
        &self.engine
    }
}

/// Where this device's hub comes from.
///
/// A file next to the store, not a setting in it: the app must know before it
/// opens a hub, and a setting that lives *in* the hub cannot say which hub to
/// open. Absent means the ordinary single-machine arrangement, which is why
/// nothing has to be configured to keep working.
pub fn server_config() -> Option<(String, String)> {
    let base = std::fs::read_to_string(crate::paths::kari_dir().join("server-url")).ok()?;
    let base = base.trim().to_string();
    if base.is_empty() {
        return None;
    }
    let token = std::env::var("KARI_SERVER_TOKEN")
        .ok()
        .or_else(|| std::fs::read_to_string(crate::paths::server_token_file()).ok())
        .unwrap_or_default()
        .trim()
        .to_string();
    Some((base, token))
}

/// Point this device at a server, after checking that the server is really
/// there and really accepts the token.
///
/// Checked rather than merely written, because the alternative is a device that
/// looks configured, shows an empty board, and gives the person no way to tell
/// a typo from an unreachable network.
pub fn set_server(base: &str, token: &str) -> anyhow::Result<ServerIdentity> {
    let base = base.trim().trim_end_matches('/');
    if base.is_empty() {
        anyhow::bail!("give the server's address, such as https://kari.example.com");
    }
    if !base.starts_with("http://") && !base.starts_with("https://") {
        anyhow::bail!("the address must start with http:// or https://");
    }
    let token = token.trim();
    if token.is_empty() {
        anyhow::bail!("give the token the server expects: `kari-server token` prints it");
    }

    crate::link::ensure_crypto_provider();
    let probe = RemoteHub {
        base: base.to_string(),
        token: token.to_string(),
        http: Client::builder().timeout(Duration::from_secs(15)).build()?,
        engine: Engine::open()?,
        tx: broadcast::channel(1).0,
    };
    let id = probe.health()?;
    if id.app != "kari-server" {
        anyhow::bail!("{base} answered, but it is not a kari server");
    }
    // The token is only proved by a route that checks it. Health is open, so a
    // wrong token would otherwise be found much later, on the first real call.
    let _: Vec<crate::model::NodeStatus> = probe.get("/kari/v1/hub/nodes")?;

    let dir = crate::paths::kari_dir();
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("server-url"), base)?;
    write_token(&crate::paths::server_token_file(), token)?;
    Ok(id)
}

#[cfg(unix)]
fn write_token(path: &std::path::Path, token: &str) -> anyhow::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    // 0600 from the moment it exists, rather than written and then chmod'd.
    let mut f = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)?;
    f.write_all(token.as_bytes())?;
    Ok(())
}

#[cfg(not(unix))]
fn write_token(path: &std::path::Path, token: &str) -> anyhow::Result<()> {
    std::fs::write(path, token)?;
    Ok(())
}

/// Stop using a server. The board goes back to whatever this device can reach
/// by itself, which on a phone is nothing until it is pointed at another one.
pub fn clear_server() -> anyhow::Result<()> {
    let dir = crate::paths::kari_dir();
    for f in [dir.join("server-url"), crate::paths::server_token_file()] {
        match std::fs::remove_file(&f) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// The hub this device should use: the server named in its configuration, or
/// the in-process one.
///
/// A server that is configured but unreachable still gives a `RemoteHub`. The
/// app must start and say the server is not answering; silently falling back to
/// a local hub would put the board back to one-hub-per-client, which is the
/// arrangement the server exists to end, and would do it without telling
/// anyone.
pub fn open_hub(engine: Arc<Engine>) -> Arc<dyn HubApi> {
    open_hub_inner(engine, true)
}

/// The same, for a device that runs no Claude Code and is therefore not a node
/// on its own board: a phone. Its engine is only the hub's store.
pub fn open_hub_without_local(engine: Arc<Engine>) -> Arc<dyn HubApi> {
    open_hub_inner(engine, false)
}

fn open_hub_inner(engine: Arc<Engine>, with_local: bool) -> Arc<dyn HubApi> {
    match server_config() {
        // A machine that runs Claude Code keeps its own engine on the board and
        // adds the server's nodes to it. What a server takes over is the other
        // hosts and the columns, never the sessions in front of the user.
        Some((base, token)) if with_local => crate::split::SplitHub::connect(engine, &base, &token),
        // A device that runs no Claude Code has nothing of its own to show, so
        // the server is the whole board: a phone.
        Some((base, token)) => {
            info!("the hub is the server at {base}");
            RemoteHub::connect(&base, &token, engine)
        }
        None if with_local => crate::hub::Hub::new(engine),
        None => crate::hub::Hub::without_local(engine),
    }
}
