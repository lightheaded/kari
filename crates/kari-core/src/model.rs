//! Domain types shared by the core, the Tauri shell and the UI (via serde).

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// State derived from all signals. Columns accept sets of these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DerivedState {
    Backlog,
    Ready,
    Working,
    MyTurn,
    NeedsDecision,
    NeedsApproval,
    WaitingOnOthers,
    Validate,
    Done,
    Stale,
    Unknown,
}

impl DerivedState {
    /// Higher wins when a manual lock competes with a fresh signal.
    pub fn priority(self) -> u8 {
        match self {
            DerivedState::NeedsApproval => 90,
            DerivedState::NeedsDecision => 85,
            DerivedState::Working => 70,
            DerivedState::Validate => 60,
            DerivedState::MyTurn => 50,
            DerivedState::WaitingOnOthers => 40,
            DerivedState::Done => 30,
            DerivedState::Ready => 22,
            DerivedState::Backlog => 20,
            DerivedState::Stale => 10,
            DerivedState::Unknown => 0,
        }
    }

    /// States that always break a manual lock: they need the user.
    pub fn breaks_lock(self) -> bool {
        matches!(
            self,
            DerivedState::NeedsApproval | DerivedState::NeedsDecision
        )
    }

    pub fn all() -> &'static [DerivedState] {
        use DerivedState::*;
        &[
            Backlog,
            Ready,
            Working,
            MyTurn,
            NeedsDecision,
            NeedsApproval,
            WaitingOnOthers,
            Validate,
            Done,
            Stale,
            Unknown,
        ]
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Column {
    pub id: String,
    pub name: String,
    pub order: i32,
    pub accepts: Vec<DerivedState>,
    pub wip_limit: Option<u32>,
    pub color: Option<String>,
    pub hidden: bool,
}

impl Column {
    pub fn defaults() -> Vec<Column> {
        use DerivedState::*;
        let mk = |id: &str, name: &str, order: i32, accepts: &[DerivedState], color: &str| Column {
            id: id.into(),
            name: name.into(),
            order,
            accepts: accepts.to_vec(),
            wip_limit: None,
            color: Some(color.into()),
            hidden: false,
        };
        vec![
            mk("backlog", "Backlog", 0, &[Backlog], "neutral"),
            mk("ready", "Ready", 1, &[Ready], "green"),
            mk("working", "Working", 2, &[Working], "green"),
            mk("my_turn", "My turn", 3, &[MyTurn, Unknown], "slate"),
            mk("decision", "Decision needed", 4, &[NeedsDecision], "amber"),
            mk("approval", "Approval needed", 5, &[NeedsApproval], "rust"),
            mk(
                "waiting",
                "Waiting on others",
                6,
                &[WaitingOnOthers],
                "slate",
            ),
            mk("validate", "Validate", 7, &[Validate], "green"),
            mk("done", "Done", 8, &[Done], "neutral"),
        ]
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CardKind {
    Session,
    Task,
}

/// A card is the board's unit. Session cards are created automatically.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Card {
    pub id: String,
    pub kind: CardKind,
    pub title: Option<String>,
    pub session_id: Option<String>,
    pub project_cwd: Option<String>,
    pub priority: i32,
    pub auto_run: bool,
    pub run_prompt: Option<String>,
    pub permission_mode: Option<String>,
    /// Model alias or full name for runs of this card, for example `fable`.
    /// Empty means the Claude Code default.
    pub model: Option<String>,
    pub estimate_weighted_tokens: Option<f64>,
    /// Column chosen by a manual move. Holds until a stronger signal arrives.
    pub manual_column: Option<String>,
    /// Priority of the derived state at the time of the manual move.
    pub manual_lock_priority: Option<u8>,
    pub tags: Vec<String>,
    pub notes: Option<String>,
    pub archived: bool,
    pub bg_job_id: Option<String>,
    /// The last state kari saw for `bg_job_id`. The job list forgets old jobs.
    pub last_job_state: Option<String>,
    pub last_job_at: Option<DateTime<Utc>>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub done_at: Option<DateTime<Utc>>,
}

/// One pending tool call at the tail of a transcript.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingTool {
    pub id: String,
    pub name: String,
    /// For AskUserQuestion: the questions and their options, flattened.
    pub questions: Vec<PendingQuestion>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingQuestion {
    pub question: String,
    pub options: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TokenTotals {
    pub input: u64,
    pub output: u64,
    pub cache_read: u64,
    pub cache_write: u64,
    pub messages: u64,
}

impl TokenTotals {
    /// Cost-shaped weight. Output and cache writes are dear, cache reads are cheap.
    pub fn weighted(&self) -> f64 {
        self.input as f64
            + self.cache_write as f64 * 1.25
            + self.cache_read as f64 * 0.1
            + self.output as f64 * 5.0
    }
}

/// Facts read from one transcript file, updated incrementally.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct SessionFacts {
    pub session_id: String,
    pub transcript_path: String,
    pub cwd: Option<String>,
    pub ai_title: Option<String>,
    pub custom_title: Option<String>,
    pub first_prompt: Option<String>,
    pub last_prompt: Option<String>,
    pub last_assistant_text: Option<String>,
    pub first_at: Option<DateTime<Utc>>,
    pub last_at: Option<DateTime<Utc>>,
    pub last_user_at: Option<DateTime<Utc>>,
    pub last_assistant_at: Option<DateTime<Utc>>,
    pub turns: u32,
    pub tokens: TokenTotals,
    pub models: BTreeSet<String>,
    pub git_branch: Option<String>,
    pub version: Option<String>,
    pub pr_links: Vec<String>,
    pub pending_tools: Vec<PendingTool>,
    /// True when the last record closes a turn (turn_duration) and no tool waits.
    pub turn_closed: bool,
    pub permission_mode: Option<String>,
    pub file_mtime: Option<DateTime<Utc>>,
    pub bytes_parsed: u64,
}

impl SessionFacts {
    pub fn title(&self) -> Option<String> {
        self.custom_title
            .clone()
            .or_else(|| self.ai_title.clone())
            .or_else(|| self.first_prompt.as_ref().map(|p| truncate(p, 80)))
    }
}

pub fn truncate(s: &str, n: usize) -> String {
    let s = s.trim().replace('\n', " ");
    if s.chars().count() <= n {
        s
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// A live process from `~/.claude/sessions/<pid>.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LiveSession {
    pub pid: u32,
    pub session_id: String,
    pub cwd: String,
    pub name: Option<String>,
    pub name_source: Option<String>,
    /// idle | busy | shell | ... as written by Claude Code.
    pub status: Option<String>,
    pub kind: Option<String>,
    pub started_at: Option<DateTime<Utc>>,
    pub status_updated_at: Option<DateTime<Utc>>,
    pub alive: bool,
}

/// A background job from `claude agents --json --all`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BgJob {
    pub id: Option<String>,
    pub session_id: Option<String>,
    pub cwd: Option<String>,
    pub kind: Option<String>,
    /// working | blocked | done | failed | stopped
    pub state: Option<String>,
    pub status: Option<String>,
    pub waiting_for: Option<String>,
    pub name: Option<String>,
    pub pid: Option<u32>,
    pub started_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HerdrAgent {
    pub pane_id: String,
    pub tab_id: Option<String>,
    pub workspace_id: Option<String>,
    pub workspace_label: Option<String>,
    pub cwd: Option<String>,
    pub agent: Option<String>,
    /// idle | working | blocked | done | unknown
    pub agent_status: Option<String>,
    pub title: Option<String>,
    pub focused: bool,
    pub session_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaWindow {
    pub used_percentage: f64,
    pub resets_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaSample {
    pub at: DateTime<Utc>,
    pub five_hour: Option<QuotaWindow>,
    pub seven_day: Option<QuotaWindow>,
    pub source: String,
}

/// One line of the run log of a background job.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobLogEntry {
    pub at: DateTime<Utc>,
    pub job_id: String,
    pub card_id: Option<String>,
    pub state: Option<String>,
    pub detail: Option<String>,
}

/// Weighted token growth of one session between two scans.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenDelta {
    pub at: DateTime<Utc>,
    pub session_id: String,
    pub weighted: f64,
}

/// Percent of the 5-hour window per million weighted tokens.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Calibration {
    pub pct_per_mtok: f64,
    /// 25th and 75th percentile of the learned pairs.
    pub low: f64,
    pub high: f64,
    pub samples: u32,
    /// `learned` or `prior`.
    pub source: String,
    pub updated_at: DateTime<Utc>,
}

impl Default for Calibration {
    fn default() -> Self {
        Calibration {
            pct_per_mtok: crate::estimate::PRIOR_PCT_PER_MTOK,
            low: crate::estimate::PRIOR_PCT_PER_MTOK * 0.4,
            high: crate::estimate::PRIOR_PCT_PER_MTOK * 2.0,
            samples: 0,
            source: "prior".into(),
            updated_at: Utc::now(),
        }
    }
}

/// What one run of a card is expected to cost.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Estimate {
    pub weighted_tokens: f64,
    pub low: f64,
    pub high: f64,
    /// Percent of the 5-hour window, from the calibration factor.
    pub pct_five_hour: f64,
    pub pct_low: f64,
    pub pct_high: f64,
    /// `manual`, `project`, `global` or `default`.
    pub source: String,
    /// How many past sessions the median came from.
    pub sessions: u32,
}

/// Why kari proposed a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProposalTrigger {
    /// The 7-day window resets soon and holds a lot of unused quota.
    WeeklyReset,
    /// The 5-hour window is nearly free and nobody is working.
    IdleFiveHour,
    /// The user pressed "Fill the quota".
    Manual,
}

