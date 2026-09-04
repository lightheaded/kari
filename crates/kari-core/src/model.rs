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
            // Three states that all wait for the user share one column. The
            // board groups them inside it, most urgent first.
            mk(
                "needs_me",
                "Needs me",
                3,
                &[NeedsApproval, NeedsDecision, MyTurn, Unknown],
                "amber",
            ),
            mk("review", "Review", 4, &[Validate, WaitingOnOthers], "slate"),
            mk("done", "Done", 5, &[Done], "neutral"),
        ]
    }

    /// The nine columns kari shipped up to version 0.4.1. A stored layout that
    /// still matches this list is replaced by `defaults` once, because the
    /// board no longer fits nine columns in one window.
    pub fn legacy_defaults() -> Vec<Column> {
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

    /// True when two layouts hold the same columns, in the same order, with the
    /// same names, states, limits and visibility. Ignores nothing.
    pub fn same_layout(a: &[Column], b: &[Column]) -> bool {
        if a.len() != b.len() {
            return false;
        }
        let key = |c: &Column| {
            let mut states: Vec<String> = c.accepts.iter().map(|s| format!("{s:?}")).collect();
            states.sort();
            (
                c.id.clone(),
                c.name.clone(),
                c.wip_limit,
                c.hidden,
                states.join(","),
            )
        };
        let mut ka: Vec<_> = a.iter().map(key).collect();
        let mut kb: Vec<_> = b.iter().map(key).collect();
        ka.sort();
        kb.sort();
        ka == kb
    }

    /// Where a manual lock on a dropped column goes. The nine-column layout had
    /// five columns the six-column layout does not.
    pub fn migrated_column_id(old: &str) -> Option<&'static str> {
        match old {
            "my_turn" | "decision" | "approval" => Some("needs_me"),
            "waiting" | "validate" => Some("review"),
            _ => None,
        }
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
    /// A permission prompt kari holds for a remote answer. Away mode only.
    #[serde(default)]
    pub permission: Option<PendingPermission>,
}

/// A permission prompt that Claude Code asked and kari holds open, so that a
/// phone can answer it. Lives in memory until answered or timed out.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PendingPermission {
    pub id: String,
    pub session_id: String,
    pub tool_name: String,
    /// The tool's input as Claude Code sent it. Can be large.
    pub tool_input: serde_json::Value,
    pub message: Option<String>,
    pub since: DateTime<Utc>,
    /// When the hold ends and the terminal dialog appears instead.
    pub until: DateTime<Utc>,
}

/// The answer to a held permission prompt: `allow` or `deny`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionAnswer {
    pub behavior: String,
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
    /// True when this node holds permission prompts for a remote answer.
    #[serde(default)]
    pub away_mode: bool,
    /// What the planner would do next on this node, and when. Read only.
    #[serde(default)]
    pub queue: Option<QueuePlan>,
    /// `off`, `ask` or `auto`. The board shows it on the automation switch.
    #[serde(default)]
    pub automation_mode: String,
}

/// How much of the automatic behaviour is on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AutomationMode {
    /// No plans, no starts. The quota belongs to the user.
    Off,
    /// kari offers a plan. The user presses Start.
    Ask,
    /// A weekly-reset plan starts without a click.
    Auto,
}

impl AutomationMode {
    pub fn key(self) -> &'static str {
        match self {
            AutomationMode::Off => "off",
            AutomationMode::Ask => "ask",
            AutomationMode::Auto => "auto",
        }
    }

    pub fn parse(s: &str) -> Option<AutomationMode> {
        match s {
            "off" => Some(AutomationMode::Off),
            "ask" => Some(AutomationMode::Ask),
            "auto" => Some(AutomationMode::Auto),
            _ => None,
        }
    }
}

/// One step the planner would take: which card, what it costs, and when.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueueStep {
    pub card_id: String,
    pub title: String,
    pub project_name: Option<String>,
    pub model: Option<String>,
    pub estimate: Estimate,
    /// Percent of the 5-hour window in use after this step, this step included.
    pub window_after_pct: f64,
    /// True when the step is inside the budget the planner may spend.
    pub fits: bool,
    /// When the step starts, as far as the windows allow a guess. None means
    /// that nothing schedules it: read `reason`.
    pub starts_at: Option<DateTime<Utc>>,
    /// One phrase for the user: `now`, `does not fit the budget`, and so on.
    pub reason: String,
}

/// The dry run of the planner. Built on demand, never stored.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QueuePlan {
    pub steps: Vec<QueueStep>,
    /// Percent of the 5-hour window the planner may spend right now.
    pub budget_pct: f64,
    /// Percent of the 5-hour window already in use.
    pub used_pct: f64,
    /// When the planner looks at the triggers again.
    pub next_check_at: DateTime<Utc>,
    /// When a trigger fires next, as far as the reset times allow a guess.
    pub next_trigger_at: Option<DateTime<Utc>>,
    pub next_trigger: Option<ProposalTrigger>,
    pub mode: AutomationMode,
    /// Why nothing would run at all. None means that the queue can run.
    pub blocked: Option<String>,
    /// Cards that hold a plan open right now.
    pub open_proposal: bool,
}

