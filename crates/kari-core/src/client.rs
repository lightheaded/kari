//! Blocking client for the node API in `api.rs`. The hub uses it for every
//! remote node, through an SSH port forward that ends on the node's loopback.

use crate::hooks::{HUB_HEADER, TOKEN_HEADER};
use crate::model::*;
use reqwest::blocking::{Client, Response};
use serde::de::DeserializeOwned;
use std::io::{BufRead, BufReader};
use std::time::Duration;

#[derive(Clone)]
pub struct ApiClient {
    base: String,
    token: String,
    /// The hub id sent with column pushes. The node checks it against its lease.
    hub_id: Option<String>,
    http: Client,
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

pub enum EventItem {
    Message(SseMessage),
    /// A comment line from the server's keepalive. The connection is alive.
    KeepAlive,
}

impl EventReader {
    /// The next item, or None when the stream ended or failed.
    pub fn recv(&mut self) -> Option<EventItem> {
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

fn error_of(resp: Response) -> anyhow::Error {
    let status = resp.status();
    let text = resp.text().unwrap_or_default();
    let msg = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("error")
                .and_then(|e| e.as_str())
                .map(|s| s.to_string())
        })
        .unwrap_or(text);
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
            base: base.trim_end_matches('/').to_string(),
            token: token.to_string(),
            hub_id: None,
            http: Client::builder()
                .timeout(Duration::from_secs(20))
                .build()
                .expect("http client"),
        }
    }

    /// Send this hub id with every request. Column pushes need it.
    pub fn with_hub(mut self, hub_id: &str) -> ApiClient {
        self.hub_id = Some(hub_id.to_string());
        self
    }

    pub fn base(&self) -> &str {
        &self.base
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
        let r = self
            .headers(self.http.get(format!("{}{path}", self.base)))
            .send()?;
        self.json(r)
    }

    fn send<T: DeserializeOwned>(
        &self,
        method: reqwest::Method,
        path: &str,
        body: Option<serde_json::Value>,
    ) -> anyhow::Result<T> {
        let mut req = self.headers(self.http.request(method, format!("{}{path}", self.base)));
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

    // ---- node ----

    pub fn health(&self) -> anyhow::Result<NodeIdentity> {
        let r = self
            .http
            .get(format!("{}/kari/health", self.base))
            .timeout(Duration::from_secs(5))
            .send()?;
        self.json(r)
    }

    /// Health with a short timeout, for trying one candidate address after
    /// another. A blocked address must not hold the whole list.
    pub fn probe(&self, secs: u64) -> anyhow::Result<NodeIdentity> {
        let r = self
            .http
            .get(format!("{}/kari/health", self.base))
            .timeout(Duration::from_secs(secs))
            .send()?;
        self.json(r)
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
    pub fn events(&self) -> anyhow::Result<EventReader> {
        let http = Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(600))
            .tcp_keepalive(Duration::from_secs(30))
            .build()?;
        let resp = http
            .get(format!("{}/kari/v1/events", self.base))
            .header(TOKEN_HEADER, &self.token)
            .header("accept", "text/event-stream")
            .send()?;
        if !resp.status().is_success() {
            return Err(error_of(resp));
        }
        Ok(EventReader {
            lines: BufReader::new(resp).lines(),
        })
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
        let r = self
            .headers(self.http.get(format!("{}/kari/v1/lease", self.base)))
            .send()?;
        if r.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        self.json(r)
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

    pub fn projects(&self) -> anyhow::Result<Vec<(String, String)>> {
        self.get("/kari/v1/projects")
    }

    pub fn stop_all(&self) -> anyhow::Result<usize> {
        self.post("/kari/v1/stop-all", None)
    }
}
