//! Blocking client for the node API in `api.rs`. The hub uses it for every
//! remote node.
//!
//! There are two ways to reach a node and the difference matters only here.
//! The hub in the desktop app connects *to* the node, over an SSH port forward
//! that ends on the node's loopback. A server is connected *by* the node, which
//! dials out and holds one socket (see `link`). Above this file the two are the
//! same node: the client keeps its typed methods and picks a `Transport`.

use crate::hooks::{HUB_HEADER, TOKEN_HEADER};
use crate::model::*;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader};
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone)]
pub struct ApiClient {
    transport: Transport,
    token: String,
    /// The hub id sent with column pushes. The node checks it against its lease.
    hub_id: Option<String>,
}

/// How a request reaches the node.
#[derive(Clone)]
enum Transport {
    /// An address this process can connect to.
    Http { base: String, http: Client },
    /// A socket the node opened to us. Nothing is dialled; the request is
    /// written into a link the server already holds.
    Link(Arc<dyn NodeLink>),
}

/// A node reached over the socket it dialled out on.
///
/// Declared here, beside the HTTP client, and implemented by the server on top
/// of `link`. The hub then reaches every node through one client whether it
/// found the node at an address or the node arrived by itself.
pub trait NodeLink: Send + Sync {
    /// One request into the node's API. The body comes back decoded; a status
    /// the node refused becomes an `Err` carrying its message, which is what
    /// the HTTP path does too.
    fn call(
        &self,
        method: &str,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<serde_json::Value>;

    /// This node's events, already filtered to it.
    fn events(&self) -> anyhow::Result<Box<dyn EventSource>>;

    /// What to show a person: an address, or the node's name.
    fn describe(&self) -> String;
}

/// A live stream of one node's events, however it arrives.
pub trait EventSource: Send {
    /// The next item, or None when the stream ended or failed.
    fn recv(&mut self) -> Option<EventItem>;
}

/// One server-sent event: its name and its data line.
#[derive(Debug, Clone, PartialEq)]
pub struct SseMessage {
    pub event: String,
    pub data: String,
}

/// A live `/events` stream. `recv` blocks until a message or a keepalive.
pub struct EventReader {
    lines: std::io::Lines<BufReader<Response>>,
}

#[derive(Debug)]
pub enum EventItem {
    Message(SseMessage),
    /// A comment line from the server's keepalive. The connection is alive.
    KeepAlive,
}

impl EventSource for EventReader {
    fn recv(&mut self) -> Option<EventItem> {
        EventReader::next_item(self)
    }
}

impl EventReader {
    /// The next item, or None when the stream ended or failed.
    fn next_item(&mut self) -> Option<EventItem> {
        let mut event = String::new();
        let mut data = String::new();
        loop {
            let line = self.lines.next()?.ok()?;
            if line.is_empty() {
                if event.is_empty() && data.is_empty() {
                    continue;
                }
                return Some(EventItem::Message(SseMessage { event, data }));
            }
            if let Some(rest) = line.strip_prefix(':') {
                let _ = rest;
                return Some(EventItem::KeepAlive);
            }
            if let Some(v) = line.strip_prefix("event:") {
                event = v.trim().to_string();
            } else if let Some(v) = line.strip_prefix("data:") {
                if !data.is_empty() {
                    data.push('\n');
                }
                data.push_str(v.trim_start());
            }
        }
    }
}

/// A 404 from a route that a newer kari added means the node is behind, not
/// that the card is missing. Say the thing the user can act on.
fn too_old(e: anyhow::Error) -> anyhow::Error {
    if e.to_string().contains("404") {
        anyhow::anyhow!("this node runs a kari that cannot hold attachments; update it")
    } else {
        e
    }
}

pub(crate) fn error_of(resp: Response) -> anyhow::Error {
    let status = resp.status();
    let path = resp.url().path().to_string();
    let text = resp.text().unwrap_or_default();
    let msg = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or(text);
    api_error(status, &path, msg.trim())
}

/// The prefix of every route this client calls.
const API_PREFIX: &str = "/kari/v1/";

/// Turn one failed reply into an error the user can act on.
///
/// A version skew reads as a bare 404 and tells the user nothing. Every handler
/// answers with a JSON `error` message (see the `ApiError` types in `api.rs` and
/// `server.rs`), and the router answers a route it does not know with an empty
/// body. So an empty 404 on an API path means one thing: the far end runs an
/// older kari than this client, and that route arrived after it. Name the skew,
/// and keep the status and the route for the next reader of a log.
fn api_error(status: reqwest::StatusCode, path: &str, msg: &str) -> anyhow::Error {
    if status == reqwest::StatusCode::NOT_FOUND && msg.is_empty() && path.starts_with(API_PREFIX) {
        return anyhow::anyhow!(
            "{status}: the kari at the other end is older than this one. \
             It does not know the route {path}. Update kari on that node."
        );
    }
    anyhow::anyhow!("{status}: {msg}")
}

impl ApiClient {
    /// A client for a node reachable at `127.0.0.1:port`.
    pub fn new(port: u16, token: &str) -> ApiClient {
        Self::at(&format!("http://127.0.0.1:{port}"), token)
    }