impl ProposalTrigger {
    pub fn key(self) -> &'static str {
        match self {
            ProposalTrigger::WeeklyReset => "weekly_reset",
            ProposalTrigger::IdleFiveHour => "idle_five_hour",
            ProposalTrigger::Manual => "manual",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProposalItem {
    pub card_id: String,
    pub title: String,
    pub project_name: Option<String>,
    pub prompt: Option<String>,
    pub model: Option<String>,
    pub estimate: Estimate,
    /// Filled when the item started.
    pub job_id: Option<String>,
    pub error: Option<String>,
}

/// A plan kari offers: these tasks, this much quota, this reason.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Proposal {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub trigger: ProposalTrigger,
    pub reason: String,
    pub items: Vec<ProposalItem>,
    /// Percent of the 5-hour window the plan may spend.
    pub budget_pct: f64,
    pub used_pct_before: f64,
    pub total_pct: f64,
    pub used_pct_after: f64,
    /// Cards that were eligible but did not fit.
    pub skipped: u32,
    pub expires_at: DateTime<Utc>,
    /// open | accepted | snoozed | dismissed | expired
    pub state: String,
    /// True when autopilot started it without a click.
    pub auto: bool,
    pub accepted_at: Option<DateTime<Utc>>,
}

/// What the UI renders for one card.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardView {
    pub card: Card,
    pub title: String,
    pub state: DerivedState,
    pub column_id: String,
    pub locked: bool,
    pub project_name: Option<String>,
    pub session: Option<SessionFacts>,
    pub live: Option<LiveSession>,
    pub bg_job: Option<BgJob>,
    pub herdr: Option<HerdrAgent>,
    pub summary: Option<Summary>,
    pub hooks: Option<HookState>,
    pub estimate: Option<Estimate>,
    pub last_activity_at: Option<DateTime<Utc>>,
    pub reason: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BoardView {
    pub columns: Vec<Column>,
    pub cards: Vec<CardView>,
    pub quota: Option<QuotaSample>,
    pub generated_at: DateTime<Utc>,
    pub scanning: bool,
    pub herdr_connected: bool,
    pub hooks_installed: bool,
    pub hooks_port: u16,
    pub calibration: Calibration,
    /// The open proposal, or the last accepted one while its jobs run.
    pub proposal: Option<Proposal>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Sessions older than this many days (by transcript mtime) are ignored.
    pub history_days: i64,
    /// A session with no live process and no activity for this many days is done.
    pub done_after_days: i64,
    /// A session with no activity for this many days is stale and hidden.
    pub stale_after_days: i64,
    pub terminal_app: String,
    pub default_permission_mode: String,
    /// Model for runs of a card that names none. Empty means the Claude Code default.
    pub default_run_model: String,
    pub max_parallel_bg: u32,
    /// Ask Haiku for session summaries after a turn ends.
    pub summaries_enabled: bool,
    /// Hard cap on summary calls per hour.
    pub summaries_per_hour: u32,
    /// Model alias passed to `claude -p --model`.
    pub summary_model: String,
    /// Only sessions active within this many hours get a summary.
    pub summary_recent_hours: i64,
    /// Local port for the Claude Code hook receiver.
    pub hooks_port: u16,
    /// Ask the OAuth usage endpoint when the status line sample is stale.
    /// Off by default: the endpoint is undocumented and needs the login token.
    pub usage_endpoint_enabled: bool,
    /// Offer plans that fill quota that would otherwise expire.
    pub proposals_enabled: bool,
    /// Trigger 1: the 7-day window holds more unused percent than this.
    pub weekly_unused_pct: f64,
    /// Trigger 1: and it resets within this many hours.
    pub weekly_hours_before_reset: i64,
    /// Trigger 2: the 5-hour window is below this percent.
    pub five_hour_idle_pct: f64,
    /// Trigger 2: and no interactive session was active for this many minutes.
    pub idle_minutes: i64,
    /// Local hours that count as working time. Start 8, end 20 means 08:00 to 20:00.
    pub working_hours_start: u32,
    pub working_hours_end: u32,
    /// Percent of the 5-hour window kept free for interactive work in working hours.
    pub working_hours_reserve_pct: f64,
    /// The planner never fills a window past this percent.
    pub fill_ceiling_pct: f64,
    /// Start a weekly-reset plan without a click. Off by default.
    pub autopilot: bool,
    /// Jobs autopilot may start at once.
    pub autopilot_max_jobs: u32,
    /// Open new sessions in a herdr pane when herdr runs. Falls back to the terminal.
    pub prefer_herdr: bool,
    /// Warn when the weekly window resets within a day with this much unused.
    pub weekly_warn_unused_pct: f64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            history_days: 30,
            done_after_days: 3,
            stale_after_days: 14,
            terminal_app: "iTerm".into(),
            default_permission_mode: "auto".into(),
            default_run_model: String::new(),
            max_parallel_bg: 2,
            summaries_enabled: true,
            summaries_per_hour: 6,
            summary_model: "haiku".into(),
            summary_recent_hours: 48,
            hooks_port: 47311,
            usage_endpoint_enabled: false,
            proposals_enabled: true,
            weekly_unused_pct: 40.0,
            weekly_hours_before_reset: 36,
            five_hour_idle_pct: 30.0,
            idle_minutes: 45,
            working_hours_start: 8,
            working_hours_end: 20,
            working_hours_reserve_pct: 30.0,
            fill_ceiling_pct: 85.0,
            autopilot: false,
            autopilot_max_jobs: 1,
            prefer_herdr: true,
            weekly_warn_unused_pct: 25.0,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewTask {
    pub title: String,
    pub project_cwd: Option<String>,
    pub run_prompt: Option<String>,
    pub auto_run: bool,
    pub priority: i32,
    pub notes: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CardPatch {
    pub title: Option<String>,
    pub model: Option<String>,
    pub priority: Option<i32>,
    pub auto_run: Option<bool>,
    pub run_prompt: Option<String>,
    pub permission_mode: Option<String>,
    pub notes: Option<String>,
    pub tags: Option<Vec<String>>,
    pub archived: Option<bool>,
    pub estimate_weighted_tokens: Option<f64>,
}

/// One hook call from Claude Code, reduced to the fields kari uses.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookEvent {
    pub at: DateTime<Utc>,
    pub session_id: String,
    pub event: String,
    pub cwd: Option<String>,
    pub transcript_path: Option<String>,
    pub notification_type: Option<String>,
    pub message: Option<String>,
    pub tool_name: Option<String>,
    pub permission_mode: Option<String>,
}

/// Per-session state folded from hook events. Lives in memory only.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct HookState {
    pub last_event: Option<String>,
    pub last_at: Option<DateTime<Utc>>,
    pub started_at: Option<DateTime<Utc>>,
    pub ended_at: Option<DateTime<Utc>>,
    /// Set by a `permission_prompt` notification, cleared by the next tool run or prompt.
    pub permission_pending_since: Option<DateTime<Utc>>,
    pub permission_message: Option<String>,
    /// Set by `idle_prompt` or `Stop`, cleared by the next prompt.
    pub idle_since: Option<DateTime<Utc>>,
    /// Set by `UserPromptSubmit`, cleared by `Stop`.
    pub turn_active: bool,
    pub events_seen: u32,
}

/// A narrative for one session, from Haiku or from heuristics.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Summary {
    pub session_id: String,
    pub narrative: String,
    pub open_questions: Vec<String>,
    pub next_step: Option<String>,
    pub judged_state: DerivedState,
    pub confidence: f64,
    pub generated_at: DateTime<Utc>,
    /// `haiku` or `heuristic`.
    pub source: String,
    /// The session's last activity time when the summary was made.
    pub based_on_at: Option<DateTime<Utc>>,
    pub model: Option<String>,
}
