export type DerivedState =
  | "backlog"
  | "ready"
  | "working"
  | "my_turn"
  | "needs_decision"
  | "needs_approval"
  | "waiting_on_others"
  | "validate"
  | "done"
  | "stale"
  | "unknown";

export const ALL_STATES: DerivedState[] = [
  "backlog",
  "ready",
  "working",
  "my_turn",
  "needs_decision",
  "needs_approval",
  "waiting_on_others",
  "validate",
  "done",
  "stale",
  "unknown",
];

// Models kari offers for a run. The empty value means the Claude Code default.
export const RUN_MODELS: { value: string; label: string }[] = [
  { value: "", label: "Default" },
  { value: "fable", label: "Fable (deep reviews)" },
  { value: "opus", label: "Opus" },
  { value: "sonnet", label: "Sonnet" },
  { value: "haiku", label: "Haiku" },
];

export const STATE_LABEL: Record<DerivedState, string> = {
  backlog: "Backlog",
  ready: "Ready",
  working: "Working",
  my_turn: "My turn",
  needs_decision: "Decision",
  needs_approval: "Approval",
  waiting_on_others: "Waiting",
  validate: "Validate",
  done: "Done",
  stale: "Stale",
  unknown: "Unknown",
};

export interface Column {
  id: string;
  name: string;
  order: number;
  accepts: DerivedState[];
  wip_limit: number | null;
  color: string | null;
  hidden: boolean;
}

export interface JobLogEntry {
  at: string;
  job_id: string;
  card_id: string | null;
  state: string | null;
  detail: string | null;
}

export interface Card {
  id: string;
  kind: "session" | "task";
  title: string | null;
  session_id: string | null;
  project_cwd: string | null;
  priority: number;
  auto_run: boolean;
  run_prompt: string | null;
  permission_mode: string | null;
  model: string | null;
  estimate_weighted_tokens: number | null;
  manual_column: string | null;
  manual_lock_priority: number | null;
  tags: string[];
  notes: string | null;
  archived: boolean;
  bg_job_id: string | null;
  last_job_state: string | null;
  last_job_at: string | null;
  created_at: string;
  updated_at: string;
  done_at: string | null;
}

