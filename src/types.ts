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

/** One sentence per state: what the signal means and what the user does about it. */
export const STATE_HELP: Record<DerivedState, string> = {
  backlog: "A task without a session. Nothing runs. Mark it \"May run unattended\" to make it eligible for a plan.",
  ready: "A task that may run unattended. The planner picks from here when quota is left over.",
  working: "Claude is busy on this session right now, in a terminal or as a background job.",
  my_turn: "The session is alive and idle. Claude answered, and the next prompt is yours.",
  needs_decision: "Claude asked a question with options and waits for your answer.",
  needs_approval: "Claude waits for a permission, a plan approval, or a dialog. Nothing moves until you approve.",
  waiting_on_others: "Someone else must act: a review, a reply, a deploy.",
  validate: "The work looks finished but is not verified: a PR is open, or a background job finished.",
  done: "Finished. The PR merged, you marked it done, or the session went quiet.",
  stale: "No process and no activity for a long time, not judged done.",
  unknown: "kari could not derive a state from the signals it has.",
};

/** How much automatic behaviour a node allows. */
export type AutomationMode = "off" | "ask" | "auto";

export const AUTOMATION_MODES: { value: AutomationMode; label: string; help: string }[] = [
  { value: "off", label: "Off", help: "No plans and no starts. The quota is yours." },
  { value: "ask", label: "Ask", help: "kari offers a plan. You press Start." },
  { value: "auto", label: "Auto", help: "A weekly-reset plan starts by itself." },
];

export interface QueueStep {
  card_id: string;
  title: string;
  project_name: string | null;
  model: string | null;
  estimate: Estimate;
  /** Percent of the 5-hour window in use after this step. */
  window_after_pct: number;
  fits: boolean;
  starts_at: string | null;
  reason: string;
}
export interface QueuePlan {
  steps: QueueStep[];
  budget_pct: number;
  used_pct: number;
  next_check_at: string;
  next_trigger_at: string | null;
  next_trigger: ProposalTrigger | null;
  mode: AutomationMode;
  /** Why nothing would run at all. Null means the queue can run. */
  blocked: string | null;
  open_proposal: boolean;
}

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
/** A permission prompt a node holds open for a remote answer (Away mode). */
export interface PendingPermission {
  id: string;
  session_id: string;
  tool_name: string;
  tool_input: unknown;
  message: string | null;
  since: string;
  until: string;
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
  permission?: PendingPermission | null;
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
  queue?: QueuePlan | null;
  automation_mode?: AutomationMode;
}
export interface Settings {
  node_name: string;
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
  away_mode: boolean;
  away_hold_secs: number;
  listen_on: string;
}
/** A project directory a node knows, with the name the board shows for it. */
export interface Project {
  cwd: string;
  name: string;
}

export interface NewTask {
  title: string;
  project_cwd: string | null;
  run_prompt: string | null;
  auto_run: boolean;
  priority: number;
  notes: string | null;
  model: string | null;
  /** Column the card must land in. A column that no new task derives gets a manual lock. */
  column_id?: string | null;
}
export interface CardPatch {
  title?: string | null;
  /** The project directory. An empty string clears it. The node refuses a path
   *  that is not a directory on that node. */
  project_cwd?: string | null;
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

/** Who may push columns to a node. */
export interface Lease {
  hub_id: string;
  hub_name: string;
  claimed_at: string;
  renewed_at: string;
}
/** One machine on the board: this machine ("local") or a remote kari node over SSH or a private address. */
export interface NodeStatus {
  id: string;
  name: string;
  kind: "local" | "remote";
  online: boolean;
  enabled: boolean;
  paired: boolean;
  ssh_host: string | null;
  address: string | null;
  remote_port: number;
  version: string | null;
  api_version: number | null;
  remote_node_id: string | null;
  last_seen: string | null;
  error: string | null;
  lease: Lease | null;
  /** True when this hub holds the lease on the node. */
  primary: boolean;
  /** True when the node holds permission prompts for a remote answer. */
  away_mode: boolean;
  /** Addresses the node answers on, best first. A pairing code carries them. */
  addresses: string[];
  /** How much automatic behaviour the node allows. Empty from an older node. */
  automation_mode: AutomationMode | "";
}
export interface HubCard extends CardView {
  node_id: string;
  node_name: string;
}
export interface NodeQuota {
  node_id: string;
  node_name: string;
  quota: QuotaSample | null;
  calibration: Calibration;
}
export interface NodeQueue {
  node_id: string;
  node_name: string;
  queue: QueuePlan;
}
export interface NodeProposal {
  node_id: string;
  node_name: string;
  proposal: Proposal;
}
/** Every node on one board. `get_board` returns this. */
export interface HubBoard {
  columns: Column[];
  hub_id: string;
  hub_name: string;
  /** True when this hub is the one that pushes columns. */
  primary: boolean;
  nodes: NodeStatus[];
  cards: HubCard[];
  quotas: NodeQuota[];
  queues: NodeQueue[];
  proposals: NodeProposal[];
  generated_at: string;
  scanning: boolean;
  herdr_connected: boolean;
  hooks_installed: boolean;
  hooks_port: number;
}
export interface LocalAddress {
  interface: string;
  ip: string;
  private: boolean;
}
export interface NewNode {
  name: string;
  ssh_host: string | null;
  /** host:port on a private network, when there is no SSH forward. */
  address?: string | null;
  /** Every address to try, from a pairing code. */
  addresses?: string[];
  remote_port: number;
  /** The node's token, when known already, for example from a pairing code. */
  token?: string | null;
}
export interface NodePatch {
  name?: string;
  addresses?: string[];
  ssh_host?: string | null;
  address?: string | null;
  remote_port?: number;
  enabled?: boolean;
}
