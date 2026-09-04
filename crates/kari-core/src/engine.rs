//! The engine owns the snapshot of every signal, the store, and the watchers.

use crate::model::*;
use crate::{
    agents, estimate, herdr, hooks, infer, launcher, paths, planner, quota, registry, store::Store,
    summary, transcript,
};
use chrono::{DateTime, Duration, Utc};
use notify::RecursiveMode;
use notify_debouncer_mini::new_debouncer;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, RwLock};
use tokio::sync::{broadcast, oneshot};
use tracing::{info, warn};

/// Seconds between two looks at the planner triggers. The poller runs every
/// 15 seconds and calls `proposal_tick` on every fourth round.
const PROPOSAL_TICK_SECS: i64 = 60;

/// The first eight characters of an id, for log lines. Never slices bytes.
fn short(s: &str) -> String {
    s.chars().take(8).collect()
}

#[derive(Debug, Clone)]
pub enum Event {
    BoardChanged,
    /// The column lease of this node changed hands.
    LeaseChanged,
    Notice {
        title: String,
        body: String,
        card_id: Option<String>,
    },
}

#[derive(Default)]
struct Snapshot {
    facts: HashMap<String, SessionFacts>,
    live: HashMap<String, LiveSession>,
    jobs: Vec<BgJob>,
    herdr: Vec<HerdrAgent>,
    herdr_ok: bool,
    quota: Option<QuotaSample>,
    prev_state: HashMap<String, DerivedState>,
    dirty_facts: HashSet<String>,
    hooks: HashMap<String, HookState>,
    summaries: HashMap<String, Summary>,
    /// Last state kari saw per background job id, to notice a change.
    job_states: HashMap<String, String>,
    hooks_installed: bool,
    /// Sessions whose turn ended since the last summary check.
    summary_wanted: HashSet<String>,
    calibration: Calibration,
    calibrated_at: Option<DateTime<Utc>>,
    /// The open proposal, or the last accepted one while its jobs run.
    proposal: Option<Proposal>,
    /// Permission prompts held for a remote answer, by id. The sender wakes
    /// the hook handler that waits on it.
    permissions: HashMap<String, (PendingPermission, Option<oneshot::Sender<String>>)>,
    /// When the planner last looked at the triggers. The queue shows the next
    /// check, which is one poller round after this.
    proposal_checked_at: Option<DateTime<Utc>>,
}

pub struct Engine {
    store: Mutex<Store>,
    snap: RwLock<Snapshot>,
    settings: RwLock<Settings>,
    tx: broadcast::Sender<Event>,
    scanning: AtomicBool,
}

impl Engine {
    /// Open the store at a chosen directory instead of the default kari
    /// directory. For a hub on a device without a home directory, such as a
    /// phone. Call it once, before any other path lookup.
    pub fn open_at(dir: &std::path::Path) -> anyhow::Result<Arc<Engine>> {
        paths::set_kari_dir(dir);
        Self::open()
    }

    pub fn open() -> anyhow::Result<Arc<Engine>> {
        let store = Store::open(&paths::kari_db())?;
        let settings = store.load_settings()?;
        let merged = store
            .kv_get("notice.columns_merged")
            .ok()
            .flatten()
            .and_then(|v| v.parse::<usize>().ok());
        let facts = store.load_facts()?;
        let summaries = store.load_summaries().unwrap_or_default();
        let (tx, _) = broadcast::channel(64);
        let mut snap = Snapshot {
            facts,
            summaries,
            hooks_installed: hooks::installed(),
            ..Default::default()
        };
        // Restore the plan panel only while the plan is still current. A plan
        // that timed out must not come back on the next start.
        let now = Utc::now();
        snap.proposal = store
            .latest_proposal(&["open", "accepted"])
            .unwrap_or_default()
            .filter(|p| p.is_live(now));
        // Seed the job states kari already knows, so a restart logs nothing twice.
        for c in store.list_cards().unwrap_or_default() {
            if let (Some(job), Some(state)) = (c.bg_job_id, c.last_job_state) {
                snap.job_states.insert(job, state);
            }
        }
        let engine = Arc::new(Engine {
            store: Mutex::new(store),
            snap: RwLock::new(snap),
            settings: RwLock::new(settings),
            tx,
            scanning: AtomicBool::new(false),
        });
        if let Some(moved) = merged {
            let _ = engine
                .store
                .lock()
                .unwrap()
                .kv_delete("notice.columns_merged");
            let _ = engine.tx.send(Event::Notice {
                title: "The board now has six columns".into(),
                body: format!(
                    "Needs me holds the three states that wait for you. Review holds Validate and Waiting on others. {moved} manual placement(s) moved with them. Columns, then Reset to defaults, restores the six at any time."
                ),
                card_id: None,
            });
        }
        Ok(engine)
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.tx.subscribe()
    }

    pub fn settings(&self) -> Settings {
        self.settings.read().unwrap().clone()
    }

    pub fn set_settings(&self, s: Settings) -> anyhow::Result<()> {
        self.store.lock().unwrap().save_settings(&s)?;
        *self.settings.write().unwrap() = s;
        self.emit_changed();
        Ok(())
    }

    fn emit_changed(&self) {
        let _ = self.tx.send(Event::BoardChanged);
    }

    // ---------------------------------------------------------------- scans

    /// Walk the projects directory and fold appended transcript bytes into facts.
    pub fn scan_transcripts(&self) -> anyhow::Result<usize> {
        let settings = self.settings();
        let cutoff = Utc::now() - Duration::days(settings.history_days);
        let root = paths::claude_projects_dir();
        let Ok(rd) = std::fs::read_dir(&root) else {
            return Ok(0);
        };
        let mut changed = 0usize;
        let mut new_cards: Vec<Card> = vec![];
        let mut dirty: Vec<String> = vec![];
        let mut deltas: Vec<TokenDelta> = vec![];

        for proj in rd.flatten() {
            let ppath = proj.path();
            if !ppath.is_dir() {
                continue;
            }
            let Ok(files) = std::fs::read_dir(&ppath) else {
                continue;
            };
            for f in files.flatten() {
                let fpath = f.path();
                if fpath.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                    continue;
                }
                let Ok(meta) = f.metadata() else { continue };
                let mtime: DateTime<Utc> =
                    meta.modified().map(DateTime::<Utc>::from).unwrap_or(cutoff);
                let session_id = fpath
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .to_string();
                if session_id.len() < 8 {
                    continue;
                }
                let mut snap = self.snap.write().unwrap();
                let known_len = snap
                    .facts
                    .get(&session_id)
                    .map(|x| x.bytes_parsed)
                    .unwrap_or(0);
                if mtime < cutoff && known_len == 0 {
                    continue; // old and never parsed: outside the history window
                }
                if known_len == meta.len() {
                    continue;
                }
                let facts = snap
                    .facts
                    .entry(session_id.clone())
                    .or_insert_with(|| SessionFacts {
                        session_id: session_id.clone(),
                        transcript_path: fpath.to_string_lossy().into_owned(),
                        ..Default::default()
                    });
                facts.transcript_path = fpath.to_string_lossy().into_owned();
                let before = facts.tokens.weighted();
                match transcript::update(&fpath, facts) {
                    Ok(true) => {
                        changed += 1;
                        // Token growth feeds the percent-per-token calibration.
                        // A session parsed for the first time has no `before`, so it is skipped.
                        let after = facts.tokens.weighted();
                        let grew = after - before;
                        let at = facts.last_at.unwrap_or_else(Utc::now);
                        if before > 0.0 && grew > 0.0 && grew < 60_000_000.0 {
                            deltas.push(TokenDelta {
                                at,
                                session_id: session_id.clone(),
                                weighted: grew,
                            });
                        }
                        dirty.push(session_id.clone());
                        snap.dirty_facts.insert(session_id.clone());
                    }
                    Ok(false) => {}
                    Err(e) => warn!("transcript {}: {e}", fpath.display()),
                }
                let cwd = snap.facts.get(&session_id).and_then(|f| f.cwd.clone());
                drop(snap);
                if let Some(card) = self.ensure_session_card(&session_id, cwd.as_deref())? {
                    new_cards.push(card);
                }
            }
        }