    /// A client for a node at a base URL, such as `http://host:47311`.
    pub fn at(base: &str, token: &str) -> ApiClient {
        ApiClient {
            transport: Transport::Http {
                base: base.trim_end_matches('/').to_string(),
                http: Client::builder()
                    .timeout(Duration::from_secs(20))
                    .build()
                    .expect("http client"),
            },
            token: token.to_string(),
            hub_id: None,
        }
    }

    /// A client for a node that dialled in. The token is already spent on the
    /// link itself, so nothing here carries one; the argument stays so that the
    /// two constructors read alike and `hub_id` still travels.
    pub fn over_link(link: Arc<dyn NodeLink>, hub_id: &str) -> ApiClient {
        ApiClient {
            transport: Transport::Link(link),
            token: String::new(),
            hub_id: Some(hub_id.to_string()),
        }
    }

    /// Send this hub id with every request. Column pushes need it.
    pub fn with_hub(mut self, hub_id: &str) -> ApiClient {
        self.hub_id = Some(hub_id.to_string());
        self
    }

    /// Where this client sends: an address, or the name of the node holding
    /// the socket. For a person to read, not to parse.
    pub fn base(&self) -> String {
        match &self.transport {
            Transport::Http { base, .. } => base.clone(),
            Transport::Link(l) => l.describe(),
        }
    }

    /// True when this node was reached over a link it opened. The hub asks
    /// because a linked node has no lease to arbitrate: a server is the only
    /// hub there is, so there is nobody to arbitrate with.
    pub fn is_linked(&self) -> bool {
        matches!(self.transport, Transport::Link(_))
    }

    fn headers(
        &self,
        mut req: reqwest::blocking::RequestBuilder,
    ) -> reqwest::blocking::RequestBuilder {
        req = req.header(TOKEN_HEADER, &self.token);
        if let Some(h) = &self.hub_id {
            req = req.header(HUB_HEADER, h);
        }
        req
    }

    fn json<T: DeserializeOwned>(&self, resp: Response) -> anyhow::Result<T> {
        if !resp.status().is_success() {
            return Err(error_of(resp));
        }
        Ok(resp.json()?)
    }

    fn get<T: DeserializeOwned>(&self, path: &str) -> anyhow::Result<T> {
        self.send(reqwest::Method::GET, path, None)
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        match &self.transport {
            Transport::Http { base, http } => {
                let mut req = self.headers(http.request(method, format!("{base}{path}")));
                if let Some(b) = body {
                    req = req.json(&b);
                }
                self.json(req.send()?)
            }
            Transport::Link(l) => {
                let v = l.call(method.as_str(), path, body)?;
                Ok(serde_json::from_value(v)?)
            }
        }
    }

    fn post<T: DeserializeOwned>(
        &self,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        self.send(reqwest::Method::POST, path, body)
    }

    // ---- node ----

    pub fn health(&self) -> anyhow::Result<NodeIdentity> {
        self.health_within(Duration::from_secs(5))
    }

    /// Health with a short timeout, for trying one candidate address after
    /// another. A blocked address must not hold the whole list.
    pub fn probe(&self, secs: u64) -> anyhow::Result<NodeIdentity> {
        self.health_within(Duration::from_secs(secs))
    }

