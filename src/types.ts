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
  /** True when the user set the time. Such a step ignores the budget. */
  scheduled?: boolean;
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
  /** A run the user booked for a time. Null means that no run waits. */
  scheduled: ScheduledRun | null;
  created_at: string;
  updated_at: string;
  done_at: string | null;
}

/** A run that waits for a time instead of a trigger. */
export interface ScheduledRun {
  at: string;
  /** A one-off prompt for this run. Null sends the prompt of the card. */
  prompt: string | null;
  /** What the user picked, in words: "when the 5-hour window resets". */
  reason: string;
  created_at: string;
}

/** The cycle a caller names. The node turns it into a time of its own. */
export type ScheduleWhen = "next_reset" | "following_cycle" | "weekly_reset" | "at";

/** A file attached to a card. The name is the id: one card holds one file of
 *  a name. The path is on the node, and the run prompt names it. */
export interface Attachment {
  name: string;
  bytes: number;
  mime: string;
  path: string;
  at: string;
}

/** One file on its way to a card. Base64, because every hop carries JSON. */
export interface NewAttachment {
  name: string;
  data_b64: string;
}

export interface AttachmentData {
  name: string;
  mime: string;
  data_b64: string;
}

/** The largest file one card takes. Matches `MAX_ATTACHMENT_BYTES` in the
 *  core: the ceiling is the link frame that carries a file to a node. */
export const MAX_ATTACHMENT_BYTES = 4 * 1024 * 1024;

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
  /** The job's own one-line account of where it stands. */
  detail?: string | null;
  /** What a blocked job waits for, in its own words. */
  needs?: string | null;
  /** The answer the job proposes. One click sends it. */
  suggested_reply?: string | null;
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
  /** The last release-shaped command kari refused in an autopilot run. */
  blocked_command: string | null;
  blocked_kind: string | null;
  blocked_at: string | null;
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
  /** How far the work travelled: committed, pushed, PR open, merged, released,
   *  deployed, CI passed or failed. Null when the transcript does not say. */
  delivery?: string | null;
}
/** One turn of a transcript, as the conversation view shows it. */
export interface TranscriptMessage {
  /** user, assistant, or peer for a prompt another process sent in. */
  role: string;
  text: string;
  at: string | null;
}
/** The conversation of one session: the last `messages.length` turns of `total`. */
export interface Conversation {
  session_id: string;
  total: number;
  messages: TranscriptMessage[];
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
  /** False when the planner left the card out. The user can still pick it. */
  fits: boolean;
  /** "budget" or "slots" when the card did not fit. */
  skip_reason: string | null;
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
  /** Files attached to the card, on the node that owns it. */
  attachments?: Attachment[];
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
  close_herdr_tab_on_done: boolean;
  weekly_warn_unused_pct: number;
  away_mode: boolean;
  away_hold_secs: number;
  /** Days a done card keeps its attachments. An archive clears them at once. */
  attachment_keep_days: number;
  listen_on: string;
  /** Install a new kari without asking. Desktop only; the node has a flag. */
  auto_update: boolean;
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
  /** Card writes the hub holds for this node until it answers again. */
  pending_writes?: number;
  /** Why the queue is not moving: what the node said about the write at the
   *  head of it. */
  pending_error?: string | null;
}
export interface HubCard extends CardView {
  node_id: string;
  node_name: string;
  /** True while the hub still holds a write for this card, because the node
   *  that owns it is away. The card is on the board, and the node has not
   *  seen it yet. */
  pending?: boolean;
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
/** The Claude Code account a node is signed in to. */
export interface AccountIdentity {
  id: string;
  email: string | null;
  display_name: string | null;
  organization_id: string | null;
}
/** One budget and every node spending it. The 5-hour and 7-day windows belong
 *  to a Claude Code account, so two machines on one login share a row. */
export interface AccountQuota {
  /** The account id, or `node:<id>` for a node whose account is unknown. */
  key: string;
  /** Alias, else the name on the account, else the login, else the node name. */
  label: string;
  alias: string | null;
  account: AccountIdentity | null;
  node_ids: string[];
  node_names: string[];
  quota: QuotaSample | null;
  calibration: Calibration | null;
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
  /** The same quota, one row per account. What the header shows. */
  accounts: AccountQuota[];
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