/// The prompt a run receives. A task card joins its title and its body with a
/// blank line, so the title never has to be repeated in the body. A session
/// card sends the body alone, because its title comes from the transcript.
pub fn compose_prompt(kind: CardKind, title: Option<&str>, body: Option<&str>) -> Option<String> {
    fn clean(v: Option<&str>) -> Option<&str> {
        v.map(str::trim).filter(|v| !v.is_empty())
    }
    let body = clean(body);
    if kind == CardKind::Session {
        return body
            .map(str::to_string)
            .or_else(|| clean(title).map(str::to_string));
    }
    match (clean(title), body) {
        (Some(t), Some(b)) => Some(format!("{t}\n\n{b}")),
        (Some(t), None) => Some(t.to_string()),
        (None, Some(b)) => Some(b.to_string()),
        (None, None) => None,
    }
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
    /// Name of this node as other kari instances see it. Empty means the host name.
    pub node_name: String,
    /// Hold permission prompts of interactive sessions for a remote answer,
    /// such as from a phone. The terminal shows a spinner instead of the
    /// dialog while kari waits, so this is off at the desk.
    pub away_mode: bool,
    /// Seconds a held permission prompt waits before the dialog appears.
    pub away_hold_secs: u64,
    /// Where the API answers besides loopback, for a hub on a phone that
    /// cannot open an SSH forward. Empty means loopback only. An interface
    /// name, such as `utun5`, means the private addresses of that interface,
    /// which is what a VPN needs. `*` means every private address of the
    /// machine. A public address is never bound.
    pub listen_on: String,
    /// The switch this replaced: it bound every private address. Read once so
    /// that a machine set up before the picker keeps answering, then written
    /// back as `*`. Never sent to the UI.
    #[serde(default, skip_serializing)]
    pub listen_private: bool,
}

impl Settings {
    /// The three-state automation mode, read from the two flags that hold it.
    ///
    /// The mode is derived, never stored. A stored field cannot work here: the
    /// whole struct carries `#[serde(default)]`, so a record written before the
    /// field existed comes back holding the struct default, and that is
    /// indistinguishable from a mode the user chose. Deriving it means an older
    /// record keeps saying exactly what it always said.
    pub fn automation(&self) -> AutomationMode {
        if !self.proposals_enabled {
            AutomationMode::Off
        } else if self.autopilot {
            AutomationMode::Auto
        } else {
            AutomationMode::Ask
        }
    }

    /// Set the mode through the two flags.
    ///
    /// `Off` leaves `autopilot` alone, so the flag still says what the user
    /// asked for the last time plans were on. Only `proposals_enabled` gates
    /// the planner, so nothing runs while the mode is `Off` either way.
    pub fn set_automation(&mut self, m: AutomationMode) {
        match m {
            AutomationMode::Off => self.proposals_enabled = false,
            AutomationMode::Ask => {
                self.proposals_enabled = true;
                self.autopilot = false;
            }
            AutomationMode::Auto => {
                self.proposals_enabled = true;
                self.autopilot = true;
            }
        }
    }
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
            node_name: String::new(),
            away_mode: false,
            away_hold_secs: 600,
            listen_on: String::new(),
            listen_private: false,
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
    /// Column the card must land in. Set when the user adds the task from the
    /// foot of a column that no derived state sends a new task to. It becomes a
    /// manual lock, which the first stronger signal breaks.
    #[serde(default)]
    pub column_id: Option<String>,
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

// ---------------------------------------------------------------- nodes

/// The version of the HTTP API a node serves. The hub refuses a different major.
pub const API_VERSION: u32 = 1;

/// What a node says about itself on `/kari/health`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeIdentity {
    pub ok: bool,
    pub app: String,
    pub version: String,
    pub api_version: u32,
    pub node_id: String,
    pub node_name: String,
    pub platform: String,
    /// `ip:port` of every private address the node listens on now. A hub that
    /// cannot open an SSH forward, such as a phone, connects to one of them.
    /// The list is how the desktop learns where a node is, without a typed IP.
    #[serde(default)]
    pub addresses: Vec<String>,
}