    /// Health is the one route with its own timeout, because it is what the
    /// hub uses to decide whether an address answers at all. Over a link there
    /// is no address and no connect to time out: the socket is already there,
    /// and `call` carries the link's own deadline.
    fn health_within(&self, timeout: Duration) -> anyhow::Result<NodeIdentity> {
        match &self.transport {
            Transport::Http { base, http } => {
                let r = http
                    .get(format!("{base}/kari/health"))
                    .timeout(timeout)
                    .send()?;
                self.json(r)
            }
            Transport::Link(l) => Ok(serde_json::from_value(l.call(
                "GET",
                "/kari/health",
                None,
            )?)?),
        }
    }

    pub fn board(&self) -> anyhow::Result<BoardView> {
        self.get("/kari/v1/board")
    }

    pub fn refresh(&self) -> anyhow::Result<()> {
        let _: serde_json::Value = self.post("/kari/v1/refresh", None)?;
        Ok(())
    }

    /// Open the event stream. The blocking client has no read timeout, so the
    /// whole request ends after ten minutes and the hub opens a new one. A dead
    /// forward ends it sooner: ssh exits after three missed keepalives and the
    /// socket closes.
    pub fn events(&self) -> anyhow::Result<Box<dyn EventSource>> {
        let (base, _) = match &self.transport {
            Transport::Http { base, http } => (base.as_str(), http),
            // The node pushes its events down the link as they happen, so
            // there is no stream to open: the server is already receiving them.
            Transport::Link(l) => return l.events(),
        };
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
            .tcp_keepalive(Duration::from_secs(30))
            .build()?;
        let resp = http
            .get(format!("{base}/kari/v1/events"))
            .header(TOKEN_HEADER, &self.token)
            .header("accept", "text/event-stream")
            .send()?;
        if !resp.status().is_success() {
            return Err(error_of(resp));
        }
        Ok(Box::new(EventReader {
            lines: BufReader::new(resp).lines(),
        }))
    }

    // ---- cards ----

    pub fn add_task(&self, t: &NewTask) -> anyhow::Result<Card> {
        self.post("/kari/v1/cards", Some(serde_json::to_value(t)?))
    }

    pub fn patch_card(&self, id: &str, p: &CardPatch) -> anyhow::Result<Card> {
        self.send(
            reqwest::Method::PATCH,
            &format!("/kari/v1/cards/{id}"),
            Some(serde_json::to_value(p)?),
        )
    }

    pub fn delete_card(&self, id: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/kari/v1/cards/{id}"),
            None,
        )
    }

    // ---- attachments ----
    //
    // The routes arrived in version 0.12.0. A node that predates them answers
    // 404, and the message says to update that node rather than repeating the
    // raw status.

    pub fn add_attachment(&self, card: &str, a: &NewAttachment) -> anyhow::Result<Attachment> {
        self.post(
            &format!("/kari/v1/cards/{card}/attachments"),
            Some(serde_json::to_value(a)?),
        )
        .map_err(too_old)
    }

    pub fn attachment(&self, card: &str, name: &str) -> anyhow::Result<AttachmentData> {
        self.get(&format!("/kari/v1/cards/{card}/attachments/{name}"))
            .map_err(too_old)
    }