export interface PendingQuestion {
  question: string;
  options: string[];
}
export interface PendingTool {
  id: string;
  name: string;
  questions: PendingQuestion[];
}
export interface TokenTotals {
  input: number;
  output: number;
  cache_read: number;
  cache_write: number;
  messages: number;
}
export interface SessionFacts {
  session_id: string;
  transcript_path: string;
  cwd: string | null;
  ai_title: string | null;
  custom_title: string | null;
  first_prompt: string | null;
  last_prompt: string | null;
  last_assistant_text: string | null;
  first_at: string | null;
  last_at: string | null;
  last_user_at: string | null;
  last_assistant_at: string | null;
  turns: number;
  tokens: TokenTotals;
  models: string[];
  git_branch: string | null;
  version: string | null;
  pr_links: string[];
  pending_tools: PendingTool[];
  turn_closed: boolean;
  permission_mode: string | null;
  file_mtime: string | null;
  bytes_parsed: number;
}
export interface LiveSession {
  pid: number;
  session_id: string;
  cwd: string;
  name: string | null;
  name_source: string | null;
  status: string | null;
  kind: string | null;
  started_at: string | null;
  status_updated_at: string | null;
  alive: boolean;
}
export interface BgJob {
  id: string | null;
  session_id: string | null;
  cwd: string | null;
  kind: string | null;
  state: string | null;
  status: string | null;
  waiting_for: string | null;
  name: string | null;
  pid: number | null;
  started_at: string | null;
}
export interface HerdrAgent {
  pane_id: string;
  tab_id: string | null;
  workspace_id: string | null;
  workspace_label: string | null;
  cwd: string | null;
  agent: string | null;
  agent_status: string | null;
  title: string | null;
  focused: boolean;
  session_id: string | null;
}
export interface QuotaWindow {
  used_percentage: number;
  resets_at: string | null;
}
export interface QuotaSample {
  at: string;
  five_hour: QuotaWindow | null;
  seven_day: QuotaWindow | null;
  source: string;
}
export interface Calibration {
  pct_per_mtok: number;
  low: number;
  high: number;
  samples: number;
  source: string;
  updated_at: string;
}
export interface Estimate {
  weighted_tokens: number;
  low: number;
  high: number;
  pct_five_hour: number;
  pct_low: number;
  pct_high: number;
  source: string;
  sessions: number;
}
export interface HookState {
  last_event: string | null;
  last_at: string | null;
  started_at: string | null;
  ended_at: string | null;
  permission_pending_since: string | null;
  permission_message: string | null;
  idle_since: string | null;
  turn_active: boolean;
  events_seen: number;
}
export interface Summary {
  session_id: string;
  narrative: string;
  open_questions: string[];
  next_step: string | null;
  judged_state: DerivedState;
  confidence: number;
  generated_at: string;
  source: string;
  based_on_at: string | null;
  model: string | null;
}
export type ProposalTrigger = "weekly_reset" | "idle_five_hour" | "manual";
export interface ProposalItem {
  card_id: string;
  title: string;
  project_name: string | null;
  prompt: string | null;
  model: string | null;
  estimate: Estimate;
  job_id: string | null;
  error: string | null;
}
export interface Proposal {
  id: string;
  created_at: string;
  trigger: ProposalTrigger;
  reason: string;
  items: ProposalItem[];
  budget_pct: number;
  used_pct_before: number;
  total_pct: number;
  used_pct_after: number;
  skipped: number;
  expires_at: string;
  state: string;
  auto: boolean;
  accepted_at: string | null;
}
export interface CardView {
  card: Card;
  title: string;
  state: DerivedState;
  column_id: string;
  locked: boolean;
  project_name: string | null;
  session: SessionFacts | null;
  live: LiveSession | null;
  bg_job: BgJob | null;
  herdr: HerdrAgent | null;
  summary: Summary | null;
  hooks: HookState | null;
  estimate: Estimate | null;
  last_activity_at: string | null;
  reason: string;
}
export interface BoardView {
  columns: Column[];
  cards: CardView[];
  quota: QuotaSample | null;
  generated_at: string;
  scanning: boolean;
  herdr_connected: boolean;
  hooks_installed: boolean;
  hooks_port: number;
  calibration: Calibration;
  proposal: Proposal | null;
}
export interface Settings {
  history_days: number;
  done_after_days: number;
  stale_after_days: number;
  terminal_app: string;
  default_permission_mode: string;
  default_run_model: string;
  max_parallel_bg: number;
  summaries_enabled: boolean;
  summaries_per_hour: number;
  summary_model: string;
  summary_recent_hours: number;
  hooks_port: number;
  usage_endpoint_enabled: boolean;
  proposals_enabled: boolean;
  weekly_unused_pct: number;
  weekly_hours_before_reset: number;
  five_hour_idle_pct: number;
  idle_minutes: number;
  working_hours_start: number;
  working_hours_end: number;
  working_hours_reserve_pct: number;
  fill_ceiling_pct: number;
  autopilot: boolean;
  autopilot_max_jobs: number;
  prefer_herdr: boolean;
  weekly_warn_unused_pct: number;
}
export interface NewTask {
  title: string;
  project_cwd: string | null;
  run_prompt: string | null;
  auto_run: boolean;
  priority: number;
  notes: string | null;
  model: string | null;
}
export interface CardPatch {
  title?: string | null;
  model?: string | null;
  priority?: number | null;
  auto_run?: boolean | null;
  run_prompt?: string | null;
  permission_mode?: string | null;
  notes?: string | null;
  tags?: string[] | null;
  archived?: boolean | null;
  estimate_weighted_tokens?: number | null;
}
