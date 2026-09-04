//! SQLite persistence at `~/.config/kari/kari.db`.

use crate::model::{
    BoardView, Card, CardKind, Column, HookEvent, JobLogEntry, NodeRecord, Proposal, QuotaSample,
    QuotaWindow, SessionFacts, Settings, Summary, TokenDelta,
};
use chrono::{DateTime, TimeZone, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use std::collections::HashMap;
use std::path::Path;

pub struct Store {
    conn: Connection,
}

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS cards (
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,
  title TEXT,
  session_id TEXT UNIQUE,
  project_cwd TEXT,
  priority INTEGER NOT NULL DEFAULT 0,
  auto_run INTEGER NOT NULL DEFAULT 0,
  run_prompt TEXT,
  permission_mode TEXT,
  estimate REAL,
  manual_column TEXT,
  manual_lock_priority INTEGER,
  tags TEXT NOT NULL DEFAULT '[]',
  notes TEXT,
  archived INTEGER NOT NULL DEFAULT 0,
  bg_job_id TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  done_at TEXT
);
CREATE TABLE IF NOT EXISTS columns (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  ord INTEGER NOT NULL,
  accepts TEXT NOT NULL,
  wip_limit INTEGER,
  color TEXT,
  hidden INTEGER NOT NULL DEFAULT 0
);
CREATE TABLE IF NOT EXISTS transcripts (
  session_id TEXT PRIMARY KEY,
  facts TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS quota_samples (
  at INTEGER NOT NULL,
  five_hour_pct REAL,
  five_hour_reset INTEGER,
  seven_day_pct REAL,
  seven_day_reset INTEGER,
  source TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS quota_samples_at ON quota_samples(at);
CREATE TABLE IF NOT EXISTS settings (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS summaries (
  session_id TEXT PRIMARY KEY,
  json TEXT NOT NULL,
  generated_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS hook_log (
  at INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  event TEXT NOT NULL,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS hook_log_at ON hook_log(at);
CREATE TABLE IF NOT EXISTS token_deltas (
  at INTEGER NOT NULL,
  session_id TEXT NOT NULL,
  weighted REAL NOT NULL
);
CREATE INDEX IF NOT EXISTS token_deltas_at ON token_deltas(at);
CREATE TABLE IF NOT EXISTS job_log (
  at INTEGER NOT NULL,
  job_id TEXT NOT NULL,
  card_id TEXT,
  state TEXT,
  detail TEXT
);
CREATE INDEX IF NOT EXISTS job_log_card ON job_log(card_id, at);
CREATE TABLE IF NOT EXISTS proposals (
  id TEXT PRIMARY KEY,
  json TEXT NOT NULL,
  created_at INTEGER NOT NULL,
  state TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS proposals_created ON proposals(created_at);
CREATE TABLE IF NOT EXISTS kv (
  key TEXT PRIMARY KEY,
  value TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS nodes (
  id TEXT PRIMARY KEY,
  json TEXT NOT NULL,
  created_at INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS node_cache (
  node_id TEXT PRIMARY KEY,
  board TEXT NOT NULL,
  seen_at INTEGER NOT NULL
);
"#;

/// Add columns that later versions of kari need. Older databases keep their rows.
fn migrate(conn: &Connection) -> anyhow::Result<()> {
    let mut have: Vec<String> = vec![];
    {
        let mut stmt = conn.prepare("PRAGMA table_info(cards)")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(1))?;
        for r in rows.flatten() {
            have.push(r);
        }
    }
    for (name, decl) in [
        ("last_job_state", "TEXT"),
        ("last_job_at", "TEXT"),
        ("model", "TEXT"),
    ] {
        if !have.iter().any(|h| h == name) {
            conn.execute(&format!("ALTER TABLE cards ADD COLUMN {name} {decl}"), [])?;
        }
    }
    Ok(())
}

/// Older builds stored the `claude --bg` job id with the terminal colour codes around it.
/// Such an id matches no job, so the card and its session drift apart.
fn repair_job_ids(conn: &Connection) -> anyhow::Result<()> {
    let mut stmt =
        conn.prepare("SELECT id, bg_job_id FROM cards WHERE instr(bg_job_id, char(27)) > 0")?;
    let rows: Vec<(String, String)> = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))?
        .filter_map(|r| r.ok())
        .collect();
    for (id, raw) in rows {
        let clean = crate::launcher::strip_ansi(&raw).trim().to_string();
        conn.execute(
            "UPDATE cards SET bg_job_id = ?2 WHERE id = ?1",
            params![id, clean],
        )?;
    }
    Ok(())
}

/// Older builds made a session card for every summarizer run. Those runs keep no
/// transcript, so their cards can never show anything. Remove the ones nobody touched.
fn prune_internal_cards(conn: &Connection) -> anyhow::Result<()> {
    let internal = crate::paths::internal_cwd_prefix();
    let n = conn.execute(
        "DELETE FROM cards WHERE kind = 'session' AND notes IS NULL AND tags = '[]'
           AND (project_cwd = ?1 OR project_cwd LIKE ?1 || '/%')
           AND session_id NOT IN (SELECT session_id FROM transcripts)",
        params![internal],
    )?;
    if n > 0 {
        tracing::info!("removed {n} internal session card(s)");
    }
    Ok(())
}

fn parse_ts(s: Option<String>) -> Option<DateTime<Utc>> {
    s.and_then(|s| DateTime::parse_from_rfc3339(&s).ok())
        .map(|d| d.with_timezone(&Utc))
}

impl Store {
    pub fn open(path: &Path) -> anyhow::Result<Store> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=NORMAL;")?;
        conn.execute_batch(SCHEMA)?;
        migrate(&conn)?;
        repair_job_ids(&conn)?;
        prune_internal_cards(&conn)?;
        let store = Store { conn };
        if store.load_columns()?.is_empty() {
            store.save_columns(&Column::defaults())?;
        } else {
            store.merge_legacy_columns()?;
        }
        Ok(store)
    }

    /// Replace the nine columns kari shipped up to version 0.4.1 with the six
    /// the board uses now. A layout the user changed is left alone.
    ///
    /// Manual locks follow their column: the three columns that waited for the
    /// user become "Needs me", and the two review columns become "Review". The
    /// key `notice.columns_merged` tells the app to say so once.
    fn merge_legacy_columns(&self) -> anyhow::Result<()> {
        let stored = self.load_columns()?;
        if !Column::same_layout(&stored, &Column::legacy_defaults()) {
            return Ok(());
        }
        self.save_columns(&Column::defaults())?;
        let mut moved = 0usize;
        for mut c in self.list_cards()? {
            let Some(old) = c.manual_column.as_deref() else {
                continue;
            };
            let Some(new) = Column::migrated_column_id(old) else {
                continue;
            };
            c.manual_column = Some(new.into());
            self.upsert_card(&c)?;
            moved += 1;
        }
        self.kv_set("notice.columns_merged", &moved.to_string())?;
        Ok(())
    }

    // ---- columns ----

    pub fn load_columns(&self) -> anyhow::Result<Vec<Column>> {
        let mut stmt = self.conn.prepare(
            "SELECT id, name, ord, accepts, wip_limit, color, hidden FROM columns ORDER BY ord",
        )?;
        let rows = stmt.query_map([], |r| {
            let accepts: String = r.get(3)?;
            Ok(Column {
                id: r.get(0)?,
                name: r.get(1)?,
                order: r.get(2)?,
                accepts: serde_json::from_str(&accepts).unwrap_or_default(),
                wip_limit: r.get::<_, Option<i64>>(4)?.map(|v| v as u32),
                color: r.get(5)?,
                hidden: r.get::<_, i64>(6)? != 0,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn save_columns(&self, cols: &[Column]) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        tx.execute("DELETE FROM columns", [])?;
        for c in cols {
            tx.execute(
                "INSERT INTO columns (id, name, ord, accepts, wip_limit, color, hidden) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                params![
                    c.id,
                    c.name,
                    c.order,
                    serde_json::to_string(&c.accepts)?,
                    c.wip_limit.map(|v| v as i64),
                    c.color,
                    c.hidden as i64
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- cards ----

    fn row_to_card(r: &rusqlite::Row<'_>) -> rusqlite::Result<Card> {
        let kind: String = r.get(1)?;
        let tags: String = r.get(12)?;
        Ok(Card {
            id: r.get(0)?,
            kind: if kind == "task" {
                CardKind::Task
            } else {
                CardKind::Session
            },
            title: r.get(2)?,
            session_id: r.get(3)?,
            project_cwd: r.get(4)?,
            priority: r.get(5)?,
            auto_run: r.get::<_, i64>(6)? != 0,
            run_prompt: r.get(7)?,
            permission_mode: r.get(8)?,
            estimate_weighted_tokens: r.get(9)?,
            manual_column: r.get(10)?,
            manual_lock_priority: r.get::<_, Option<i64>>(11)?.map(|v| v as u8),
            tags: serde_json::from_str(&tags).unwrap_or_default(),
            notes: r.get(13)?,
            archived: r.get::<_, i64>(14)? != 0,
            bg_job_id: r.get(15)?,
            created_at: parse_ts(r.get(16)?).unwrap_or_else(Utc::now),
            updated_at: parse_ts(r.get(17)?).unwrap_or_else(Utc::now),
            done_at: parse_ts(r.get(18)?),
            last_job_state: r.get(19)?,
            last_job_at: parse_ts(r.get(20)?),
            model: r.get(21)?,
        })
    }

    const CARD_COLS: &'static str = "id, kind, title, session_id, project_cwd, priority, auto_run, run_prompt, permission_mode, estimate, manual_column, manual_lock_priority, tags, notes, archived, bg_job_id, created_at, updated_at, done_at, last_job_state, last_job_at, model";

    pub fn list_cards(&self) -> anyhow::Result<Vec<Card>> {
        let sql = format!("SELECT {} FROM cards", Self::CARD_COLS);
        let mut stmt = self.conn.prepare(&sql)?;
        let rows = stmt.query_map([], Self::row_to_card)?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    pub fn get_card(&self, id: &str) -> anyhow::Result<Option<Card>> {
        let sql = format!("SELECT {} FROM cards WHERE id = ?1", Self::CARD_COLS);
        Ok(self
            .conn
            .query_row(&sql, params![id], Self::row_to_card)
            .optional()?)
    }

    pub fn card_by_session(&self, session_id: &str) -> anyhow::Result<Option<Card>> {
        let sql = format!(
            "SELECT {} FROM cards WHERE session_id = ?1",
            Self::CARD_COLS
        );
        Ok(self
            .conn
            .query_row(&sql, params![session_id], Self::row_to_card)
            .optional()?)
    }

    pub fn upsert_card(&self, c: &Card) -> anyhow::Result<()> {
        self.conn.execute(
            &format!(
                "INSERT INTO cards ({}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22)
                 ON CONFLICT(id) DO UPDATE SET kind=excluded.kind, title=excluded.title, session_id=excluded.session_id,
                 project_cwd=excluded.project_cwd, priority=excluded.priority, auto_run=excluded.auto_run,
                 run_prompt=excluded.run_prompt, permission_mode=excluded.permission_mode, estimate=excluded.estimate,
                 manual_column=excluded.manual_column, manual_lock_priority=excluded.manual_lock_priority, tags=excluded.tags,
                 notes=excluded.notes, archived=excluded.archived, bg_job_id=excluded.bg_job_id, updated_at=excluded.updated_at,
                 done_at=excluded.done_at, last_job_state=excluded.last_job_state, last_job_at=excluded.last_job_at,
                 model=excluded.model",
                Self::CARD_COLS
            ),
            params![
                c.id,
                match c.kind { CardKind::Task => "task", CardKind::Session => "session" },
                c.title,
                c.session_id,
                c.project_cwd,
                c.priority,
                c.auto_run as i64,
                c.run_prompt,
                c.permission_mode,
                c.estimate_weighted_tokens,
                c.manual_column,
                c.manual_lock_priority.map(|v| v as i64),
                serde_json::to_string(&c.tags)?,
                c.notes,
                c.archived as i64,
                c.bg_job_id,
                c.created_at.to_rfc3339(),
                c.updated_at.to_rfc3339(),
                c.done_at.map(|d| d.to_rfc3339()),
                c.last_job_state,
                c.last_job_at.map(|d| d.to_rfc3339()),
                c.model,
            ],
        )?;
        Ok(())
    }

    pub fn delete_card(&self, id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM cards WHERE id = ?1", params![id])?;
        Ok(())
    }

    // ---- transcript facts cache ----

    pub fn load_facts(&self) -> anyhow::Result<HashMap<String, SessionFacts>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, facts FROM transcripts")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = HashMap::new();
        for (id, json) in rows.flatten() {
            if let Ok(f) = serde_json::from_str::<SessionFacts>(&json) {
                out.insert(id, f);
            }
        }
        Ok(out)
    }

    pub fn save_facts(&self, facts: &SessionFacts) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO transcripts (session_id, facts) VALUES (?1, ?2) ON CONFLICT(session_id) DO UPDATE SET facts=excluded.facts",
            params![facts.session_id, serde_json::to_string(facts)?],
        )?;
        Ok(())
    }

    pub fn save_facts_batch(&self, all: &[&SessionFacts]) -> anyhow::Result<()> {
        let tx = self.conn.unchecked_transaction()?;
        for f in all {
            tx.execute(
                "INSERT INTO transcripts (session_id, facts) VALUES (?1, ?2) ON CONFLICT(session_id) DO UPDATE SET facts=excluded.facts",
                params![f.session_id, serde_json::to_string(f)?],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    // ---- quota ----

    pub fn insert_quota_sample(&self, s: &QuotaSample) -> anyhow::Result<()> {
        // Skip an identical timestamp: the file is re-read on every refresh.
        let exists: Option<i64> = self
            .conn
            .query_row(
                "SELECT 1 FROM quota_samples WHERE at = ?1 AND source = ?2",
                params![s.at.timestamp(), s.source],
                |r| r.get(0),
            )
            .optional()?;
        if exists.is_some() {
            return Ok(());
        }
        self.conn.execute(
            "INSERT INTO quota_samples (at, five_hour_pct, five_hour_reset, seven_day_pct, seven_day_reset, source) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
            params![
                s.at.timestamp(),
                s.five_hour.as_ref().map(|w| w.used_percentage),
                s.five_hour.as_ref().and_then(|w| w.resets_at).map(|d| d.timestamp()),
                s.seven_day.as_ref().map(|w| w.used_percentage),
                s.seven_day.as_ref().and_then(|w| w.resets_at).map(|d| d.timestamp()),
                s.source,
            ],
        )?;
        Ok(())
    }

    pub fn quota_history(&self, limit: usize) -> anyhow::Result<Vec<QuotaSample>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, five_hour_pct, five_hour_reset, seven_day_pct, seven_day_reset, source FROM quota_samples ORDER BY at DESC LIMIT ?1",
        )?;
        let rows = stmt.query_map(params![limit as i64], |r| {
            let w = |p: Option<f64>, reset: Option<i64>| {
                p.map(|used| QuotaWindow {
                    used_percentage: used,
                    resets_at: reset.and_then(|s| Utc.timestamp_opt(s, 0).single()),
                })
            };
            Ok(QuotaSample {
                at: Utc
                    .timestamp_opt(r.get::<_, i64>(0)?, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                five_hour: w(r.get(1)?, r.get(2)?),
                seven_day: w(r.get(3)?, r.get(4)?),
                source: r.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ---- settings ----

    pub fn load_settings(&self) -> anyhow::Result<Settings> {
        let v: Option<String> = self
            .conn
            .query_row(
                "SELECT value FROM settings WHERE key = 'settings'",
                [],
                |r| r.get(0),
            )
            .optional()?;
        let mut s: Settings = v
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        // A machine set up before the interface picker had a switch that
        // bound every private address. Keep it answering.
        if s.listen_private && s.listen_on.is_empty() {
            s.listen_on = "*".into();
        }
        s.listen_private = false;
        Ok(s)
    }

    pub fn save_settings(&self, s: &Settings) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO settings (key, value) VALUES ('settings', ?1) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![serde_json::to_string(s)?],
        )?;
        Ok(())
    }

    // ---- summaries ----

    pub fn load_summaries(&self) -> anyhow::Result<HashMap<String, Summary>> {
        let mut stmt = self
            .conn
            .prepare("SELECT session_id, json FROM summaries")?;
        let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        let mut out = HashMap::new();
        for (id, json) in rows.flatten() {
            if let Ok(s) = serde_json::from_str::<Summary>(&json) {
                out.insert(id, s);
            }
        }
        Ok(out)
    }

    pub fn save_summary(&self, s: &Summary) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO summaries (session_id, json, generated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(session_id) DO UPDATE SET json=excluded.json, generated_at=excluded.generated_at",
            params![s.session_id, serde_json::to_string(s)?, s.generated_at.timestamp()],
        )?;
        Ok(())
    }

    /// Count summary calls made by Haiku in the last hour, across restarts.
    pub fn summaries_in_last_hour(&self) -> anyhow::Result<u32> {
        let since = Utc::now().timestamp() - 3600;
        let n: i64 = self.conn.query_row(
            "SELECT COUNT(*) FROM summaries WHERE generated_at > ?1 AND json LIKE '%\"source\":\"haiku\"%'",
            params![since],
            |r| r.get(0),
        )?;
        Ok(n as u32)
    }

    // ---- token deltas and calibration input ----

    /// Record the weighted-token growth kari saw in one scan.
    pub fn insert_token_delta(&self, d: &TokenDelta) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO token_deltas (at, session_id, weighted) VALUES (?1, ?2, ?3)",
            params![d.at.timestamp(), d.session_id, d.weighted],
        )?;
        Ok(())
    }

    pub fn insert_token_deltas(&self, all: &[TokenDelta]) -> anyhow::Result<()> {
        if all.is_empty() {
            return Ok(());
        }
        let tx = self.conn.unchecked_transaction()?;
        for d in all {
            tx.execute(
                "INSERT INTO token_deltas (at, session_id, weighted) VALUES (?1, ?2, ?3)",
                params![d.at.timestamp(), d.session_id, d.weighted],
            )?;
        }
        tx.execute(
            "DELETE FROM token_deltas WHERE at < ?1",
            params![Utc::now().timestamp() - 14 * 86400],
        )?;
        tx.commit()?;
        Ok(())
    }

    pub fn token_deltas_since(&self, secs: i64) -> anyhow::Result<Vec<TokenDelta>> {
        let since = Utc::now().timestamp() - secs;
        let mut stmt = self.conn.prepare(
            "SELECT at, session_id, weighted FROM token_deltas WHERE at > ?1 ORDER BY at",
        )?;
        let rows = stmt.query_map(params![since], |r| {
            Ok(TokenDelta {
                at: Utc
                    .timestamp_opt(r.get::<_, i64>(0)?, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                session_id: r.get(1)?,
                weighted: r.get(2)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    /// Samples in the window, oldest first, as calibration needs them.
    pub fn quota_samples_since(&self, secs: i64) -> anyhow::Result<Vec<QuotaSample>> {
        let since = Utc::now().timestamp() - secs;
        let mut stmt = self.conn.prepare(
            "SELECT at, five_hour_pct, five_hour_reset, seven_day_pct, seven_day_reset, source FROM quota_samples WHERE at > ?1 ORDER BY at",
        )?;
        let rows = stmt.query_map(params![since], |r| {
            let w = |p: Option<f64>, reset: Option<i64>| {
                p.map(|used| QuotaWindow {
                    used_percentage: used,
                    resets_at: reset.and_then(|s| Utc.timestamp_opt(s, 0).single()),
                })
            };
            Ok(QuotaSample {
                at: Utc
                    .timestamp_opt(r.get::<_, i64>(0)?, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                five_hour: w(r.get(1)?, r.get(2)?),
                seven_day: w(r.get(3)?, r.get(4)?),
                source: r.get(5)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ---- run log ----

    pub fn log_job(&self, e: &JobLogEntry) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO job_log (at, job_id, card_id, state, detail) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![e.at.timestamp(), e.job_id, e.card_id, e.state, e.detail],
        )?;
        self.conn.execute(
            "DELETE FROM job_log WHERE at < ?1",
            params![Utc::now().timestamp() - 30 * 86400],
        )?;
        Ok(())
    }

    /// The card that ran a job, from the run log. Finds an older run after the
    /// card moved on to a newer job.
    pub fn card_id_for_job(&self, job_id: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row(
                "SELECT card_id FROM job_log WHERE job_id = ?1 AND card_id IS NOT NULL ORDER BY at DESC LIMIT 1",
                params![job_id],
                |r| r.get::<_, String>(0),
            )
            .optional()?)
    }

    pub fn job_log(&self, card_id: &str, limit: usize) -> anyhow::Result<Vec<JobLogEntry>> {
        let mut stmt = self.conn.prepare(
            "SELECT at, job_id, card_id, state, detail FROM job_log WHERE card_id = ?1 ORDER BY at DESC LIMIT ?2",
        )?;
        let rows = stmt.query_map(params![card_id, limit as i64], |r| {
            Ok(JobLogEntry {
                at: Utc
                    .timestamp_opt(r.get::<_, i64>(0)?, 0)
                    .single()
                    .unwrap_or_else(Utc::now),
                job_id: r.get(1)?,
                card_id: r.get(2)?,
                state: r.get(3)?,
                detail: r.get(4)?,
            })
        })?;
        Ok(rows.filter_map(|r| r.ok()).collect())
    }

    // ---- proposals ----

    pub fn save_proposal(&self, p: &Proposal) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO proposals (id, json, created_at, state) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET json=excluded.json, state=excluded.state",
            params![
                p.id,
                serde_json::to_string(p)?,
                p.created_at.timestamp(),
                p.state
            ],
        )?;
        self.conn.execute(
            "DELETE FROM proposals WHERE created_at < ?1",
            params![Utc::now().timestamp() - 30 * 86400],
        )?;
        Ok(())
    }

    pub fn get_proposal(&self, id: &str) -> anyhow::Result<Option<Proposal>> {
        let json: Option<String> = self
            .conn
            .query_row(
                "SELECT json FROM proposals WHERE id = ?1",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    /// The newest proposal in any of these states.
    pub fn latest_proposal(&self, states: &[&str]) -> anyhow::Result<Option<Proposal>> {
        let list = states
            .iter()
            .map(|s| format!("'{s}'"))
            .collect::<Vec<_>>()
            .join(",");
        let sql = format!(
            "SELECT json FROM proposals WHERE state IN ({list}) ORDER BY created_at DESC LIMIT 1"
        );
        let json: Option<String> = self.conn.query_row(&sql, [], |r| r.get(0)).optional()?;
        Ok(json.and_then(|j| serde_json::from_str(&j).ok()))
    }

    pub fn list_proposals(&self, limit: usize) -> anyhow::Result<Vec<Proposal>> {
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM proposals ORDER BY created_at DESC LIMIT ?1")?;
        let rows = stmt.query_map(params![limit as i64], |r| r.get::<_, String>(0))?;
        Ok(rows
            .filter_map(|r| r.ok())
            .filter_map(|j| serde_json::from_str(&j).ok())
            .collect())
    }

    // ---- small key-value state ----

    pub fn kv_get(&self, key: &str) -> anyhow::Result<Option<String>> {
        Ok(self
            .conn
            .query_row("SELECT value FROM kv WHERE key = ?1", params![key], |r| {
                r.get(0)
            })
            .optional()?)
    }

    pub fn kv_delete(&self, key: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM kv WHERE key = ?1", params![key])?;
        Ok(())
    }

    pub fn kv_set(&self, key: &str, value: &str) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO kv (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
            params![key, value],
        )?;
        Ok(())
    }

    // ---- hook log ----

    // ---- nodes ----

    pub fn list_nodes(&self) -> anyhow::Result<Vec<NodeRecord>> {
        let mut stmt = self
            .conn
            .prepare("SELECT json FROM nodes ORDER BY created_at, id")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        let mut out = vec![];
        for j in rows.flatten() {
            if let Ok(n) = serde_json::from_str::<NodeRecord>(&j) {
                out.push(n);
            }
        }
        Ok(out)
    }

    pub fn upsert_node(&self, n: &NodeRecord) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO nodes (id, json, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(id) DO UPDATE SET json = excluded.json",
            params![n.id, serde_json::to_string(n)?, n.created_at.timestamp()],
        )?;
        Ok(())
    }

    pub fn delete_node(&self, id: &str) -> anyhow::Result<()> {
        self.conn
            .execute("DELETE FROM nodes WHERE id = ?1", params![id])?;
        self.conn
            .execute("DELETE FROM node_cache WHERE node_id = ?1", params![id])?;
        Ok(())
    }

    /// The last board a remote node sent, for the time it is offline.
    pub fn node_cache(&self, node_id: &str) -> anyhow::Result<Option<(BoardView, DateTime<Utc>)>> {
        let row: Option<(String, i64)> = self
            .conn
            .query_row(
                "SELECT board, seen_at FROM node_cache WHERE node_id = ?1",
                params![node_id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(row.and_then(|(b, at)| {
            let board = serde_json::from_str::<BoardView>(&b).ok()?;
            let at = Utc.timestamp_opt(at, 0).single()?;
            Some((board, at))
        }))
    }

    pub fn save_node_cache(&self, node_id: &str, board: &BoardView) -> anyhow::Result<()> {
        self.conn.execute(
            "INSERT INTO node_cache (node_id, board, seen_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(node_id) DO UPDATE SET board = excluded.board, seen_at = excluded.seen_at",
            params![
                node_id,
                serde_json::to_string(board)?,
                Utc::now().timestamp()
            ],
        )?;
        Ok(())
    }

    pub fn log_hook(&self, e: &HookEvent) -> anyhow::Result<()> {
        let detail = e.notification_type.clone().or_else(|| e.tool_name.clone());
        self.conn.execute(
            "INSERT INTO hook_log (at, session_id, event, detail) VALUES (?1, ?2, ?3, ?4)",
            params![e.at.timestamp(), e.session_id, e.event, detail],
        )?;
        // Keep the log small.
        self.conn.execute(
            "DELETE FROM hook_log WHERE at < ?1",
            params![Utc::now().timestamp() - 7 * 86400],
        )?;
        Ok(())
    }
}