    pub fn delete_attachment(&self, card: &str, name: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/kari/v1/cards/{card}/attachments/{name}"),
            None,
        )
        .map_err(too_old)
    }

    /// Put a deleted card back. The route arrived after version 0.5.4, so a
    /// node that predates it answers 404. Say what to do about it.
    pub fn restore_card(&self, card: &Card) -> anyhow::Result<Card> {
        self.post("/kari/v1/cards/restore", Some(serde_json::to_value(card)?))
            .map_err(|e| {
                if e.to_string().contains("404") {
                    anyhow::anyhow!("this node runs a kari that cannot undo a delete; update it")
                } else {
                    e
                }
            })
    }

    /// Send a manual order for one column. The route arrived after version
    /// 0.4.1, so a node that predates it answers 404. Say what to do about it.
    pub fn reorder_cards(&self, ranked: &[String], unranked: &[String]) -> anyhow::Result<()> {
        self.post::<()>(
            "/kari/v1/cards/reorder",
            Some(serde_json::json!({ "ranked": ranked, "unranked": unranked })),
        )
        .map_err(|e| {
            if e.to_string().contains("404") {
                anyhow::anyhow!("this node runs a kari that cannot reorder cards; update it, or set the priority in the card drawer")
            } else {
                e
            }
        })
    }

    pub fn move_card(&self, id: &str, column_id: &str) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/cards/{id}/move"),
            Some(serde_json::json!({ "column_id": column_id })),
        )
    }

    pub fn start_card(&self, id: &str, prompt: Option<String>) -> anyhow::Result<String> {
        self.post(
            &format!("/kari/v1/cards/{id}/start"),
            Some(serde_json::json!({ "prompt": prompt })),
        )
    }

    pub fn stop_card(&self, id: &str) -> anyhow::Result<()> {
        self.post(&format!("/kari/v1/cards/{id}/stop"), None)
    }

    /// The route arrived with version 0.10.0. An older node answers 404, and
    /// the message says what to do about it.
    pub fn send_prompt(&self, id: &str, text: &str) -> anyhow::Result<String> {
        self.post(
            &format!("/kari/v1/cards/{id}/send"),
            Some(serde_json::json!({ "text": text })),
        )
        .map_err(|e| {
            if e.to_string().contains("404") {
                anyhow::anyhow!("this node runs a kari that cannot take a prompt; update it")
            } else {
                e
            }
        })
    }

    pub fn conversation(&self, id: &str, limit: usize) -> anyhow::Result<Conversation> {
        self.get(&format!("/kari/v1/cards/{id}/conversation?limit={limit}"))
    }

    /// Book a run of a card on that node. The route arrived after version
    /// 0.9.0, so an older node answers 404. Say what to do about it.
    pub fn schedule_card(&self, id: &str, req: &ScheduleRequest) -> anyhow::Result<Card> {
        self.post(
            &format!("/kari/v1/cards/{id}/schedule"),
            Some(serde_json::to_value(req)?),
        )
        .map_err(|e| {
            if e.to_string().contains("404") {
                anyhow::anyhow!("this node runs a kari that cannot schedule a run; update it")
            } else {
                e
            }
        })
    }

    pub fn cancel_schedule(&self, id: &str) -> anyhow::Result<Card> {
        self.send(
            reqwest::Method::DELETE,
            &format!("/kari/v1/cards/{id}/schedule"),
            None,
        )
    }

    pub fn summarize_card(&self, id: &str) -> anyhow::Result<Summary> {
        self.post(&format!("/kari/v1/cards/{id}/summarize"), None)
    }

    pub fn jump(&self, id: &str) -> anyhow::Result<JumpPlan> {
        self.post(&format!("/kari/v1/cards/{id}/jump"), None)
    }

    pub fn job_log(&self, id: &str, limit: usize) -> anyhow::Result<Vec<JobLogEntry>> {
        self.get(&format!("/kari/v1/cards/{id}/jobs?limit={limit}"))
    }

    // ---- columns, settings ----

    pub fn columns(&self) -> anyhow::Result<Vec<Column>> {
        self.get("/kari/v1/columns")
    }

    pub fn set_columns(&self, cols: &[Column]) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::PUT,
            "/kari/v1/columns",
            Some(serde_json::to_value(cols)?),
        )
    }

    pub fn settings(&self) -> anyhow::Result<Settings> {
        self.get("/kari/v1/settings")
    }

    // ---- held permissions ----

    pub fn permissions(&self) -> anyhow::Result<Vec<PendingPermission>> {
        self.get("/kari/v1/permissions")
    }

    pub fn answer_permission(&self, id: &str, behavior: &str) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/permissions/{id}"),
            Some(serde_json::json!({ "behavior": behavior })),
        )
    }

    /// Flip Away mode on the node. Reads the settings, changes one field, writes them back.
    pub fn set_away_mode(&self, on: bool) -> anyhow::Result<()> {
        let mut s = self.settings()?;
        if s.away_mode == on {
            return Ok(());
        }
        s.away_mode = on;
        self.set_settings(&s)
    }

    /// Write the automation mode through the settings, so a node that predates
    /// the mode field still follows the two booleans it does understand.
    pub fn set_automation_mode(&self, mode: AutomationMode) -> anyhow::Result<()> {
        let mut s = self.settings()?;
        s.set_automation(mode);
        self.set_settings(&s)
    }

    // ---- lease ----

    /// The node's column lease. `Ok(None)` when free. An older node without
    /// the route answers 404, which reads as "no lease" too.
    pub fn lease(&self) -> anyhow::Result<Option<Lease>> {
        match &self.transport {
            Transport::Http { base, http } => {
                let r = self
                    .headers(http.get(format!("{base}/kari/v1/lease")))
                    .send()?;
                if r.status() == reqwest::StatusCode::NOT_FOUND {
                    return Ok(None);
                }
                self.json(r)
            }
            // A server is the only hub, so it takes no lease and asks for
            // none. Answering "free" keeps the caller's shape without
            // pretending a 404 came back.
            Transport::Link(_) => Ok(None),
        }
    }

    pub fn claim_lease(&self, claim: &LeaseClaim) -> anyhow::Result<Lease> {
        self.post("/kari/v1/lease", Some(serde_json::to_value(claim)?))
    }

    pub fn release_lease(&self, hub_id: &str) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::DELETE,
            "/kari/v1/lease",
            Some(serde_json::json!({ "hub_id": hub_id })),
        )
    }

    pub fn set_settings(&self, s: &Settings) -> anyhow::Result<()> {
        self.send(
            reqwest::Method::PUT,
            "/kari/v1/settings",
            Some(serde_json::to_value(s)?),
        )
    }

    // ---- proposals ----

    pub fn proposal(&self) -> anyhow::Result<Option<Proposal>> {
        self.get("/kari/v1/proposal")
    }

    pub fn propose_now(&self) -> anyhow::Result<Proposal> {
        self.post("/kari/v1/proposal", None)
    }

    pub fn proposal_history(&self, limit: usize) -> anyhow::Result<Vec<Proposal>> {
        self.get(&format!("/kari/v1/proposals?limit={limit}"))
    }

    pub fn accept_proposal(
        &self,
        id: &str,
        card_ids: Option<Vec<String>>,
    ) -> anyhow::Result<usize> {
        self.post(
            &format!("/kari/v1/proposals/{id}/accept"),
            Some(serde_json::json!({ "card_ids": card_ids })),
        )
    }

    pub fn snooze_proposal(&self, id: &str, minutes: i64) -> anyhow::Result<()> {
        self.post(
            &format!("/kari/v1/proposals/{id}/snooze"),
            Some(serde_json::json!({ "minutes": minutes })),
        )
    }

    pub fn dismiss_proposal(&self, id: &str) -> anyhow::Result<()> {
        self.post(&format!("/kari/v1/proposals/{id}/dismiss"), None)
    }

    pub fn stop_proposal(&self, id: &str) -> anyhow::Result<usize> {
        self.post(&format!("/kari/v1/proposals/{id}/stop"), None)
    }

    // ---- the rest ----

    pub fn quota_history(&self, limit: usize) -> anyhow::Result<Vec<QuotaSample>> {
        self.get(&format!("/kari/v1/quota?limit={limit}"))
    }

    pub fn projects(&self) -> anyhow::Result<Vec<Project>> {
        self.get("/kari/v1/projects")
    }

    pub fn stop_all(&self) -> anyhow::Result<usize> {
        self.post("/kari/v1/stop-all", None)
    }
}