/// A remote kari node the desktop app connects to. Stored in the local database.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct NodeRecord {
    pub id: String,
    /// Display name. Empty means the SSH host, else the node's own name.
    pub name: String,
    /// SSH host alias from `~/.ssh/config`. None means a direct connection to
    /// `address`, else to `127.0.0.1:remote_port` for tests.
    pub ssh_host: Option<String>,
    /// `host:port` of the node's API on a private network, for a hub that
    /// cannot open an SSH forward, such as a phone. Wins over `remote_port`
    /// when `ssh_host` is None.
    pub address: Option<String>,
    /// Every address the node answered on or advertised, in the order to try.
    /// `address` holds the one in use; a failed connection walks this list, so
    /// a node that moves to another address is found again without a re-pair.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// Port of the node's API on its own loopback interface.
    pub remote_port: u16,
    pub enabled: bool,
    pub created_at: DateTime<Utc>,
}

impl Default for NodeRecord {
    fn default() -> Self {
        NodeRecord {
            id: String::new(),
            name: String::new(),
            ssh_host: None,
            address: None,
            addresses: Vec::new(),
            remote_port: 47311,
            enabled: true,
            created_at: Utc::now(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct NewNode {
    pub name: String,
    pub ssh_host: Option<String>,
    /// `host:port` for a direct connection over a private network.
    pub address: Option<String>,
    /// Addresses to try, from a pairing code. The hub keeps the one that answers.
    #[serde(default)]
    pub addresses: Vec<String>,
    pub remote_port: u16,
    /// The node's token, when the caller has it already, for example from a
    /// pairing QR code. None means: read it over SSH, or pair later by hand.
    pub token: Option<String>,
}

impl Default for NewNode {
    fn default() -> Self {
        NewNode {
            name: String::new(),
            ssh_host: None,
            address: None,
            addresses: Vec::new(),
            remote_port: 47311,
            token: None,
        }
    }
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NodePatch {
    pub name: Option<String>,
    pub ssh_host: Option<Option<String>>,
    pub address: Option<Option<String>>,
    pub addresses: Option<Vec<String>>,
    pub remote_port: Option<u16>,
    pub enabled: Option<bool>,
}

/// Connection state of one node as the board shows it. The local node is always
/// first, with id `local`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct NodeStatus {
    pub id: String,
    pub name: String,
    pub kind: String,
    pub online: bool,
    pub enabled: bool,
    pub paired: bool,
    pub ssh_host: Option<String>,
    pub address: Option<String>,
    pub remote_port: u16,
    pub version: Option<String>,
    pub api_version: Option<u32>,
    pub remote_node_id: Option<String>,
    pub last_seen: Option<DateTime<Utc>>,
    pub error: Option<String>,
    /// The hub that holds this node's column lease, as the node reports it.
    #[serde(default)]
    pub lease: Option<Lease>,
    /// True when this hub holds the lease on this node.
    #[serde(default)]
    pub primary: bool,
    /// True when the node holds permission prompts for a remote answer.
    #[serde(default)]
    pub away_mode: bool,
    /// Addresses this node advertised or answered on. The UI shows them, and a
    /// pairing code carries them to the phone.
    #[serde(default)]
    pub addresses: Vec<String>,
    /// How much automatic behaviour the node allows: `off`, `ask` or `auto`.
    #[serde(default)]
    pub automation_mode: String,
}

/// Who may push columns to a node. One row per node, kept in its `kv` table.
/// A lease that was not renewed for `LEASE_TTL_SECS` counts as free.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct Lease {
    pub hub_id: String,
    pub hub_name: String,
    pub claimed_at: DateTime<Utc>,
    pub renewed_at: DateTime<Utc>,
}

/// Seconds without a renewal after which any hub may claim a lease.
pub const LEASE_TTL_SECS: i64 = 600;

impl Lease {
    pub fn expired(&self, now: DateTime<Utc>) -> bool {
        (now - self.renewed_at).num_seconds() > LEASE_TTL_SECS
    }
}

/// A hub asks for a node's lease. `take` wins over a live holder; the user
/// pressed "Make this device primary". Without it, the claim succeeds only
/// when the lease is free, expired, or already this hub's.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct LeaseClaim {
    pub hub_id: String,
    pub hub_name: String,
    pub take: bool,
}

/// A card on the hub board: the node's view plus the node it lives on.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubCard {
    pub node_id: String,
    pub node_name: String,
    #[serde(flatten)]
    pub view: CardView,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeQuota {
    pub node_id: String,
    pub node_name: String,
    pub quota: Option<QuotaSample>,
    pub calibration: Calibration,
}

/// The dry run of one node's planner, as the queue strip shows it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeQueue {
    pub node_id: String,
    pub node_name: String,
    pub queue: QueuePlan,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NodeProposal {
    pub node_id: String,
    pub node_name: String,
    pub proposal: Proposal,
}

/// The merged board of every node. Columns come from the hub's own store; the
/// primary hub pushes them to every node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HubBoard {
    pub columns: Vec<Column>,
    /// This hub, as the nodes know it.
    #[serde(default)]
    pub hub_id: String,
    #[serde(default)]
    pub hub_name: String,
    /// True when this hub is the one that pushes columns.
    #[serde(default)]
    pub primary: bool,
    pub nodes: Vec<NodeStatus>,
    pub cards: Vec<HubCard>,
    pub quotas: Vec<NodeQuota>,
    /// One per node that answered. A node running an older kari sends none.
    #[serde(default)]
    pub queues: Vec<NodeQueue>,
    pub proposals: Vec<NodeProposal>,
    pub generated_at: DateTime<Utc>,
    pub scanning: bool,
    pub herdr_connected: bool,
    pub hooks_installed: bool,
    pub hooks_port: u16,
}

/// What "Jump in" must do. The node computes it, the caller runs it where the
/// user sits: in a local terminal, or over SSH for a remote node.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct JumpPlan {
    /// Directory to start in.
    pub cwd: String,
    /// Shell command to run there. Empty when a herdr pane was focused and no
    /// command is needed.
    pub command: String,
    /// The herdr pane the node focused, when one matched.
    pub herdr_pane: Option<String>,
    /// One line for the user.
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_task_prompt_joins_the_title_and_the_body() {
        let p = compose_prompt(
            CardKind::Task,
            Some("Fix the flaky auth test"),
            Some("It fails one run in five."),
        );
        assert_eq!(
            p.as_deref(),
            Some("Fix the flaky auth test\n\nIt fails one run in five.")
        );
    }