        if !dirty.is_empty() {
            let snap = self.snap.read().unwrap();
            let batch: Vec<&SessionFacts> =
                dirty.iter().filter_map(|id| snap.facts.get(id)).collect();
            self.store.lock().unwrap().save_facts_batch(&batch)?;
        }
        if !deltas.is_empty() {
            let _ = self.store.lock().unwrap().insert_token_deltas(&deltas);
        }
        if changed > 0 {
            info!(
                "transcripts changed: {changed}, new cards: {}",
                new_cards.len()
            );
        }
        Ok(changed)
    }

    fn ensure_session_card(
        &self,
        session_id: &str,
        cwd: Option<&str>,
    ) -> anyhow::Result<Option<Card>> {
        let store = self.store.lock().unwrap();
        Self::ensure_session_card_in(&store, session_id, cwd)
    }

    /// The caller holds the store lock.
    fn ensure_session_card_in(
        store: &Store,
        session_id: &str,
        cwd: Option<&str>,
    ) -> anyhow::Result<Option<Card>> {
        // The summarizer and other internal runs are kari's own work, not board items.
        if cwd.is_some_and(paths::is_internal_cwd) {
            return Ok(None);
        }
        if let Some(mut existing) = store.card_by_session(session_id)? {
            if existing.project_cwd.is_none() && cwd.is_some() {
                existing.project_cwd = cwd.map(|s| s.to_string());
                store.upsert_card(&existing)?;
            }
            return Ok(None);
        }
        let now = Utc::now();
        let card = Card {
            id: uuid::Uuid::new_v4().to_string(),
            kind: CardKind::Session,
            title: None,
            session_id: Some(session_id.to_string()),
            project_cwd: cwd.map(|s| s.to_string()),
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
            created_at: now,
            updated_at: now,
            done_at: None,
        };
        store.upsert_card(&card)?;
        Ok(Some(card))
    }

    pub fn scan_live(&self) {
        let live = registry::read_all();
        let mut snap = self.snap.write().unwrap();
        snap.live = live;
        drop(snap);
        // A live session may have no transcript yet (first prompt pending). Make its card anyway.
        let live: Vec<LiveSession> = self
            .snap
            .read()
            .unwrap()
            .live
            .values()
            .filter(|l| l.alive)
            .cloned()
            .collect();
        for l in live {
            let _ = self.ensure_session_card(&l.session_id, Some(&l.cwd));
        }
    }

    pub fn scan_jobs(&self) {
        match agents::list() {
            Ok(jobs) => {
                let mut snap = self.snap.write().unwrap();
                snap.jobs = jobs;
            }
            Err(e) => warn!("claude agents: {e}"),
        }
        let jobs: Vec<BgJob> = self.snap.read().unwrap().jobs.clone();
        if let Err(e) = self.link_jobs(&jobs) {
            warn!("link jobs: {e}");
        }
        self.track_job_states();
    }

    /// Give every background job one card.
    /// A job that kari started belongs to the card that holds its id, or that ran it
    /// earlier by the run log. A card with no session adopts the job's session. A
    /// session card made for the same session before the link existed merges into
    /// the owner, so the board shows the task once. An older run of a card that moved
    /// on to a new job stays in the run log only.
    /// A job that kari did not start gets a session card of its own.
    fn link_jobs(&self, jobs: &[BgJob]) -> anyhow::Result<()> {
        let store = self.store.lock().unwrap();
        let cards = store.list_cards()?;
        let mut removed: HashSet<String> = HashSet::new();
        for j in jobs {
            let (Some(sid), Some(jid)) = (j.session_id.as_deref(), j.id.as_deref()) else {
                continue;
            };
            let by_id = cards
                .iter()
                .find(|c| !removed.contains(&c.id) && c.bg_job_id.as_deref() == Some(jid));
            let owner = match by_id {
                Some(c) => Some(c),
                None => store.card_id_for_job(jid)?.and_then(|id| {
                    cards
                        .iter()
                        .find(|c| c.id == id && !removed.contains(&c.id))
                }),
            };
            let Some(owner) = owner else {
                Self::ensure_session_card_in(&store, sid, j.cwd.as_deref())?;
                continue;
            };
            let mut owner = owner.clone();
            let mut changed = false;
            if owner.session_id.is_none() {
                owner.session_id = Some(sid.to_string());
                changed = true;
                info!("card {} adopts session {}", short(&owner.id), short(sid));
            }
            let twins: Vec<&Card> = cards
                .iter()
                .filter(|c| {
                    c.id != owner.id
                        && c.kind == CardKind::Session
                        && c.session_id.as_deref() == Some(sid)
                        && !removed.contains(&c.id)
                })
                .collect();
            for twin in twins {
                if owner.notes.is_none() && twin.notes.is_some() {
                    owner.notes = twin.notes.clone();
                    changed = true;
                }
                for t in &twin.tags {
                    if !owner.tags.contains(t) {
                        owner.tags.push(t.clone());
                        changed = true;
                    }
                }
                store.delete_card(&twin.id)?;
                removed.insert(twin.id.clone());
                info!(
                    "session card {} merged into card {}",
                    short(&twin.id),
                    short(&owner.id)
                );
            }
            if changed {
                owner.updated_at = Utc::now();
                store.upsert_card(&owner)?;
            }
        }
        Ok(())
    }

    /// Write one run-log line per job state change and remember the state on the card.
    /// `claude agents` forgets a job after a while, so the card must hold the outcome.
    fn track_job_states(&self) {
        let jobs: Vec<BgJob> = self.snap.read().unwrap().jobs.clone();
        let cards = self.store.lock().unwrap().list_cards().unwrap_or_default();
        let now = Utc::now();
        let mut logs: Vec<JobLogEntry> = vec![];
        let mut touched: Vec<Card> = vec![];
        {
            let mut snap = self.snap.write().unwrap();
            for j in &jobs {
                let (Some(id), Some(state)) = (j.id.as_deref(), j.state.as_deref()) else {
                    continue;
                };
                if snap.job_states.get(id).map(|s| s.as_str()) == Some(state) {
                    continue;
                }
                snap.job_states.insert(id.to_string(), state.to_string());
                // Only jobs kari started belong in a run log.
                let Some(card) = cards.iter().find(|c| c.bg_job_id.as_deref() == Some(id)) else {
                    continue;
                };
                let card = Some(card);
                logs.push(JobLogEntry {
                    at: now,
                    job_id: id.to_string(),
                    card_id: card.map(|c| c.id.clone()),
                    state: Some(state.to_string()),
                    detail: j.waiting_for.clone().or_else(|| j.status.clone()),
                });
                if let Some(c) = card {
                    let mut c = c.clone();
                    c.last_job_state = Some(state.to_string());
                    c.last_job_at = Some(now);
                    c.updated_at = now;
                    touched.push(c);
                }
            }
        }
        if logs.is_empty() {
            return;
        }
        let store = self.store.lock().unwrap();
        for l in &logs {
            let _ = store.log_job(l);
        }
        for c in &touched {
            let _ = store.upsert_card(c);
        }
        info!("job states: {} change(s)", logs.len());
    }

    pub fn job_log(&self, card_id: &str, limit: usize) -> Vec<JobLogEntry> {
        self.store
            .lock()
            .unwrap()
            .job_log(card_id, limit)
            .unwrap_or_default()
    }

    pub fn scan_herdr(&self) {
        let agents = if herdr::available() {
            herdr::agents().ok()
        } else {
            None
        };
        let mut snap = self.snap.write().unwrap();
        snap.herdr_ok = agents.is_some();
        snap.herdr = agents.unwrap_or_default();
    }

    pub fn scan_quota(self: &Arc<Self>) {
        if let Some(s) = quota::read_latest() {
            let _ = self.store.lock().unwrap().insert_quota_sample(&s);
            let mut snap = self.snap.write().unwrap();
            if snap.quota.as_ref().is_none_or(|q| s.at >= q.at) {
                snap.quota = Some(s);
            }
        }
        let installed = hooks::installed();
        self.snap.write().unwrap().hooks_installed = installed;

        let settings = self.settings();
        let now = Utc::now();
        // No session refreshed the status line for a while. Ask the usage endpoint.
        if settings.usage_endpoint_enabled {
            let stale = self
                .snap
                .read()
                .unwrap()
                .quota
                .as_ref()
                .is_none_or(|q| (now - q.at).num_seconds() > quota::STALE_AFTER_SECS);
            if stale {
                let me = Arc::clone(self);
                std::thread::spawn(move || match quota::fetch_usage() {
                    Ok(s) => {
                        let _ = me.store.lock().unwrap().insert_quota_sample(&s);
                        let mut snap = me.snap.write().unwrap();
                        if snap.quota.as_ref().is_none_or(|q| s.at >= q.at) {
                            snap.quota = Some(s);
                        }
                        drop(snap);
                        me.emit_changed();
                    }
                    Err(e) => tracing::debug!("usage endpoint: {e}"),
                });
            }
        }

        let due = {
            let snap = self.snap.read().unwrap();
            snap.calibrated_at
                .is_none_or(|t| now - t > Duration::minutes(10))
        };
        if due {
            self.refresh_calibration();
        }
    }

    /// Learn percent per million weighted tokens from the last three days.
    pub fn refresh_calibration(&self) {
        let (samples, deltas) = {
            let st = self.store.lock().unwrap();
            (
                st.quota_samples_since(3 * 86400).unwrap_or_default(),
                st.token_deltas_since(3 * 86400).unwrap_or_default(),
            )
        };
        let c = estimate::calibrate(&samples, &deltas);
        info!(
            "calibration: {:.3} pct/Mtok ({}, {} pairs)",
            c.pct_per_mtok, c.source, c.samples
        );
        let mut snap = self.snap.write().unwrap();
        snap.calibration = c;
        snap.calibrated_at = Some(Utc::now());
    }

    pub fn calibration(&self) -> Calibration {
        self.snap.read().unwrap().calibration.clone()
    }

    // ---------------------------------------------------------------- hooks

    /// Fold one Claude Code hook payload into the snapshot. Fast: no disk scan here.
    pub fn ingest_hook(self: &Arc<Self>, payload: serde_json::Value) -> anyhow::Result<()> {
        let Some(ev) = hooks::parse(&payload) else {
            anyhow::bail!("payload has no session_id or hook_event_name")
        };
        if !hooks::valid_session_id(&ev.session_id) {
            anyhow::bail!("payload has a malformed session_id");
        }
        if ev
            .cwd
            .as_deref()
            .is_some_and(|c| !Path::new(c).is_absolute())
        {
            anyhow::bail!("payload cwd is not an absolute path");
        }
        // kari's own summary runs live under the kari directory. Ignore them.
        if ev
            .cwd
            .as_deref()
            .is_some_and(|c| Path::new(c).starts_with(paths::kari_dir()))
        {
            return Ok(());
        }
        {
            let mut snap = self.snap.write().unwrap();
            let st = snap.hooks.entry(ev.session_id.clone()).or_default();
            hooks::apply(st, &ev);
            if ev.event == "Stop" {
                snap.summary_wanted.insert(ev.session_id.clone());
            }
            // The session moved on, so a held prompt of it is stale. The
            // handler that waits on it answers with no decision.
            if matches!(
                ev.event.as_str(),
                "Stop" | "SessionEnd" | "PostToolUse" | "UserPromptSubmit"
            ) {
                let stale: Vec<String> = snap
                    .permissions
                    .iter()
                    .filter(|(_, (p, _))| p.session_id == ev.session_id)
                    .map(|(id, _)| id.clone())
                    .collect();
                for id in stale {
                    if let Some((_, Some(tx))) = snap.permissions.remove(&id) {
                        let _ = tx.send(String::new());
                    }
                }
            }
        }
        let _ = self.store.lock().unwrap().log_hook(&ev);
        let _ = self.ensure_session_card(&ev.session_id, ev.cwd.as_deref());
        info!(
            "hook {} {} {}",
            ev.event,
            ev.notification_type
                .clone()
                .or(ev.tool_name.clone())
                .unwrap_or_default(),
            short(&ev.session_id)
        );
        // The registry and transcript lag behind the hook by a moment. Rescan shortly.
        let me = Arc::clone(self);
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(400));
            me.refresh_light();
        });
        self.detect_transitions();
        self.emit_changed();
        Ok(())
    }

    // ---------------------------------------------------------------- held permissions

    /// Hold a permission prompt for a remote answer. Returns the receiver the
    /// hook handler waits on, or None when Away mode is off or the payload is
    /// not a permission request. The notice names the tool and the card.
    pub fn hold_permission(
        self: &Arc<Self>,
        payload: &serde_json::Value,
    ) -> Option<oneshot::Receiver<String>> {
        let s = |k: &str| {
            payload
                .get(k)
                .and_then(|v| v.as_str())
                .map(|v| v.to_string())
        };
        if s("hook_event_name").as_deref() != Some(hooks::HELD_EVENT) {
            return None;
        }
        let settings = self.settings();
        if !settings.away_mode {
            return None;
        }
        let session_id = s("session_id").filter(|x| hooks::valid_session_id(x))?;
        let tool_name = s("tool_name").unwrap_or_else(|| "tool".into());
        let now = Utc::now();
        let hold = settings
            .away_hold_secs
            .clamp(5, hooks::HELD_TIMEOUT_SECS - 30) as i64;
        let pending = PendingPermission {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.clone(),
            tool_name: tool_name.clone(),
            tool_input: payload
                .get("tool_input")
                .cloned()
                .unwrap_or(serde_json::Value::Null),
            message: s("message"),
            since: now,
            until: now + Duration::seconds(hold),
        };
        let (tx, rx) = oneshot::channel();
        let card_id = self
            .store
            .lock()
            .unwrap()
            .card_by_session(&session_id)
            .ok()
            .flatten()
            .map(|c| c.id);
        let title = self
            .snap
            .read()
            .unwrap()
            .facts
            .get(&session_id)
            .and_then(|f| f.title())
            .unwrap_or_else(|| short(&session_id));
        let summary = summarize_input(&tool_name, &pending.tool_input);
        self.snap
            .write()
            .unwrap()
            .permissions
            .insert(pending.id.clone(), (pending, Some(tx)));
        info!(
            "holding a {tool_name} permission for {} up to {hold} s",
            short(&session_id)
        );
        let _ = self.tx.send(Event::Notice {
            title: format!("Allow {tool_name}? · {title}"),
            body: summary,
            card_id,
        });
        self.emit_changed();
        Some(rx)
    }

    /// Answer a held prompt. The hook handler prints the decision and Claude
    /// Code continues without a dialog.
    pub fn answer_permission(&self, id: &str, behavior: &str) -> anyhow::Result<()> {
        if !matches!(behavior, "allow" | "deny") {
            anyhow::bail!("behavior must be allow or deny");
        }
        let (pending, tx) = {
            let mut snap = self.snap.write().unwrap();
            let Some(entry) = snap.permissions.remove(id) else {
                anyhow::bail!("this prompt is no longer held; the terminal has it now")
            };
            if let Some(st) = snap.hooks.get_mut(&entry.0.session_id) {
                st.permission_pending_since = None;
                st.permission_message = None;
            }
            entry
        };
        let Some(tx) = tx else {
            anyhow::bail!("this prompt is no longer held; the terminal has it now")
        };
        if tx.send(behavior.to_string()).is_err() {
            anyhow::bail!("the hook gave up on this prompt already");
        }
        info!(
            "{behavior}: {} for {}",
            pending.tool_name,
            short(&pending.session_id)
        );
        self.emit_changed();
        Ok(())
    }

    /// The hold ran out. Forget the prompt; the terminal shows the dialog now.
    pub fn drop_permission(&self, id: &str) {
        let removed = self.snap.write().unwrap().permissions.remove(id).is_some();
        if removed {
            self.emit_changed();
        }
    }

    pub fn pending_permissions(&self) -> Vec<PendingPermission> {
        let mut v: Vec<PendingPermission> = self
            .snap
            .read()
            .unwrap()
            .permissions
            .values()
            .map(|(p, _)| p.clone())
            .collect();
        v.sort_by_key(|p| p.since);
        v
    }

    pub fn install_hooks(&self) -> anyhow::Result<String> {
        let port = self.settings().hooks_port;
        let cmd = hooks::install(port)?;
        self.snap.write().unwrap().hooks_installed = true;
        self.emit_changed();
        Ok(format!("hooks installed, relay is {cmd}"))
    }

    pub fn uninstall_hooks(&self) -> anyhow::Result<()> {
        hooks::uninstall()?;
        self.snap.write().unwrap().hooks_installed = false;
        self.emit_changed();
        Ok(())
    }

    // ---------------------------------------------------------------- summaries

    /// Summarize one card now, outside the throttle.
    pub fn summarize_card(&self, card_id: &str) -> anyhow::Result<Summary> {
        let sid = {
            let store = self.store.lock().unwrap();
            let Some(c) = store.get_card(card_id)? else {
                anyhow::bail!("card not found")
            };
            c.session_id
                .ok_or_else(|| anyhow::anyhow!("task cards have no transcript to summarize"))?
        };
        self.summarize_session(&sid)
    }

    fn summarize_session(&self, session_id: &str) -> anyhow::Result<Summary> {
        let settings = self.settings();
        let facts = self
            .snap
            .read()
            .unwrap()
            .facts
            .get(session_id)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("no transcript facts yet"))?;
        let t0 = std::time::Instant::now();
        let s = summary::generate(&facts, &settings.summary_model)?;
        info!(
            "summary for {} took {:?} ({:?}, {:.2})",
            short(session_id),
            t0.elapsed(),
            s.judged_state,
            s.confidence
        );
        self.store.lock().unwrap().save_summary(&s)?;
        {
            let mut snap = self.snap.write().unwrap();
            snap.summaries.insert(session_id.to_string(), s.clone());
            snap.summary_wanted.remove(session_id);
        }
        self.detect_transitions();
        self.emit_changed();
        Ok(s)
    }

    /// Pick at most one session that deserves a fresh summary and summarize it.
    /// Throttle: `summaries_per_hour` globally, one per session per 10 minutes,
    /// only sessions active within `summary_recent_hours`, only after the turn settled.
    pub fn summary_tick(&self) {
        let settings = self.settings();
        if !settings.summaries_enabled {
            return;
        }
        let used = self
            .store
            .lock()
            .unwrap()
            .summaries_in_last_hour()
            .unwrap_or(u32::MAX);
        if used >= settings.summaries_per_hour {
            return;
        }
        let now = Utc::now();
        let candidate = {
            let snap = self.snap.read().unwrap();
            let recent = now - Duration::hours(settings.summary_recent_hours);
            let mut best: Option<(i32, DateTime<Utc>, String)> = None;
            for f in snap.facts.values() {
                let Some(last) = f.last_at else { continue };
                if f.turns == 0 || last < recent {
                    continue;
                }
                // Let the turn settle. A busy session changes every second.
                if now - last < Duration::seconds(20) {
                    continue;
                }
                let alive = snap.live.get(&f.session_id).is_some_and(|l| l.alive);
                if alive
                    && snap
                        .live
                        .get(&f.session_id)
                        .and_then(|l| l.status.clone())
                        .as_deref()
                        == Some("busy")
                {
                    continue;
                }
                if !f.turn_closed
                    && !f
                        .pending_tools
                        .iter()
                        .any(|t| t.name == "AskUserQuestion" || t.name == "ExitPlanMode")
                    && alive
                {
                    continue;
                }
                if let Some(s) = snap.summaries.get(&f.session_id) {
                    if s.based_on_at.is_some_and(|b| b >= last) {
                        continue; // nothing new since the last summary
                    }
                    if now - s.generated_at < Duration::minutes(10) {
                        continue;
                    }
                }
                let mut score = 0;
                if alive {
                    score += 100;
                }
                if snap.summary_wanted.contains(&f.session_id) {
                    score += 50;
                }
                if !snap.summaries.contains_key(&f.session_id) {
                    score += 10;
                }
                let better = match &best {
                    None => true,
                    Some((bs, bl, _)) => score > *bs || (score == *bs && last > *bl),
                };
                if better {
                    best = Some((score, last, f.session_id.clone()));
                }
            }
            best.map(|b| b.2)
        };
        let Some(sid) = candidate else { return };
        if let Err(e) = self.summarize_session(&sid) {
            warn!("summary {}: {e}", short(&sid));
            // Do not retry this exact state every tick.
            self.snap.write().unwrap().summary_wanted.remove(&sid);
        }
    }

    pub fn refresh_all(self: &Arc<Self>) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        let t0 = std::time::Instant::now();
        self.scan_live();
        if let Err(e) = self.scan_transcripts() {
            warn!("scan_transcripts: {e}");
        }
        self.scan_jobs();
        self.scan_herdr();
        self.scan_quota();
        self.scanning.store(false, Ordering::SeqCst);
        info!("refresh_all took {:?}", t0.elapsed());
        self.detect_transitions();
        self.emit_changed();
    }

    /// Cheap refresh for file events: no `claude agents` call.
    pub fn refresh_light(self: &Arc<Self>) {
        if self.scanning.swap(true, Ordering::SeqCst) {
            return;
        }
        self.scan_live();
        if let Err(e) = self.scan_transcripts() {
            warn!("scan_transcripts: {e}");
        }
        self.scan_herdr();
        self.scan_quota();
        self.scanning.store(false, Ordering::SeqCst);
        self.detect_transitions();
        self.emit_changed();
    }

    fn detect_transitions(&self) {
        let board = self.board();
        let mut snap = self.snap.write().unwrap();
        for cv in &board.cards {
            let prev = snap.prev_state.insert(cv.card.id.clone(), cv.state);
            if prev.is_none() || prev == Some(cv.state) {
                continue;
            }
            let notice = match cv.state {
                DerivedState::NeedsDecision => Some(("Decision needed", cv.reason.clone())),
                DerivedState::NeedsApproval => Some(("Approval needed", cv.reason.clone())),
                DerivedState::Validate if cv.bg_job.is_some() => {
                    Some(("Background job finished", cv.reason.clone()))
                }
                DerivedState::MyTurn if cv.reason.contains("failed") => {
                    Some(("Background job failed", cv.reason.clone()))
                }
                _ => None,
            };
            if let Some((title, body)) = notice {
                let _ = self.tx.send(Event::Notice {
                    title: format!("{title}: {}", cv.title),
                    body,
                    card_id: Some(cv.card.id.clone()),
                });
            }
        }
    }

    // ---------------------------------------------------------------- board

    fn match_herdr<'a>(
        agents: &'a [HerdrAgent],
        live: Option<&LiveSession>,
        facts: Option<&SessionFacts>,
        live_in_cwd: usize,
    ) -> Option<&'a HerdrAgent> {
        let sid = live
            .map(|l| l.session_id.as_str())
            .or(facts.map(|f| f.session_id.as_str()))?;
        if let Some(a) = agents.iter().find(|a| a.session_id.as_deref() == Some(sid)) {
            return Some(a);
        }
        let cwd = live
            .map(|l| l.cwd.as_str())
            .or(facts.and_then(|f| f.cwd.as_deref()))?;
        let in_cwd: Vec<&HerdrAgent> = agents
            .iter()
            .filter(|a| a.cwd.as_deref() == Some(cwd))
            .collect();
        if in_cwd.is_empty() {
            return None;
        }
        let title = facts.and_then(|f| f.title()).map(|t| t.to_lowercase());
        if let Some(t) = &title {
            if let Some(a) = in_cwd
                .iter()
                .find(|a| a.title.as_deref().map(|x| x.to_lowercase()) == Some(t.clone()))
            {
                return Some(a);
            }
        }
        if in_cwd.len() == 1 && live_in_cwd == 1 {
            return Some(in_cwd[0]);
        }
        None
    }

    pub fn board(&self) -> BoardView {
        let settings = self.settings();
        let now = Utc::now();
        let (cards, columns) = {
            let store = self.store.lock().unwrap();
            (
                store.list_cards().unwrap_or_default(),
                store.load_columns().unwrap_or_default(),
            )
        };
        let snap = self.snap.read().unwrap();
        let pools = estimate::pools(&snap.facts);
        let calibration = snap.calibration.clone();
        let mut live_per_cwd: HashMap<String, usize> = HashMap::new();
        for l in snap.live.values().filter(|l| l.alive) {
            *live_per_cwd.entry(l.cwd.clone()).or_default() += 1;
        }
        let mut out = vec![];
        let mut lock_breaks: Vec<Card> = vec![];
        for card in cards {
            if card.archived {
                continue;
            }
            let facts = card.session_id.as_ref().and_then(|s| snap.facts.get(s));
            let live = card
                .session_id
                .as_ref()
                .and_then(|s| snap.live.get(s))
                .filter(|l| l.alive);
            let bg = snap.jobs.iter().find(|j| {
                (card.bg_job_id.is_some() && j.id == card.bg_job_id)
                    || (card.session_id.is_some() && j.session_id == card.session_id)
            });
            let cwd_owned = card
                .project_cwd
                .clone()
                .or_else(|| facts.and_then(|f| f.cwd.clone()));
            let live_in_cwd = cwd_owned
                .as_ref()
                .and_then(|c| live_per_cwd.get(c))
                .copied()
                .unwrap_or(0);
            let herdr_agent = Self::match_herdr(&snap.herdr, live, facts, live_in_cwd);
            let permission = card.session_id.as_deref().and_then(|sid| {
                snap.permissions
                    .values()
                    .map(|(p, _)| p)
                    .filter(|p| p.session_id == sid)
                    .min_by_key(|p| p.since)
                    .cloned()
            });
            // A session that never got a prompt and has no process is noise,
            // unless kari holds a permission prompt for it.
            if card.kind == CardKind::Session
                && live.is_none()
                && bg.is_none()
                && permission.is_none()
                && facts.is_none_or(|f| f.turns == 0)
            {
                continue;
            }
            let hook_state = card.session_id.as_ref().and_then(|s| snap.hooks.get(s));
            let summ = card.session_id.as_ref().and_then(|s| snap.summaries.get(s));
            let est = estimate::estimate_with(&pools, &card, facts, &calibration);
            let inputs = infer::Inputs {
                card: &card,
                facts,
                live,
                bg,
                herdr: herdr_agent,
                hooks: hook_state,
                permission: permission.as_ref(),
                summary: summ,
                now,
                settings: &settings,
            };
            let (state, reason) = infer::derive(&inputs);
            let (column_id, locked, broke) = infer::resolve_column(&card, state, &columns);
            let Some(column_id) = column_id else { continue };
            if broke {
                let mut c = card.clone();
                c.manual_column = None;
                c.manual_lock_priority = None;
                lock_breaks.push(c);
            }
            let title = card
                .title
                .clone()
                .or_else(|| facts.and_then(|f| f.title()))
                .or_else(|| live.and_then(|l| l.name.clone()))
                .or_else(|| bg.and_then(|b| b.name.clone()))
                .unwrap_or_else(|| {
                    card.session_id
                        .as_deref()
                        .map(|s| s.chars().take(8).collect())
                        .unwrap_or_else(|| "untitled".into())
                });
            out.push(CardView {
                permission,
                title,
                state,
                column_id,
                locked,
                project_name: cwd_owned.as_deref().map(paths::project_display_name),
                last_activity_at: infer::last_activity(facts, live, bg),
                session: facts.cloned(),
                live: live.cloned(),
                bg_job: bg.cloned(),
                herdr: herdr_agent.cloned(),
                summary: summ.cloned(),
                hooks: hook_state.cloned(),
                estimate: Some(est),
                reason,
                card,
            });
        }
        let quota = snap.quota.clone();
        let herdr_connected = snap.herdr_ok;
        let hooks_installed = snap.hooks_installed;
        let proposal = snap.proposal.clone();
        let checked_at = snap.proposal_checked_at;
        drop(snap);
        if !lock_breaks.is_empty() {
            let store = self.store.lock().unwrap();
            for c in lock_breaks {
                let _ = store.upsert_card(&c);
            }
        }
        let mut view = BoardView {
            columns,
            cards: out,
            quota,
            generated_at: now,
            scanning: self.scanning.load(Ordering::SeqCst),
            herdr_connected,
            hooks_installed,
            hooks_port: settings.hooks_port,
            calibration,
            proposal,
            away_mode: settings.away_mode,
            queue: None,
            automation_mode: settings.automation().key().into(),
        };
        // The queue reads the finished view, so it comes last.
        view.queue = Some(self.queue_of(&view, &settings, now, checked_at));
        view
    }

    /// The dry run of the planner for the queue strip. It starts nothing and
    /// stores nothing: every call answers from the board it is given.
    fn queue_of(
        &self,
        view: &BoardView,
        settings: &Settings,
        now: DateTime<Utc>,
        checked_at: Option<DateTime<Utc>>,
    ) -> QueuePlan {
        let ctx = Self::planner_context(view, now);
        let open = view.proposal.as_ref().is_some_and(|p| p.state == "open");
        planner::queue(
            Self::candidates(view),
            &ctx,
            settings,
            settings.automation(),
            open,
            checked_at
                .map(|t| t + Duration::seconds(PROPOSAL_TICK_SECS))
                .unwrap_or(now + Duration::seconds(PROPOSAL_TICK_SECS)),
        )
    }

    /// Turn Away mode on or off: hold permission prompts for a remote answer.
    pub fn set_away_mode(&self, on: bool) -> anyhow::Result<()> {
        let mut s = self.settings();
        if s.away_mode == on {
            return Ok(());
        }
        s.away_mode = on;
        self.set_settings(s)
    }

    // ---------------------------------------------------------------- mutations

    pub fn move_card(&self, card_id: &str, column_id: &str) -> anyhow::Result<()> {
        let board = self.board();
        let Some(cv) = board.cards.iter().find(|c| c.card.id == card_id) else {
            anyhow::bail!("card not found")
        };
        let Some(col) = board.columns.iter().find(|c| c.id == column_id) else {
            anyhow::bail!("column not found")
        };
        let mut card = cv.card.clone();
        let now = Utc::now();
        let natural = infer::column_for(cv.state, &board.columns);
        if natural.as_deref() == Some(column_id) {
            card.manual_column = None;
            card.manual_lock_priority = None;
        } else {
            card.manual_column = Some(column_id.to_string());
            card.manual_lock_priority = Some(cv.state.priority());
        }
        // Dropping on a Done column marks done; leaving it clears the mark.
        if col.accepts.contains(&DerivedState::Done) {
            card.done_at = Some(now);
        } else if card.done_at.is_some() {
            card.done_at = None;
        }
        // A manual move settles the outcome of the last job.
        if col.accepts.contains(&DerivedState::Done) || col.accepts.contains(&DerivedState::Backlog)
        {
            card.last_job_state = None;
            card.last_job_at = None;
        }
        // Dropping a task on Ready enables auto-run when a prompt exists.
        // A task on Ready may run unattended. Its title alone is prompt enough,
        // so nothing is copied into the body.
        if col.accepts.contains(&DerivedState::Ready) && card.kind == CardKind::Task {
            card.auto_run = true;
            card.manual_column = None;
            card.manual_lock_priority = None;
        }
        if col.accepts.contains(&DerivedState::Backlog) && card.kind == CardKind::Task {
            card.auto_run = false;
            card.manual_column = None;
            card.manual_lock_priority = None;
        }
        card.updated_at = now;
        self.store.lock().unwrap().upsert_card(&card)?;
        self.emit_changed();
        Ok(())
    }

    pub fn add_task(&self, t: NewTask) -> anyhow::Result<Card> {
        let now = Utc::now();
        let columns = self.columns();
        let target = t
            .column_id
            .as_deref()
            .and_then(|id| columns.iter().find(|c| c.id == id));
        // A task added at the foot of a column must appear there. Backlog and
        // Ready need no lock, because a task derives one of those two states on
        // its own. Every other column gets a manual lock instead.
        let auto_run =
            t.auto_run || target.is_some_and(|c| c.accepts.contains(&DerivedState::Ready));
        let lock = target.filter(|c| {
            !c.accepts.contains(&DerivedState::Ready) && !c.accepts.contains(&DerivedState::Backlog)
        });
        let card = Card {
            id: uuid::Uuid::new_v4().to_string(),
            kind: CardKind::Task,
            title: Some(t.title),
            session_id: None,
            project_cwd: t.project_cwd,
            priority: t.priority,
            auto_run,
            run_prompt: t.run_prompt,
            permission_mode: None,
            model: t.model.filter(|m| !m.trim().is_empty()),
            estimate_weighted_tokens: None,
            manual_column: lock.map(|c| c.id.clone()),
            manual_lock_priority: lock.map(|_| 0),
            tags: vec![],
            notes: t.notes,
            archived: false,
            bg_job_id: None,
            last_job_state: None,
            last_job_at: None,
            created_at: now,
            updated_at: now,
            done_at: None,
        };
        self.store.lock().unwrap().upsert_card(&card)?;
        self.emit_changed();
        Ok(card)
    }

    pub fn patch_card(&self, id: &str, p: CardPatch) -> anyhow::Result<Card> {
        let store = self.store.lock().unwrap();
        let Some(mut c) = store.get_card(id)? else {
            anyhow::bail!("card not found")
        };
        if let Some(v) = p.title {
            c.title = if v.trim().is_empty() { None } else { Some(v) };
        }
        if let Some(v) = p.priority {
            c.priority = v;
        }
        if let Some(v) = p.auto_run {
            c.auto_run = v;
        }
        if let Some(v) = p.run_prompt {
            c.run_prompt = if v.trim().is_empty() { None } else { Some(v) };
        }
        if let Some(v) = p.permission_mode {
            c.permission_mode = if v.trim().is_empty() { None } else { Some(v) };
        }
        if let Some(v) = p.model {
            c.model = if v.trim().is_empty() { None } else { Some(v) };
        }
        if let Some(v) = p.notes {
            c.notes = if v.trim().is_empty() { None } else { Some(v) };
        }
        if let Some(v) = p.tags {
            c.tags = v;
        }
        if let Some(v) = p.archived {
            c.archived = v;
        }
        if let Some(v) = p.estimate_weighted_tokens {
            c.estimate_weighted_tokens = Some(v);
        }
        c.updated_at = Utc::now();
        store.upsert_card(&c)?;
        drop(store);
        self.emit_changed();
        Ok(c)
    }

    pub fn delete_card(&self, id: &str) -> anyhow::Result<()> {
        self.store.lock().unwrap().delete_card(id)?;
        self.emit_changed();
        Ok(())
    }

    /// Put a card back exactly as it was. This is the undo of `delete_card`,
    /// so the board sends the card it deleted, with its own id and times. A
    /// card that is there again already gets the same content, because a
    /// second undo must not fail.
    pub fn restore_card(&self, card: Card) -> anyhow::Result<Card> {
        self.store.lock().unwrap().upsert_card(&card)?;
        self.emit_changed();
        Ok(card)
    }

    /// Store a manual order for one column. `ranked` holds the cards the user
    /// placed, top first; they get descending positive priorities. `unranked`
    /// holds the rest of that column, which go back to priority 0 and so sort
    /// automatically, below every ranked card.
    ///
    /// Priority is what the planner sorts by as well, so the top of a ranked
    /// backlog is also the first card a plan takes.
    pub fn reorder_cards(&self, ranked: &[String], unranked: &[String]) -> anyhow::Result<()> {
        let now = Utc::now();
        {
            let store = self.store.lock().unwrap();
            let n = ranked.len() as i32;
            let want = |i: usize| n - i as i32;
            for (i, id) in ranked.iter().enumerate() {
                let Some(mut c) = store.get_card(id)? else {
                    continue;
                };
                if c.priority == want(i) {
                    continue;
                }
                c.priority = want(i);
                c.updated_at = now;
                store.upsert_card(&c)?;
            }
            for id in unranked {
                let Some(mut c) = store.get_card(id)? else {
                    continue;
                };
                if c.priority == 0 {
                    continue;
                }
                c.priority = 0;
                c.updated_at = now;
                store.upsert_card(&c)?;
            }
        }
        self.emit_changed();
        Ok(())
    }

    /// The user clicked the stale mark: ask the usage endpoint now, outside the
    /// rate limit, keep the sample, and refresh the board.
    pub fn fetch_usage_now(self: &Arc<Self>) -> anyhow::Result<QuotaSample> {
        let s = quota::fetch_usage_with(true)?;
        let _ = self.store.lock().unwrap().insert_quota_sample(&s);
        {
            let mut snap = self.snap.write().unwrap();
            if snap.quota.as_ref().is_none_or(|q| s.at >= q.at) {
                snap.quota = Some(s.clone());
            }
        }
        self.emit_changed();
        Ok(s)
    }

    /// Write the automation mode and keep the two older settings in step.
    pub fn set_automation_mode(&self, mode: AutomationMode) -> anyhow::Result<()> {
        let mut s = self.settings();
        if s.automation() == mode {
            return Ok(());
        }
        s.set_automation(mode);
        self.set_settings(s)
    }

    pub fn columns(&self) -> Vec<Column> {
        self.store
            .lock()
            .unwrap()
            .load_columns()
            .unwrap_or_default()
    }

    pub fn set_columns(&self, cols: Vec<Column>) -> anyhow::Result<()> {
        if cols.is_empty() {
            anyhow::bail!("at least one column is required");
        }
        self.store.lock().unwrap().save_columns(&cols)?;
        self.emit_changed();
        Ok(())
    }

    pub fn reset_columns(&self) -> anyhow::Result<()> {
        self.set_columns(Column::defaults())
    }

    pub fn quota_history(&self, limit: usize) -> Vec<QuotaSample> {
        self.store
            .lock()
            .unwrap()
            .quota_history(limit)
            .unwrap_or_default()
    }

    /// Every project directory this node knows, for the pickers.
    pub fn projects(&self) -> Vec<Project> {
        let snap = self.snap.read().unwrap();
        let mut set: HashSet<String> = HashSet::new();
        for f in snap.facts.values() {
            if let Some(c) = &f.cwd {
                set.insert(c.clone());
            }
        }
        for l in snap.live.values() {
            set.insert(l.cwd.clone());
        }
        let mut v: Vec<Project> = set
            .into_iter()
            .map(|cwd| Project {
                name: paths::project_display_name(&cwd),
                cwd,
            })
            .collect();
        v.sort_by(|a, b| (&a.name, &a.cwd).cmp(&(&b.name, &b.cwd)));
        v
    }

    /// Repair cards whose project directory holds a display name, not a path.
    /// A swapped pair in the project list wrote values such as "kari" into new
    /// cards, and such a card can neither run nor open a terminal.
    pub fn repair_project_cwds(&self) {
        let cards = match self.store.lock().unwrap().list_cards() {
            Ok(c) => c,
            Err(e) => {
                warn!("repair_project_cwds: {e}");
                return;
            }
        };
        let broken: Vec<Card> = cards
            .into_iter()
            .filter(|c| {
                c.project_cwd
                    .as_deref()
                    .is_some_and(|p| !paths::is_usable_cwd(p))
            })
            .collect();
        if broken.is_empty() {
            return;
        }
        let known = self.projects();
        let mut fixed = 0usize;
        let mut cleared = 0usize;
        for mut card in broken {
            let bad = card.project_cwd.clone().unwrap_or_default();
            // One project with that display name is a safe answer. Two are not.
            let mut hits = known.iter().filter(|p| p.name == bad);
            match (hits.next(), hits.next()) {
                (Some(p), None) => {
                    card.project_cwd = Some(p.cwd.clone());
                    fixed += 1;
                }
                _ => {
                    card.project_cwd = None;
                    cleared += 1;
                }
            }
            card.updated_at = Utc::now();
            if let Err(e) = self.store.lock().unwrap().upsert_card(&card) {
                warn!("repair_project_cwds {}: {e}", card.id);
            }
        }
        info!("repaired {fixed} project directories, cleared {cleared}");
        self.emit_changed();
    }

    /// The model a run of this card must use. None means the Claude Code default.
    fn run_model(card: &Card, settings: &Settings) -> Option<String> {
        card.model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| Some(settings.default_run_model.clone()).filter(|m| !m.trim().is_empty()))
    }

    /// Open the session where it lives. Returns a short description of what happened.
    /// Work out what "Jump in" must do for a card, and do the part that lives on
    /// this node: focus or open a herdr pane. The returned command, when not
    /// empty, must run in a terminal where the user sits.
    pub fn jump_plan(&self, card_id: &str) -> anyhow::Result<JumpPlan> {
        let settings = self.settings();
        let board = self.board();
        let Some(cv) = board.cards.iter().find(|c| c.card.id == card_id) else {
            anyhow::bail!("card not found")
        };
        // Only a directory that exists can hold a terminal or an agent. A bad
        // value falls through to the next source, and home is the last resort.
        let cwd = [
            cv.card.project_cwd.clone(),
            cv.session.as_ref().and_then(|s| s.cwd.clone()),
            cv.live.as_ref().map(|l| l.cwd.clone()),
        ]
        .into_iter()
        .flatten()
        .find(|c| paths::is_usable_cwd(c))
        .unwrap_or_else(|| paths::home().to_string_lossy().into_owned());
        if let Some(h) = &cv.herdr {
            herdr::focus(h)?;
            return Ok(JumpPlan {
                cwd,
                command: String::new(),
                herdr_pane: Some(h.pane_id.clone()),
                message: format!("focused herdr pane {}", h.pane_id),
            });
        }
        if let Some(job) = cv.bg_job.as_ref().and_then(|j| j.id.clone()) {
            return Ok(JumpPlan {
                cwd,
                command: launcher::attach_command(&job),
                herdr_pane: None,
                message: format!("attached to background job {job}"),
            });
        }
        // herdr is the better home for a new pane when it runs.
        let herdr_ok = settings.prefer_herdr && self.snap.read().unwrap().herdr_ok;
        let model = Self::run_model(&cv.card, &settings);
        if herdr_ok {
            let mut args: Vec<String> = match &cv.card.session_id {
                Some(sid) => vec!["--resume".into(), sid.clone()],
                None => vec![],
            };
            if let Some(m) = &model {
                args.push("--model".into());
                args.push(m.clone());
            }
            match herdr::open_agent(&cwd, &cv.title, "claude", &args, true) {
                Ok(p) => {
                    self.scan_herdr();
                    self.emit_changed();
                    return Ok(JumpPlan {
                        cwd,
                        command: String::new(),
                        herdr_pane: Some(p.pane_id.clone()),
                        message: format!("opened a herdr pane {}", p.pane_id),
                    });
                }
                Err(e) => warn!("herdr launch: {e}"),
            }
        }
        if let Some(sid) = &cv.card.session_id {
            return Ok(JumpPlan {
                cwd,
                command: launcher::resume_command(sid, model.as_deref()),
                herdr_pane: None,
                message: format!("opened {}", short(sid)),
            });
        }
        // A task without a session: open a fresh Claude Code in the project.
        Ok(JumpPlan {
            cwd: cwd.clone(),
            command: launcher::new_command(model.as_deref()),
            herdr_pane: None,
            message: format!("opened a new session in {cwd}"),
        })
    }

    /// Jump in on this machine: run the plan in the configured terminal.
    pub fn jump_in(&self, card_id: &str) -> anyhow::Result<String> {
        let settings = self.settings();
        let plan = self.jump_plan(card_id)?;
        if plan.herdr_pane.is_some() {
            launcher::raise_terminal(&settings.terminal_app);
            return Ok(plan.message);
        }
        launcher::open_in_terminal(&settings.terminal_app, &plan.cwd, &plan.command)?;
        Ok(format!("{} in {}", plan.message, settings.terminal_app))
    }

    // ---------------------------------------------------------------- node identity

    /// A stable id for this node, made on first use.
    pub fn node_id(&self) -> String {
        let store = self.store.lock().unwrap();
        if let Ok(Some(id)) = store.kv_get("node_id") {
            return id;
        }
        let id = uuid::Uuid::new_v4().to_string();
        let _ = store.kv_set("node_id", &id);
        id
    }

    /// The name other kari instances show for this node.
    pub fn node_name(&self) -> String {
        let s = self.settings();
        if !s.node_name.trim().is_empty() {
            return s.node_name.trim().to_string();
        }
        paths::hostname()
    }

    pub fn identity(&self) -> NodeIdentity {
        NodeIdentity {
            ok: true,
            app: "kari".into(),
            version: crate::version().into(),
            api_version: API_VERSION,
            node_id: self.node_id(),
            node_name: self.node_name(),
            platform: std::env::consts::OS.into(),
            addresses: crate::net::bound_reachable(),
            account: crate::account::read(),
        }
    }

    /// Small facts the hub keeps beside the board, such as its primary intent.
    pub fn kv_get(&self, key: &str) -> Option<String> {
        self.store.lock().unwrap().kv_get(key).ok().flatten()
    }

    pub fn kv_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.store.lock().unwrap().kv_set(key, value)
    }

    /// The name the user gave each account, keyed the way `account::group_key`
    /// keys them. Lives in the hub's store: it is a label this device shows,
    /// not something a node knows about itself.
    pub fn account_aliases(&self) -> std::collections::HashMap<String, String> {
        self.store
            .lock()
            .unwrap()
            .kv_prefix(crate::account::ALIAS_KEY_PREFIX)
            .unwrap_or_default()
    }

    /// Name an account, or clear the name with an empty string.
    pub fn set_account_alias(&self, key: &str, alias: &str) -> anyhow::Result<()> {
        let full = format!("{}{key}", crate::account::ALIAS_KEY_PREFIX);
        let alias = alias.trim();
        let store = self.store.lock().unwrap();
        if alias.is_empty() {
            store.kv_delete(&full)?;
        } else {
            store.kv_set(&full, alias)?;
        }
        drop(store);
        self.emit_changed();
        Ok(())
    }

    // ---------------------------------------------------------------- column lease

    const LEASE_KEY: &'static str = "lease";

    /// The hub that may push columns to this node, if any.
    pub fn lease(&self) -> Option<Lease> {
        let store = self.store.lock().unwrap();
        store
            .kv_get(Self::LEASE_KEY)
            .ok()
            .flatten()
            .and_then(|j| serde_json::from_str(&j).ok())
    }

    /// True when a hub with this id may push columns: no lease, an expired
    /// lease, or its own lease. `None` is a caller without a hub id, such as a
    /// script or an older kari; it passes only while no hub holds the lease.
    pub fn lease_allows(&self, hub_id: Option<&str>) -> bool {
        match self.lease() {
            None => true,
            Some(l) if l.expired(Utc::now()) => true,
            Some(l) => hub_id == Some(l.hub_id.as_str()),
        }
    }

    /// Claim or renew the lease. One SQLite transaction, so two hubs that claim
    /// at once see one winner.
    pub fn claim_lease(&self, claim: LeaseClaim) -> anyhow::Result<Lease> {
        if claim.hub_id.trim().is_empty() {
            anyhow::bail!("a claim needs a hub id");
        }
        let now = Utc::now();
        let changed;
        let lease = {
            let store = self.store.lock().unwrap();
            let current: Option<Lease> = store
                .kv_get(Self::LEASE_KEY)?
                .and_then(|j| serde_json::from_str(&j).ok());
            let lease = match current {
                Some(l) if l.hub_id == claim.hub_id => {
                    changed = false;
                    Lease {
                        hub_name: claim.hub_name.clone(),
                        renewed_at: now,
                        ..l
                    }
                }
                Some(l) if !l.expired(now) && !claim.take => {
                    anyhow::bail!("not primary: {} holds the lease", l.hub_name);
                }
                _ => {
                    changed = true;
                    Lease {
                        hub_id: claim.hub_id.clone(),
                        hub_name: claim.hub_name.clone(),
                        claimed_at: now,
                        renewed_at: now,
                    }
                }
            };
            store.kv_set(Self::LEASE_KEY, &serde_json::to_string(&lease)?)?;
            lease
        };
        if changed {
            info!("lease taken by {} ({})", lease.hub_name, lease.hub_id);
            let _ = self.tx.send(Event::LeaseChanged);
        }
        Ok(lease)
    }

    /// Give the lease back. Only the holder can.
    pub fn release_lease(&self, hub_id: &str) -> anyhow::Result<()> {
        let released = {
            let store = self.store.lock().unwrap();
            let current: Option<Lease> = store
                .kv_get(Self::LEASE_KEY)?
                .and_then(|j| serde_json::from_str(&j).ok());
            match current {
                Some(l) if l.hub_id == hub_id => {
                    store.kv_delete(Self::LEASE_KEY)?;
                    true
                }
                Some(l) => anyhow::bail!("not primary: {} holds the lease", l.hub_name),
                None => false,
            }
        };
        if released {
            let _ = self.tx.send(Event::LeaseChanged);
        }
        Ok(())
    }

    // ---------------------------------------------------------------- remote nodes (hub side)

    pub fn list_nodes(&self) -> Vec<NodeRecord> {
        self.store.lock().unwrap().list_nodes().unwrap_or_default()
    }

    pub fn save_node(&self, n: &NodeRecord) -> anyhow::Result<()> {
        self.store.lock().unwrap().upsert_node(n)
    }

    pub fn delete_node(&self, id: &str) -> anyhow::Result<()> {
        self.store.lock().unwrap().delete_node(id)
    }

    pub fn node_cache(&self, id: &str) -> Option<(BoardView, DateTime<Utc>)> {
        self.store.lock().unwrap().node_cache(id).ok().flatten()
    }

    pub fn save_node_cache(&self, id: &str, board: &BoardView) {
        if let Err(e) = self.store.lock().unwrap().save_node_cache(id, board) {
            warn!("node cache: {e}");
        }
    }

    /// Start a card as a background job. Task cards start fresh; session cards resume.
    pub fn start_card(
        &self,
        card_id: &str,
        prompt_override: Option<String>,
    ) -> anyhow::Result<String> {
        let settings = self.settings();
        let board = self.board();
        let Some(cv) = board.cards.iter().find(|c| c.card.id == card_id) else {
            anyhow::bail!("card not found")
        };
        let card = &cv.card;
        let cwd = [
            card.project_cwd.clone(),
            cv.session.as_ref().and_then(|s| s.cwd.clone()),
        ]
        .into_iter()
        .flatten()
        .find(|c| paths::is_usable_cwd(c))
        .ok_or_else(|| match card.project_cwd.as_deref() {
            Some(bad) => anyhow::anyhow!(
                "the project directory of this card is not a directory on this node: {bad}. Set it in the card."
            ),
            None => anyhow::anyhow!("card has no project directory"),
        })?;
        // A task card joins its title and its body, so the title never has to be
        // repeated in the body. A one-off prompt replaces both.
        let prompt = match prompt_override {
            Some(p) if !p.trim().is_empty() => Some(p),
            _ => compose_prompt(card.kind, card.title.as_deref(), card.run_prompt.as_deref()),
        }
        .ok_or_else(|| anyhow::anyhow!("card has no prompt"))?;
        let mode = card
            .permission_mode
            .clone()
            .unwrap_or(settings.default_permission_mode.clone());
        let model = Self::run_model(card, &settings);
        let name = launcher::slugify(&cv.title);
        // A job that failed before its first turn leaves a session id with no transcript.
        // Such a session cannot resume, so the run starts fresh.
        let resume = card.session_id.as_deref().filter(|_| cv.session.is_some());
        let started = launcher::start_background(
            &cwd,
            &prompt,
            Some(&name),
            &mode,
            resume,
            model.as_deref(),
        )?;
        let mut c = card.clone();
        c.bg_job_id = Some(started.job_id.clone());
        c.last_job_state = Some("working".into());
        c.last_job_at = Some(Utc::now());
        c.manual_column = None;
        c.manual_lock_priority = None;
        c.updated_at = Utc::now();
        self.store.lock().unwrap().upsert_card(&c)?;
        self.scan_jobs();
        self.emit_changed();
        Ok(started.job_id)
    }

    pub fn stop_card(&self, card_id: &str) -> anyhow::Result<()> {
        let board = self.board();
        let Some(cv) = board.cards.iter().find(|c| c.card.id == card_id) else {
            anyhow::bail!("card not found")
        };
        let Some(job) = cv
            .bg_job
            .as_ref()
            .and_then(|j| j.id.clone())
            .or(cv.card.bg_job_id.clone())
        else {
            anyhow::bail!("no background job")
        };
        launcher::stop_background(&job)?;
        self.scan_jobs();
        self.emit_changed();
        Ok(())
    }

    /// Stop every background job kari started.
    pub fn stop_all(&self) -> anyhow::Result<usize> {
        let cards = self.store.lock().unwrap().list_cards()?;
        let mut n = 0;
        for c in cards {
            if let Some(j) = &c.bg_job_id {
                if launcher::stop_background(j).is_ok() {
                    n += 1;
                }
            }
        }
        self.scan_jobs();
        self.emit_changed();
        Ok(n)
    }

    // ---------------------------------------------------------------- proposals

    /// Cards that may run unattended right now.
    fn candidates(board: &BoardView) -> Vec<planner::Candidate> {
        board
            .cards
            .iter()
            .filter(|cv| {
                let c = &cv.card;
                if !c.auto_run || c.archived || c.done_at.is_some() {
                    return false;
                }
                // Never take over a session the user has open, or a job already running.
                if cv.live.is_some() {
                    return false;
                }
                if matches!(
                    cv.bg_job.as_ref().and_then(|j| j.state.as_deref()),
                    Some("working") | Some("blocked")
                ) {
                    return false;
                }
                if c.project_cwd.is_none()
                    && cv.session.as_ref().and_then(|s| s.cwd.as_ref()).is_none()
                {
                    return false;
                }
                let has_prompt = c
                    .run_prompt
                    .as_deref()
                    .is_some_and(|p| !p.trim().is_empty())
                    || (c.kind == CardKind::Task
                        && c.title.as_deref().is_some_and(|t| !t.trim().is_empty()));
                if !has_prompt {
                    return false;
                }
                // A card that waits for the user is theirs, not the planner's.
                !matches!(
                    cv.state,
                    DerivedState::NeedsDecision
                        | DerivedState::NeedsApproval
                        | DerivedState::Working
                        | DerivedState::Done
                )
            })
            .map(|cv| planner::Candidate {
                card_id: cv.card.id.clone(),
                title: cv.title.clone(),
                project_name: cv.project_name.clone(),
                prompt: compose_prompt(
                    cv.card.kind,
                    cv.card.title.as_deref(),
                    cv.card.run_prompt.as_deref(),
                ),
                model: cv.card.model.clone(),
                priority: cv.card.priority,
                created_at: cv.card.created_at,
                estimate: cv.estimate.clone().unwrap_or_else(|| {
                    estimate::estimate_for(&cv.card, &HashMap::new(), &board.calibration)
                }),
            })
            .collect()
    }

    fn planner_context<'a>(board: &'a BoardView, now: DateTime<Utc>) -> planner::Context<'a> {
        let running_jobs = board
            .cards
            .iter()
            .filter(|cv| {
                cv.card.bg_job_id.is_some()
                    && cv.bg_job.as_ref().and_then(|j| j.state.as_deref()) == Some("working")
            })
            .count() as u32;
        let last_interactive_at = board
            .cards
            .iter()
            .filter(|cv| cv.live.is_some())
            .filter_map(|cv| cv.last_activity_at)
            .max();
        let any_busy = board
            .cards
            .iter()
            .any(|cv| cv.live.as_ref().and_then(|l| l.status.as_deref()) == Some("busy"));
        planner::Context {
            now,
            quota: board.quota.as_ref(),
            running_jobs,
            last_interactive_at,
            any_busy,
        }
    }

    fn snooze_until(&self, trigger_key: &str) -> Option<DateTime<Utc>> {
        let v = self
            .store
            .lock()
            .unwrap()
            .kv_get(&format!("snooze:{trigger_key}"))
            .ok()??;
        DateTime::parse_from_rfc3339(&v)
            .ok()
            .map(|d| d.with_timezone(&Utc))
    }

    fn set_snooze(&self, trigger_key: &str, until: DateTime<Utc>) {
        let _ = self
            .store
            .lock()
            .unwrap()
            .kv_set(&format!("snooze:{trigger_key}"), &until.to_rfc3339());
    }

    /// Retire an offer nobody answered.
    fn expire_proposal(&self) {
        let now = Utc::now();
        let mut snap = self.snap.write().unwrap();
        let Some(p) = snap.proposal.as_mut() else {
            return;
        };
        if p.is_live(now) {
            return;
        }
        // Give the row a state the board never restores. An accepted plan that
        // keeps the state "accepted" comes back on every start.
        p.state = match p.state.as_str() {
            "open" => "expired".into(),
            "accepted" => "started".into(),
            other => other.to_string(),
        };
        let done = p.clone();
        snap.proposal = None;
        drop(snap);
        let _ = self.store.lock().unwrap().save_proposal(&done);
    }

    pub fn proposal(&self) -> Option<Proposal> {
        self.snap.read().unwrap().proposal.clone()
    }

    /// Check the triggers and offer a plan. Runs on a timer.
    pub fn proposal_tick(self: &Arc<Self>) {
        let settings = self.settings();
        self.snap.write().unwrap().proposal_checked_at = Some(Utc::now());
        // Retire a plan that timed out even when plans are off, so the panel
        // does not stay on the board after the user turns automation off.
        self.expire_proposal();
        if settings.automation() == AutomationMode::Off {
            return;
        }
        if self.snap.read().unwrap().proposal.is_some() {
            return; // one offer at a time
        }
        let now = Utc::now();
        let board = self.board();
        let ctx = Self::planner_context(&board, now);
        let Some((trigger, reason)) = planner::detect_trigger(&ctx, &settings) else {
            return;
        };
        if let Some(until) = self.snooze_until(trigger.key()) {
            if until > now {
                return;
            }
        }
        let Some(p) = planner::plan(trigger, reason, Self::candidates(&board), &ctx, &settings)
        else {
            return;
        };
        self.publish_proposal(p);
    }

    /// Build a plan on demand, whatever the windows look like.
    pub fn propose_now(self: &Arc<Self>) -> anyhow::Result<Proposal> {
        let settings = self.settings();
        let now = Utc::now();
        let board = self.board();
        let ctx = Self::planner_context(&board, now);
        let budget = planner::budget_pct(&ctx, &settings);
        let cands = Self::candidates(&board);
        if cands.is_empty() {
            anyhow::bail!("no card is marked may run unattended with a prompt and a project");
        }
        let p = planner::plan(
            ProposalTrigger::Manual,
            format!("you asked for a plan; budget {budget:.0} percent of the 5-hour window"),
            cands,
            &ctx,
            &settings,
        )
        .ok_or_else(|| {
            anyhow::anyhow!(
                "nothing fits: {budget:.0} percent of the 5-hour window is free and the smallest task needs more"
            )
        })?;
        self.publish_proposal(p.clone());
        Ok(p)
    }

    fn publish_proposal(self: &Arc<Self>, p: Proposal) {
        let settings = self.settings();
        let _ = self.store.lock().unwrap().save_proposal(&p);
        self.snap.write().unwrap().proposal = Some(p.clone());
        // Autopilot answers the weekly-reset trigger by itself.
        let auto = settings.automation() == AutomationMode::Auto
            && p.trigger == ProposalTrigger::WeeklyReset;
        if auto {
            let me = Arc::clone(self);
            let id = p.id.clone();
            std::thread::spawn(move || {
                if let Err(e) = me.accept_proposal(&id, None, true) {
                    warn!("autopilot: {e}");
                }
            });
            return;
        }
        let _ = self.tx.send(Event::Notice {
            title: format!("{} task(s) can fill the quota", p.items.len()),
            body: format!(
                "{}. Plan spends {:.0} percent of the 5-hour window.",
                p.reason, p.total_pct
            ),
            card_id: None,
        });
        self.emit_changed();
    }

    /// Start the items of a proposal. `card_ids` picks a subset.
    pub fn accept_proposal(
        &self,
        id: &str,
        card_ids: Option<Vec<String>>,
        auto: bool,
    ) -> anyhow::Result<usize> {
        let mut p = self
            .store
            .lock()
            .unwrap()
            .get_proposal(id)?
            .ok_or_else(|| anyhow::anyhow!("proposal not found"))?;
        if matches!(p.state.as_str(), "accepted" | "started") && card_ids.is_none() {
            anyhow::bail!("proposal already started");
        }
        let settings = self.settings();
        let mut started = 0usize;
        let cap = settings.autopilot_max_jobs.max(1) as usize;
        for item in p.items.iter_mut() {
            if let Some(pick) = &card_ids {
                if !pick.contains(&item.card_id) {
                    continue;
                }
            }
            if item.job_id.is_some() {
                continue;
            }
            if auto && started >= cap {
                break;
            }
            match self.start_card(&item.card_id, None) {
                Ok(job) => {
                    item.job_id = Some(job);
                    item.error = None;
                    started += 1;
                }
                Err(e) => item.error = Some(e.to_string()),
            }
        }
        p.state = "accepted".into();
        p.accepted_at = Some(Utc::now());
        p.auto = auto;
        self.store.lock().unwrap().save_proposal(&p)?;
        self.snap.write().unwrap().proposal = Some(p.clone());
        let errors: Vec<&str> = p.items.iter().filter_map(|i| i.error.as_deref()).collect();
        let _ = self.tx.send(Event::Notice {
            title: if auto {
                format!("Autopilot started {started} task(s)")
            } else {
                format!("Started {started} task(s)")
            },
            body: if errors.is_empty() {
                format!("{:.0} percent of the 5-hour window planned.", p.total_pct)
            } else {
                format!("{} could not start: {}", errors.len(), errors.join("; "))
            },
            card_id: None,
        });
        self.emit_changed();
        Ok(started)
    }

    /// Hold the same trigger back for a while.
    pub fn snooze_proposal(&self, id: &str, minutes: i64) -> anyhow::Result<()> {
        let mut p = self
            .store
            .lock()
            .unwrap()
            .get_proposal(id)?
            .ok_or_else(|| anyhow::anyhow!("proposal not found"))?;
        p.state = "snoozed".into();
        self.set_snooze(p.trigger.key(), Utc::now() + Duration::minutes(minutes));
        self.store.lock().unwrap().save_proposal(&p)?;
        self.snap.write().unwrap().proposal = None;
        self.emit_changed();
        Ok(())
    }

    /// Drop the offer. The same trigger stays quiet until its window moves on.
    pub fn dismiss_proposal(&self, id: &str) -> anyhow::Result<()> {
        let mut p = self
            .store
            .lock()
            .unwrap()
            .get_proposal(id)?
            .ok_or_else(|| anyhow::anyhow!("proposal not found"))?;
        p.state = "dismissed".into();
        let now = Utc::now();
        let until = match p.trigger {
            // The weekly trigger only stops when the window resets.
            ProposalTrigger::WeeklyReset => self
                .snap
                .read()
                .unwrap()
                .quota
                .as_ref()
                .and_then(|q| q.seven_day.as_ref())
                .and_then(|w| w.resets_at)
                .unwrap_or(now + Duration::hours(12)),
            ProposalTrigger::IdleFiveHour => now + Duration::hours(2),
            ProposalTrigger::Manual => now + Duration::minutes(1),
        };
        self.set_snooze(p.trigger.key(), until);
        self.store.lock().unwrap().save_proposal(&p)?;
        self.snap.write().unwrap().proposal = None;
        self.emit_changed();
        Ok(())
    }

    /// Stop every job this proposal started. The undo for an autopilot run.
    pub fn stop_proposal(&self, id: &str) -> anyhow::Result<usize> {
        let p = self
            .store
            .lock()
            .unwrap()
            .get_proposal(id)?
            .ok_or_else(|| anyhow::anyhow!("proposal not found"))?;
        let running: HashSet<String> = self
            .snap
            .read()
            .unwrap()
            .jobs
            .iter()
            .filter(|j| matches!(j.state.as_deref(), Some("working") | Some("blocked")))
            .filter_map(|j| j.id.clone())
            .collect();
        let mut n = 0;
        for item in &p.items {
            if let Some(job) = &item.job_id {
                // A job that already finished needs no stop.
                if !running.contains(job) {
                    continue;
                }
                if launcher::stop_background(job).is_ok() {
                    n += 1;
                }
            }
        }
        self.scan_jobs();
        self.emit_changed();
        Ok(n)
    }

    pub fn proposal_history(&self, limit: usize) -> Vec<Proposal> {
        self.store
            .lock()
            .unwrap()
            .list_proposals(limit)
            .unwrap_or_default()
    }

    // ---------------------------------------------------------------- notices

    fn notice_once(&self, key: &str, gap: Duration) -> bool {
        let now = Utc::now();
        let store = self.store.lock().unwrap();
        if let Ok(Some(v)) = store.kv_get(key) {
            if let Ok(t) = DateTime::parse_from_rfc3339(&v) {
                if now - t.with_timezone(&Utc) < gap {
                    return false;
                }
            }
        }
        let _ = store.kv_set(key, &now.to_rfc3339());
        true
    }

    /// Two standing warnings: quota that will expire, and a column over its limit.
    pub fn notice_tick(&self) {
        let settings = self.settings();
        let now = Utc::now();
        let quota = self.snap.read().unwrap().quota.clone();
        if let Some(seven) = quota.as_ref().and_then(|q| q.seven_day.as_ref()) {
            let unused = 100.0 - seven.used_percentage;
            if let Some(reset) = seven.resets_at {
                let left = reset - now;
                if left > Duration::zero()
                    && left <= Duration::hours(24)
                    && unused > settings.weekly_warn_unused_pct
                {
                    // One warning per weekly window.
                    if self.notice_once(
                        &format!("notice:weekly:{}", reset.timestamp()),
                        Duration::days(30),
                    ) {
                        let _ = self.tx.send(Event::Notice {
                            title: format!("{unused:.0} percent of the weekly quota expires soon"),
                            body: format!(
                                "The 7-day window resets in {} hours.",
                                left.num_hours().max(1)
                            ),
                            card_id: None,
                        });
                    }
                }
            }
        }
        let board = self.board();
        for col in board.columns.iter().filter(|c| !c.hidden) {
            let Some(limit) = col.wip_limit else { continue };
            let n = board
                .cards
                .iter()
                .filter(|cv| cv.column_id == col.id)
                .count() as u32;
            if n <= limit {
                continue;
            }
            if self.notice_once(&format!("notice:wip:{}", col.id), Duration::minutes(60)) {
                let _ = self.tx.send(Event::Notice {
                    title: format!("{} holds {} cards", col.name, n),
                    body: format!("The limit is {limit}. Finish or park something."),
                    card_id: None,
                });
            }
        }
    }

    // ---------------------------------------------------------------- watchers

    /// Start file watchers and pollers. Call once.
    pub fn start_watchers(self: &Arc<Self>) {
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("kari-initial-scan".into())
            .spawn(move || {
                me.refresh_all();
                me.repair_project_cwds();
            })
            .expect("spawn");

        // File watcher: registry, transcripts, jobs, rate limits.
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("kari-watch".into())
            .spawn(move || {
                let (tx, rx) = std::sync::mpsc::channel();
                let mut debouncer = match new_debouncer(std::time::Duration::from_millis(600), tx) {
                    Ok(d) => d,
                    Err(e) => {
                        warn!("watcher failed: {e}");
                        return;
                    }
                };
                for (p, mode) in [
                    (paths::claude_sessions_dir(), RecursiveMode::NonRecursive),
                    (paths::claude_projects_dir(), RecursiveMode::Recursive),
                    (paths::claude_jobs_dir(), RecursiveMode::Recursive),
                    (paths::kari_dir(), RecursiveMode::NonRecursive),
                ] {
                    let _ = std::fs::create_dir_all(&p);
                    if let Err(e) = debouncer.watcher().watch(Path::new(&p), mode) {
                        warn!("watch {}: {e}", p.display());
                    }
                }
                for res in rx {
                    match res {
                        Ok(events) => {
                            let jobs_dir = paths::claude_jobs_dir();
                            let touches_jobs = events.iter().any(|e| e.path.starts_with(&jobs_dir));
                            if touches_jobs {
                                me.scan_jobs();
                            }
                            me.refresh_light();
                        }
                        Err(e) => warn!("watch error: {e}"),
                    }
                }
            })
            .expect("spawn");

        // Summarizer: one Haiku call at most per tick, throttled inside summary_tick.
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("kari-summary".into())
            .spawn(move || {
                std::thread::sleep(std::time::Duration::from_secs(20));
                loop {
                    me.summary_tick();
                    std::thread::sleep(std::time::Duration::from_secs(30));
                }
            })
            .expect("spawn");

        // Poller: background jobs and herdr every 15 s, full refresh every 2 min.
        let me = Arc::clone(self);
        std::thread::Builder::new()
            .name("kari-poll".into())
            .spawn(move || {
                let mut tick: u64 = 0;
                loop {
                    std::thread::sleep(std::time::Duration::from_secs(15));
                    tick += 1;
                    if tick.is_multiple_of(8) {
                        me.refresh_all();
                    } else {
                        me.scan_jobs();
                        me.scan_herdr();
                        me.scan_quota();
                        me.detect_transitions();
                        me.emit_changed();
                    }
                    if tick.is_multiple_of(4) {
                        me.proposal_tick();
                        me.notice_tick();
                    }
                }
            })
            .expect("spawn");
    }
}

