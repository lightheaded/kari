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
use tokio::sync::broadcast;
use tracing::{info, warn};

/// The first eight characters of an id, for log lines. Never slices bytes.
fn short(s: &str) -> String {
    s.chars().take(8).collect()
}

#[derive(Debug, Clone)]
pub enum Event {
    BoardChanged,
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
}

pub struct Engine {
    store: Mutex<Store>,
    snap: RwLock<Snapshot>,
    settings: RwLock<Settings>,
    tx: broadcast::Sender<Event>,
    scanning: AtomicBool,
}

impl Engine {
    pub fn open() -> anyhow::Result<Arc<Engine>> {
        let store = Store::open(&paths::kari_db())?;
        let settings = store.load_settings()?;
        let facts = store.load_facts()?;
        let summaries = store.load_summaries().unwrap_or_default();
        let (tx, _) = broadcast::channel(64);
        let mut snap = Snapshot {
            facts,
            summaries,
            hooks_installed: hooks::installed(),
            ..Default::default()
        };
        snap.proposal = store
            .latest_proposal(&["open", "accepted"])
            .unwrap_or_default();
        // Seed the job states kari already knows, so a restart logs nothing twice.
        for c in store.list_cards().unwrap_or_default() {
            if let (Some(job), Some(state)) = (c.bg_job_id, c.last_job_state) {
                snap.job_states.insert(job, state);
            }
        }
        Ok(Arc::new(Engine {
            store: Mutex::new(store),
            snap: RwLock::new(snap),
            settings: RwLock::new(settings),
            tx,
            scanning: AtomicBool::new(false),
        }))
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
        // Link jobs to cards by session id when kari did not start them.
        let jobs: Vec<BgJob> = self.snap.read().unwrap().jobs.clone();
        for j in jobs {
            if let (Some(sid), Some(_id)) = (&j.session_id, &j.id) {
                let _ = self.ensure_session_card(sid, j.cwd.as_deref());
            }
        }
        self.track_job_states();
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

    pub fn install_hooks(&self) -> anyhow::Result<String> {
        let port = self.settings().hooks_port;
        let p = hooks::install(port)?;
        self.snap.write().unwrap().hooks_installed = true;
        self.emit_changed();
        Ok(format!("hooks installed, relay at {}", p.display()))
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
            // A session that never got a prompt and has no process is noise.
            if card.kind == CardKind::Session
                && live.is_none()
                && bg.is_none()
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
        drop(snap);
        if !lock_breaks.is_empty() {
            let store = self.store.lock().unwrap();
            for c in lock_breaks {
                let _ = store.upsert_card(&c);
            }
        }
        BoardView {
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
        }
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
        if col.accepts.contains(&DerivedState::Ready) && card.kind == CardKind::Task {
            card.auto_run = true;
            if card.run_prompt.is_none() {
                card.run_prompt = card.title.clone();
            }
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
        let card = Card {
            id: uuid::Uuid::new_v4().to_string(),
            kind: CardKind::Task,
            title: Some(t.title),
            session_id: None,
            project_cwd: t.project_cwd,
            priority: t.priority,
            auto_run: t.auto_run,
            run_prompt: t.run_prompt,
            permission_mode: None,
            model: t.model.filter(|m| !m.trim().is_empty()),
            estimate_weighted_tokens: None,
            manual_column: None,
            manual_lock_priority: None,
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

    pub fn projects(&self) -> Vec<(String, String)> {
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
        let mut v: Vec<(String, String)> = set
            .into_iter()
            .map(|c| (paths::project_display_name(&c), c))
            .collect();
        v.sort();
        v
    }

    /// The model a run of this card must use. None means the Claude Code default.
    fn run_model(card: &Card, settings: &Settings) -> Option<String> {
        card.model
            .clone()
            .filter(|m| !m.trim().is_empty())
            .or_else(|| Some(settings.default_run_model.clone()).filter(|m| !m.trim().is_empty()))
    }

    /// Open the session where it lives. Returns a short description of what happened.
    pub fn jump_in(&self, card_id: &str) -> anyhow::Result<String> {
        let settings = self.settings();
        let board = self.board();
        let Some(cv) = board.cards.iter().find(|c| c.card.id == card_id) else {
            anyhow::bail!("card not found")
        };
        if let Some(h) = &cv.herdr {
            launcher::focus_herdr(h, &settings.terminal_app)?;
            return Ok(format!("focused herdr pane {}", h.pane_id));
        }
        let cwd = cv
            .card
            .project_cwd
            .clone()
            .or_else(|| cv.session.as_ref().and_then(|s| s.cwd.clone()))
            .or_else(|| cv.live.as_ref().map(|l| l.cwd.clone()))
            .unwrap_or_else(|| paths::home().to_string_lossy().into_owned());
        if let Some(job) = cv.bg_job.as_ref().and_then(|j| j.id.clone()) {
            launcher::open_in_terminal(
                &settings.terminal_app,
                &cwd,
                &launcher::attach_command(&job),
            )?;
            return Ok(format!("attached to background job {job}"));
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
                    return Ok(format!("opened a herdr pane {}", p.pane_id));
                }
                Err(e) => warn!("herdr launch: {e}"),
            }
        }
        if let Some(sid) = &cv.card.session_id {
            launcher::open_in_terminal(
                &settings.terminal_app,
                &cwd,
                &launcher::resume_command(sid, model.as_deref()),
            )?;
            return Ok(format!(
                "opened {} in {}",
                short(sid),
                settings.terminal_app
            ));
        }
        // A task without a session: open a fresh Claude Code in the project.
        launcher::open_in_terminal(
            &settings.terminal_app,
            &cwd,
            &launcher::new_command(model.as_deref()),
        )?;
        Ok(format!("opened a new session in {cwd}"))
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
        let cwd = card
            .project_cwd
            .clone()
            .or_else(|| cv.session.as_ref().and_then(|s| s.cwd.clone()))
            .ok_or_else(|| anyhow::anyhow!("card has no project directory"))?;
        let prompt = prompt_override
            .or_else(|| card.run_prompt.clone())
            .or_else(|| card.title.clone())
            .ok_or_else(|| anyhow::anyhow!("card has no prompt"))?;
        let mode = card
            .permission_mode
            .clone()
            .unwrap_or(settings.default_permission_mode.clone());
        let model = Self::run_model(card, &settings);
        let name = launcher::slugify(&cv.title);
        let started = launcher::start_background(
            &cwd,
            &prompt,
            Some(&name),
            &mode,
            card.session_id.as_deref(),
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
                prompt: cv.card.run_prompt.clone().or_else(|| cv.card.title.clone()),
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
        let stale_accept = p.state == "accepted"
            && p.accepted_at
                .is_none_or(|t| now - t > Duration::minutes(30));
        if p.state == "open" && p.expires_at < now {
            p.state = "expired".into();
        } else if !stale_accept {
            return;
        }
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
        if !settings.proposals_enabled {
            return;
        }
        self.expire_proposal();
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
        let auto = settings.autopilot && p.trigger == ProposalTrigger::WeeklyReset;
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
        if p.state == "accepted" && card_ids.is_none() {
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
            .spawn(move || me.refresh_all())
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