#[cfg(test)]
mod tests {
    use super::api_error;
    use reqwest::StatusCode;

    /// The route the send button calls. An older server does not have it.
    const SEND: &str = "/kari/v1/hub/nodes/n1/cards/c1/send";

    #[test]
    fn an_empty_404_on_an_api_route_names_the_skew() {
        let e = api_error(StatusCode::NOT_FOUND, SEND, "").to_string();
        assert!(e.contains("older than this one"), "{e}");
        assert!(e.contains("Update kari on that node"), "{e}");
        // The status and the route stay in the message, for the log.
        assert!(e.contains("404"), "{e}");
        assert!(e.contains(SEND), "{e}");
    }

    #[test]
    fn a_404_that_carries_a_message_keeps_it() {
        // A handler answered. It is not a skew, so do not guess at one.
        let e = api_error(StatusCode::NOT_FOUND, SEND, "no node n1 is linked").to_string();
        assert_eq!(e, "404 Not Found: no node n1 is linked");
    }

    #[test]
    fn another_status_keeps_its_message() {
        let e = api_error(StatusCode::BAD_GATEWAY, SEND, "the node does not answer").to_string();
        assert_eq!(e, "502 Bad Gateway: the node does not answer");
    }

    #[test]
    fn an_empty_404_off_the_api_keeps_its_shape() {
        // A proxy or a wrong base URL, not a kari route. Claim nothing.
        let e = api_error(StatusCode::NOT_FOUND, "/", "").to_string();
        assert_eq!(e, "404 Not Found: ");
    }
}