/// One line about a tool call, for a notification: the command, the file, or
/// the first words of the input.
fn summarize_input(tool: &str, input: &serde_json::Value) -> String {
    let pick = |keys: &[&str]| {
        keys.iter()
            .find_map(|k| input.get(k).and_then(|v| v.as_str()))
            .map(|s| s.to_string())
    };
    let text = match tool {
        "Bash" => pick(&["command"]),
        "Edit" | "Write" | "MultiEdit" | "Read" | "NotebookEdit" => {
            pick(&["file_path", "notebook_path"])
        }
        "WebFetch" => pick(&["url"]),
        "Agent" | "Task" => pick(&["description", "prompt"]),
        _ => pick(&[
            "command",
            "file_path",
            "url",
            "query",
            "pattern",
            "description",
        ]),
    }
    .or_else(|| input.as_str().map(|s| s.to_string()))
    .unwrap_or_else(|| {
        let raw = input.to_string();
        if raw == "null" {
            String::new()
        } else {
            raw
        }
    });
    let one_line: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    truncate(&one_line, 160)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_summary_names_the_thing() {
        let v = serde_json::json!({ "command": "cargo   test\n--workspace" });
        assert_eq!(summarize_input("Bash", &v), "cargo test --workspace");
        let v = serde_json::json!({ "file_path": "/tmp/x.rs", "old_string": "a" });
        assert_eq!(summarize_input("Edit", &v), "/tmp/x.rs");
        assert_eq!(summarize_input("Other", &serde_json::Value::Null), "");
    }

    /// The one test that opens an engine: `open_at` fixes the kari directory
    /// for the whole process, so every other test must stay away from it.
    #[test]
    fn lease_claim_rules() {
        let dir = std::env::temp_dir().join(format!("kari-lease-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        let e = Engine::open_at(&dir).unwrap();
        let claim = |id: &str, take: bool| LeaseClaim {
            hub_id: id.into(),
            hub_name: id.to_uppercase(),
            take,
        };
        assert!(e.lease().is_none());
        assert!(e.lease_allows(None), "a free lease lets an old client push");

        let a = e.claim_lease(claim("a", false)).unwrap();
        assert_eq!(a.hub_id, "a");
        assert!(e.lease_allows(Some("a")));
        assert!(!e.lease_allows(Some("b")));
        assert!(
            !e.lease_allows(None),
            "a held lease keeps an anonymous client out"
        );

        // Renewal by the holder keeps the claim time and moves the renewal time.
        let a2 = e.claim_lease(claim("a", false)).unwrap();
        assert_eq!(a2.claimed_at, a.claimed_at);
        assert!(a2.renewed_at >= a.renewed_at);

        // Another hub without `take` is refused while the lease is fresh.
        let err = e.claim_lease(claim("b", false)).unwrap_err().to_string();
        assert!(err.contains("not primary"), "{err}");
        assert_eq!(e.lease().unwrap().hub_id, "a");

        // With `take` it wins.
        let b = e.claim_lease(claim("b", true)).unwrap();
        assert_eq!(b.hub_id, "b");
        assert!(e.lease_allows(Some("b")));
        assert!(!e.lease_allows(Some("a")));

        // Only the holder releases.
        assert!(e.release_lease("a").is_err());
        e.release_lease("b").unwrap();
        assert!(e.lease().is_none());

        // An expired lease is free for anyone.
        let old = Lease {
            hub_id: "c".into(),
            hub_name: "C".into(),
            claimed_at: Utc::now() - Duration::hours(2),
            renewed_at: Utc::now() - Duration::hours(1),
        };
        e.kv_set(Engine::LEASE_KEY, &serde_json::to_string(&old).unwrap())
            .unwrap();
        assert!(e.lease_allows(None));
        assert!(e.lease_allows(Some("a")));
        let a3 = e.claim_lease(claim("a", false)).unwrap();
        assert_eq!(a3.hub_id, "a");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