    #[test]
    fn a_task_prompt_falls_back_to_one_part() {
        assert_eq!(
            compose_prompt(CardKind::Task, Some("Only a title"), None).as_deref(),
            Some("Only a title")
        );
        assert_eq!(
            compose_prompt(CardKind::Task, None, Some("Only a body")).as_deref(),
            Some("Only a body")
        );
        assert_eq!(
            compose_prompt(CardKind::Task, Some("  "), Some("   ")),
            None
        );
    }

    #[test]
    fn a_session_prompt_never_carries_the_title() {
        // The title of a session card comes from the transcript, so it is not
        // an instruction and must stay out of a continue prompt.
        let p = compose_prompt(
            CardKind::Session,
            Some("Refactor of the store layer"),
            Some("Continue with the next step."),
        );
        assert_eq!(p.as_deref(), Some("Continue with the next step."));
    }

    #[test]
    fn the_six_columns_accept_every_state_but_stale() {
        let cols = Column::defaults();
        assert_eq!(cols.len(), 6);
        for s in DerivedState::all() {
            if *s == DerivedState::Stale {
                continue;
            }
            assert!(
                cols.iter().any(|c| c.accepts.contains(s)),
                "no column accepts {s:?}"
            );
        }
    }

    #[test]
    fn the_layout_check_sees_a_changed_name_or_state() {
        let nine = Column::legacy_defaults();
        assert!(Column::same_layout(&nine, &Column::legacy_defaults()));
        assert!(!Column::same_layout(&nine, &Column::defaults()));
        let mut renamed = Column::legacy_defaults();
        renamed[0].name = "Later".into();
        assert!(!Column::same_layout(&renamed, &Column::legacy_defaults()));
        let mut limited = Column::legacy_defaults();
        limited[2].wip_limit = Some(3);
        assert!(!Column::same_layout(&limited, &Column::legacy_defaults()));
    }

    #[test]
    fn the_automation_mode_reads_and_writes_the_two_flags() {
        let mut s = Settings::default();
        assert_eq!(s.automation(), AutomationMode::Ask);
        s.set_automation(AutomationMode::Auto);
        assert!(s.proposals_enabled && s.autopilot);
        assert_eq!(s.automation(), AutomationMode::Auto);
        s.set_automation(AutomationMode::Ask);
        assert_eq!(s.automation(), AutomationMode::Ask);
    }

    #[test]
    fn turning_the_mode_off_keeps_what_autopilot_said() {
        // The user had autopilot on. Off must not throw that away, because the
        // mode is derived and there is nowhere else to remember it.
        let mut s = Settings::default();
        s.set_automation(AutomationMode::Auto);
        s.set_automation(AutomationMode::Off);
        assert_eq!(s.automation(), AutomationMode::Off);
        assert!(s.autopilot, "autopilot must survive an Off");
        s.set_automation(AutomationMode::Auto);
        assert_eq!(s.automation(), AutomationMode::Auto);
    }

    #[test]
    fn a_record_from_an_older_kari_keeps_its_meaning() {
        // A settings record written before the mode existed carries the two
        // flags only. Autopilot on has always meant Auto, and must keep doing so.
        let stored = Settings {
            proposals_enabled: true,
            autopilot: true,
            ..Default::default()
        };
        assert_eq!(stored.automation(), AutomationMode::Auto);
        let quiet = Settings {
            proposals_enabled: false,
            autopilot: false,
            ..Default::default()
        };
        assert_eq!(quiet.automation(), AutomationMode::Off);
    }
}
