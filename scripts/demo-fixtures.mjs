#!/usr/bin/env node
// Write the dummy board that the screenshots use.
//
// Usage: node scripts/demo-fixtures.mjs [outdir]     (default: docs/demo)
//
// The output has the shape that the Vite dev server serves at /dev/*.json:
// board.json, settings.json and job-log.json. Every project, session, prompt
// and path here is invented. Do not put real data in this file.
//
// Times are fixed. scripts/screenshots.mjs pins the browser clock to NOW so
// that "3m ago" and "resets in 2h" read the same in every release.

import { mkdirSync, writeFileSync } from "node:fs";
import { join } from "node:path";

export const NOW = "2026-09-03T10:00:00Z";
const now = new Date(NOW).getTime();

const min = (n) => new Date(now - n * 60_000).toISOString();
const hours = (n) => min(n * 60);
const days = (n) => hours(n * 24);
const ahead = (minutes) => new Date(now + minutes * 60_000).toISOString();

const HOME = "/Users/dev";
const LAB_HOME = "/home/dev";
const projects = {
  atlas: { name: "atlas-api", cwd: `${HOME}/src/atlas-api` },
  store: { name: "storefront-web", cwd: `${HOME}/src/storefront-web` },
  docs: { name: "docs-site", cwd: `${HOME}/src/docs-site` },
  mobile: { name: "mobile-app", cwd: `${HOME}/src/mobile-app` },
  infra: { name: "infra", cwd: `${HOME}/src/infra` },
  ranker: { name: "ranker", cwd: `${LAB_HOME}/src/ranker` },
  batch: { name: "batch-jobs", cwd: `${LAB_HOME}/src/batch-jobs` },
};

// The board holds two machines: this one and one remote node over SSH.
const NODES = [
  {
    id: "local",
    name: "studio",
    kind: "local",
    online: true,
    enabled: true,
    paired: true,
    ssh_host: null,
    remote_port: 0,
    version: "0.1.0",
    api_version: 1,
    remote_node_id: null,
    last_seen: min(0),
    error: null,
    lease: null,
    primary: true,
    away_mode: false,
    addresses: [],
    automation_mode: "ask",
  },
  {
    id: "lab",
    name: "lab",
    kind: "remote",
    online: true,
    enabled: true,
    paired: true,
    ssh_host: "lab",
    remote_port: 47311,
    version: "0.1.0",
    api_version: 1,
    remote_node_id: "node_lab",
    last_seen: min(1),
    error: null,
    lease: null,
    primary: true,
    away_mode: false,
    addresses: ["lab:47311"],
    automation_mode: "ask",
  },
];

// The six default columns. Needs me and Review each merge several states, and
// the board groups those states inside the column.
const columns = [
  ["backlog", "Backlog", ["backlog"], null, "neutral"],
  ["ready", "Ready", ["ready"], null, "green"],
  ["working", "Working", ["working"], 3, "green"],
  ["needs_me", "Needs me", ["needs_approval", "needs_decision", "my_turn", "unknown"], null, "amber"],
  ["review", "Review", ["validate", "waiting_on_others"], null, "slate"],
  ["done", "Done", ["done"], null, "neutral"],
].map(([id, name, accepts, wip, color], order) => ({ id, name, order, accepts, wip_limit: wip, color, hidden: false }));

/** The column a state lands in, so a card's column_id matches the new set. */
const COLUMN_OF = Object.fromEntries(columns.flatMap((c) => c.accepts.map((s) => [s, c.id])));

/** Where a column id from the old nine-column set goes now. */
const MERGED = {
  my_turn: "needs_me",
  decision: "needs_me",
  approval: "needs_me",
  waiting: "review",
  validate: "review",
};

/** A card sits in the column its state derives, unless a manual lock holds it. */
function columnOf(o) {
  if (o.locked) return MERGED[o.column_id] ?? o.column_id;
  return COLUMN_OF[o.state] ?? o.column_id;
}

const PCT_PER_MTOK = 1.9;
const calibration = { pct_per_mtok: PCT_PER_MTOK, low: 1.4, high: 2.6, samples: 7, source: "learned", updated_at: hours(3) };

function estimate(weighted, source, sessions) {
  const pct = (w) => (w / 1e6) * PCT_PER_MTOK;
  return {
    weighted_tokens: weighted,
    low: weighted * 0.6,
    high: weighted * 1.7,
    pct_five_hour: pct(weighted),
    pct_low: pct(weighted * 0.6),
    pct_high: pct(weighted * 1.7),
    source,
    sessions,
  };
}

function card(id, kind, project, overrides = {}) {
  return {
    id,
    kind,
    title: null,
    session_id: null,
    project_cwd: project?.cwd ?? null,
    priority: 0,
    auto_run: false,
    run_prompt: null,
    permission_mode: null,
    model: null,
    estimate_weighted_tokens: null,
    manual_column: null,
    manual_lock_priority: null,
    tags: [],
    notes: null,
    archived: false,
    bg_job_id: null,
    last_job_state: null,
    last_job_at: null,
    created_at: days(2),
    updated_at: hours(1),
    done_at: null,
    ...overrides,
  };
}

function session(id, project, o) {
  const tokens = o.tokens ?? { input: 120_000, output: 18_000, cache_read: 2_400_000, cache_write: 180_000, messages: 60 };
  return {
    session_id: id,
    transcript_path: `${HOME}/.claude/projects/-Users-dev-src-${project.name}/${id}.jsonl`,
    cwd: project.cwd,
    ai_title: o.title,
    custom_title: null,
    first_prompt: o.first_prompt ?? o.title,
    last_prompt: o.last_prompt ?? o.first_prompt ?? o.title,
    last_assistant_text: o.last_reply ?? null,
    first_at: o.first_at,
    last_at: o.last_at,
    last_user_at: o.last_user_at ?? o.last_at,
    last_assistant_at: o.last_at,
    turns: o.turns ?? 12,
    tokens,
    models: o.models ?? ["claude-opus-5"],
    git_branch: o.branch ?? "main",
    version: "2.1.4",
    pr_links: o.pr_links ?? [],
    pending_tools: o.pending_tools ?? [],
    turn_closed: o.turn_closed ?? true,
    permission_mode: o.permission_mode ?? "default",
    file_mtime: o.last_at,
    bytes_parsed: 480_000,
  };
}

function live(sessionId, project, status, pid) {
  return {
    pid,
    session_id: sessionId,
    cwd: project.cwd,
    name: null,
    name_source: null,
    status,
    kind: "interactive",
    started_at: hours(2),
    status_updated_at: min(1),
    alive: true,
  };
}

function view(o) {
  return {
    card: o.card,
    title: o.title,
    state: o.state,
    column_id: columnOf(o),
    locked: o.locked ?? false,
    project_name: o.project?.name ?? null,
    session: o.session ?? null,
    live: o.live ?? null,
    bg_job: o.bg_job ?? null,
    herdr: o.herdr ?? null,
    summary: o.summary ?? null,
    hooks: o.hooks ?? null,
    estimate: o.estimate ?? null,
    last_activity_at: o.last_activity_at ?? null,
    reason: o.reason,
  };
}

function summary(sessionId, narrative, judged, confidence, o = {}) {
  return {
    session_id: sessionId,
    narrative,
    open_questions: o.open_questions ?? [],
    next_step: o.next_step ?? null,
    judged_state: judged,
    confidence,
    generated_at: o.at ?? min(9),
    source: "haiku",
    based_on_at: o.at ?? min(9),
    model: "haiku",
  };
}

// Made-up session ids for the dummy board. The names avoid the words a secret
// scanner treats as a credential keyword, because the values are UUID-shaped.
const S = {
  parser: "5b1c9e2a-7d34-4f0e-9a8b-3c2d1e0f4a5b",
  cache: "8e2f4a6c-1b3d-4e5f-a7b9-c1d3e5f7a9b1",
  middleware: "c3d5e7f9-2a4b-4c6d-8e0f-1a3b5c7d9e1f",
  ci: "a1b2c3d4-e5f6-4a7b-8c9d-0e1f2a3b4c5d",
  search: "f0e1d2c3-b4a5-4968-8776-655443322110",
  crash: "0a1b2c3d-4e5f-4a6b-8c7d-9e0f1a2b3c4d",
  release: "9f8e7d6c-5b4a-4392-8170-6f5e4d3c2b1a",
  rename: "1234abcd-5678-4ef0-9abc-def012345678",
  batch: "2c4e6a8b-0d1f-4325-9687-a1b2c3d4e5f6",
  smoke: "7b6a5948-3726-4150-8d9e-f0a1b2c3d4e5",
};

const localCards = [
  // Backlog: tasks without a run prompt or not marked for unattended runs.
  view({
    card: card("task_rate_limit", "task", projects.atlas, {
      title: "Add rate limiting to the public API",
      priority: 2,
      notes: "Token bucket per API key. Start with 600 requests per minute.",
      created_at: days(5),
    }),
    title: "Add rate limiting to the public API",
    state: "backlog",
    column_id: "backlog",
    project: projects.atlas,
    estimate: estimate(2_100_000, "project", 9),
    last_activity_at: days(5),
    reason: "task without a session",
  }),
  view({
    card: card("task_migration_guide", "task", projects.docs, {
      title: "Write the migration guide for v3",
      priority: 1,
      created_at: days(3),
    }),
    title: "Write the migration guide for v3",
    state: "backlog",
    column_id: "backlog",
    project: projects.docs,
    estimate: estimate(1_300_000, "global", 31),
    last_activity_at: days(3),
    reason: "task without a session",
  }),

  // Ready: tasks that may run unattended and carry a prompt.
  view({
    card: card("task_deps", "task", projects.store, {
      title: "Upgrade dependencies and fix the breaking changes",
      priority: 0,
      auto_run: true,
      model: "sonnet",
      run_prompt: "Upgrade every dependency to the latest minor version. Fix the breaking changes. Run the tests. Stop when they pass.",
    }),
    title: "Upgrade dependencies and fix the breaking changes",
    state: "ready",
    column_id: "ready",
    project: projects.store,
    estimate: estimate(3_400_000, "project", 6),
    last_activity_at: days(1),
    reason: "may run unattended, prompt set",
  }),
  view({
    card: card("task_payment_review", "task", projects.atlas, {
      title: "Review the payment module for race conditions",
      priority: 0,
      auto_run: true,
      model: "fable",
      run_prompt: "Review src/payments for race conditions and double charges. Write findings to REVIEW.md with file and line references. Do not change code.",
    }),
    title: "Review the payment module for race conditions",
    state: "ready",
    column_id: "ready",
    project: projects.atlas,
    estimate: estimate(5_800_000, "project", 9),
    last_activity_at: hours(20),
    reason: "may run unattended, prompt set",
  }),
  view({
    card: card("task_checkout_tests", "task", projects.store, {
      title: "Add screenshot tests for the checkout flow",
      priority: 0,
      auto_run: true,
      run_prompt: "Add Playwright screenshot tests for the three checkout pages. Reuse the existing test helpers.",
    }),
    title: "Add screenshot tests for the checkout flow",
    state: "ready",
    column_id: "ready",
    project: projects.store,
    estimate: estimate(2_600_000, "project", 6),
    last_activity_at: hours(30),
    reason: "may run unattended, prompt set",
  }),

  // Working: one interactive session in a herdr pane, one background job.
  view({
    card: card("sess_parser", "session", null, { session_id: S.parser, created_at: hours(2), updated_at: min(1) }),
    title: "Refactor the transcript parser into a streaming reader",
    state: "working",
    column_id: "working",
    project: projects.atlas,
    session: session(S.parser, projects.atlas, {
      title: "Refactor the transcript parser into a streaming reader",
      first_prompt: "Refactor the transcript parser into a streaming reader. Keep the public API. Add a benchmark.",
      last_prompt: "Good. Now make the benchmark run in CI and fail on a 20 percent regression.",
      first_at: hours(2),
      last_at: min(1),
      turns: 14,
      turn_closed: false,
      branch: "feat/streaming-parser",
      models: ["claude-opus-5"],
      tokens: { input: 210_000, output: 41_000, cache_read: 5_100_000, cache_write: 320_000, messages: 96 },
    }),
    live: live(S.parser, projects.atlas, "busy", 48213),
    herdr: {
      pane_id: "p3",
      tab_id: "t2",
      workspace_id: "ws-atlas",
      workspace_label: "atlas",
      cwd: projects.atlas.cwd,
      agent: "claude",
      agent_status: "working",
      title: "claude",
      focused: false,
      session_id: S.parser,
    },
    summary: summary(
      S.parser,
      "The parser now reads the transcript line by line and keeps the same public API. A benchmark exists. The session works on the CI step for it.",
      "working",
      0.9,
      { at: min(4) },
    ),
    hooks: { last_event: "PostToolUse", last_at: min(1), started_at: hours(2), ended_at: null, permission_pending_since: null, permission_message: null, idle_since: null, turn_active: true, events_seen: 412 },
    last_activity_at: min(1),
    reason: "registry status busy",
  }),
  view({
    card: card("task_flaky", "task", projects.infra, {
      title: "Nightly: fix the flaky integration tests",
      priority: 0,
      auto_run: true,
      model: "haiku",
      run_prompt: "Run the integration suite three times. Find the tests that fail at least once. Fix the cause, not the assertion.",
      bg_job_id: "job_7f3a2c",
      last_job_state: "working",
      last_job_at: min(38),
      created_at: days(4),
    }),
    title: "Nightly: fix the flaky integration tests",
    state: "working",
    column_id: "working",
    project: projects.infra,
    bg_job: { id: "job_7f3a2c", session_id: null, cwd: projects.infra.cwd, kind: "background", state: "working", status: "running", waiting_for: null, name: "Nightly: fix the flaky integration tests", pid: 51190, started_at: min(38) },
    estimate: estimate(1_900_000, "project", 4),
    last_activity_at: min(3),
    reason: "background job working",
  }),

  // My turn: the process is idle and the last turn finished.
  view({
    card: card("sess_cache", "session", null, { session_id: S.cache, created_at: hours(3), updated_at: min(6) }),
    title: "Explain the caching layer and propose a simpler design",
    state: "my_turn",
    column_id: "my_turn",
    project: projects.docs,
    session: session(S.cache, projects.docs, {
      title: "Explain the caching layer and propose a simpler design",
      first_prompt: "Explain how the caching layer works and propose a simpler design.",
      last_prompt: "Write the proposal as an ADR in docs/adr.",
      last_reply: "I wrote docs/adr/0007-single-cache-tier.md. It removes the second tier and keeps the invalidation hooks. Tell me if you want the diagram in it.",
      first_at: hours(3),
      last_at: min(6),
      turns: 7,
      tokens: { input: 64_000, output: 9_800, cache_read: 1_200_000, cache_write: 88_000, messages: 31 },
    }),
    live: live(S.cache, projects.docs, "idle", 47102),
    summary: summary(S.cache, "The session explained the two cache tiers and wrote an ADR that proposes one tier. It waits for a reaction to the ADR.", "my_turn", 0.85, {
      next_step: "Read the ADR and answer the diagram question.",
      at: min(5),
    }),
    hooks: { last_event: "Stop", last_at: min(6), started_at: hours(3), ended_at: null, permission_pending_since: null, permission_message: null, idle_since: min(6), turn_active: false, events_seen: 88 },
    last_activity_at: min(6),
    reason: "process idle, turn finished",
  }),

  // Decision needed: an AskUserQuestion call without an answer.
  view({
    card: card("sess_auth", "session", null, { session_id: S.middleware, created_at: hours(1), updated_at: min(12) }),
    title: "Migrate the auth middleware to the new token format",
    state: "needs_decision",
    column_id: "decision",
    project: projects.atlas,
    session: session(S.middleware, projects.atlas, {
      title: "Migrate the auth middleware to the new token format",
      first_prompt: "Migrate the auth middleware to the new token format. Keep old tokens valid for one release.",
      first_at: hours(1),
      last_at: min(12),
      turns: 5,
      turn_closed: false,
      branch: "feat/token-v2",
      pending_tools: [
        {
          id: "toolu_01",
          name: "AskUserQuestion",
          questions: [{ question: "Which format must new sessions use?", options: ["JWT with rotation", "Opaque tokens", "Keep both for one release"] }],
        },
      ],
      tokens: { input: 38_000, output: 6_100, cache_read: 900_000, cache_write: 52_000, messages: 22 },
    }),
    live: live(S.middleware, projects.atlas, "idle", 47355),
    hooks: { last_event: "PreToolUse", last_at: min(12), started_at: hours(1), ended_at: null, permission_pending_since: null, permission_message: null, idle_since: null, turn_active: true, events_seen: 40 },
    last_activity_at: min(12),
    reason: "AskUserQuestion pending",
  }),

  // Approval needed: a permission prompt reported by the hooks.
  view({
    card: card("sess_ci", "session", null, { session_id: S.ci, created_at: min(50), updated_at: min(2) }),
    title: "Clean up the CI cache and rerun the failing jobs",
    state: "needs_approval",
    column_id: "approval",
    project: projects.infra,
    session: session(S.ci, projects.infra, {
      title: "Clean up the CI cache and rerun the failing jobs",
      first_prompt: "Clean up the CI cache and rerun the failing jobs.",
      first_at: min(50),
      last_at: min(2),
      turns: 3,
      turn_closed: false,
      models: ["claude-sonnet-5"],
      tokens: { input: 12_000, output: 2_300, cache_read: 300_000, cache_write: 20_000, messages: 9 },
    }),
    live: live(S.ci, projects.infra, "idle", 47980),
    hooks: {
      last_event: "Notification",
      last_at: min(2),
      started_at: min(50),
      ended_at: null,
      permission_pending_since: min(2),
      permission_message: "Bash: gh cache delete --all",
      idle_since: null,
      turn_active: true,
      events_seen: 19,
    },
    last_activity_at: min(2),
    reason: "permission prompt pending",
  }),

  // Waiting on others: a PR is open and the summary says a review is due.
  view({
    card: card("sess_search", "session", null, { session_id: S.search, created_at: days(1), updated_at: hours(5) }),
    title: "Fix the search indexer and open a PR",
    state: "waiting_on_others",
    column_id: "waiting",
    project: projects.store,
    session: session(S.search, projects.store, {
      title: "Fix the search indexer and open a PR",
      first_prompt: "The search indexer skips products with an empty category. Fix it, add a test, open a PR.",
      last_prompt: "Push and open the PR.",
      last_reply: "Opened https://github.com/example/storefront-web/pull/412. It waits for a review from the search team.",
      first_at: days(1),
      last_at: hours(5),
      turns: 9,
      branch: "fix/indexer-empty-category",
      pr_links: ["https://github.com/example/storefront-web/pull/412"],
      tokens: { input: 71_000, output: 12_400, cache_read: 1_600_000, cache_write: 110_000, messages: 44 },
    }),
    summary: summary(S.search, "The indexer fix is pushed and a PR is open. The search team must review it before it can merge.", "waiting_on_others", 0.88, { at: hours(5) }),
    last_activity_at: hours(5),
    reason: "summary judged waiting on others",
  }),

  // Validate: a background job finished.
  view({
    card: card("task_crash", "task", projects.mobile, {
      title: "Fix the crash on empty transcript files",
      session_id: S.crash,
      priority: 0,
      auto_run: true,
      run_prompt: "Fix the crash when a transcript file is empty. Add a regression test. Open a PR.",
      bg_job_id: "job_2b9e41",
      last_job_state: "done",
      last_job_at: min(25),
      created_at: days(2),
    }),
    title: "Fix the crash on empty transcript files",
    state: "validate",
    column_id: "validate",
    project: projects.mobile,
    session: session(S.crash, projects.mobile, {
      title: "Fix the crash on empty transcript files",
      first_prompt: "Fix the crash when a transcript file is empty. Add a regression test. Open a PR.",
      last_reply: "Done. The reader returns an empty session for a zero-byte file. Regression test added. PR: https://github.com/example/mobile-app/pull/88",
      first_at: min(58),
      last_at: min(25),
      turns: 1,
      branch: "fix/empty-transcript",
      pr_links: ["https://github.com/example/mobile-app/pull/88"],
      permission_mode: "bypassPermissions",
      models: ["claude-sonnet-5"],
      tokens: { input: 29_000, output: 5_600, cache_read: 720_000, cache_write: 41_000, messages: 18 },
    }),
    bg_job: { id: "job_2b9e41", session_id: S.crash, cwd: projects.mobile.cwd, kind: "background", state: "done", status: "completed", waiting_for: null, name: "Fix the crash on empty transcript files", pid: null, started_at: min(58) },
    summary: summary(S.crash, "The reader now handles a zero-byte transcript and a regression test covers it. A PR is open and nobody checked the result yet.", "validate", 0.92, {
      next_step: "Read the diff in PR 88 and run the app once with an empty transcript.",
      at: min(24),
    }),
    estimate: estimate(1_100_000, "project", 3),
    last_activity_at: min(25),
    reason: "background job done",
  }),

  // Done.
  view({
    card: card("sess_release", "session", null, { session_id: S.release, created_at: days(3), updated_at: days(1), done_at: days(1), manual_column: "done", manual_lock_priority: 30 }),
    title: "Set up the release workflow",
    state: "done",
    column_id: "done",
    locked: true,
    project: projects.infra,
    session: session(S.release, projects.infra, {
      title: "Set up the release workflow",
      first_prompt: "Set up a GitHub release workflow that builds the app for both Mac architectures.",
      first_at: days(3),
      last_at: days(1),
      turns: 11,
      tokens: { input: 90_000, output: 15_000, cache_read: 2_000_000, cache_write: 140_000, messages: 52 },
    }),
    last_activity_at: days(1),
    reason: "manual move",
  }),
  view({
    card: card("sess_keys", "session", null, { session_id: S.rename, created_at: days(6), updated_at: days(4), done_at: days(1) }),
    title: "Rename the config keys to snake case",
    state: "done",
    column_id: "done",
    project: projects.docs,
    session: session(S.rename, projects.docs, {
      title: "Rename the config keys to snake case",
      first_prompt: "Rename every config key to snake case and update the docs.",
      first_at: days(6),
      last_at: days(4),
      turns: 4,
      models: ["claude-sonnet-5"],
      tokens: { input: 22_000, output: 3_900, cache_read: 480_000, cache_write: 30_000, messages: 14 },
    }),
    last_activity_at: days(4),
    reason: "no activity for 3 days, judged done",
  }),
];

// Cards on the remote node. The transcript paths and the projects live on that machine.
const labCards = [
  view({
    card: card("task_scheduler", "task", projects.batch, {
      title: "Move the nightly job to the new scheduler",
      priority: 0,
      created_at: days(4),
    }),
    title: "Move the nightly job to the new scheduler",
    state: "backlog",
    column_id: "backlog",
    project: projects.batch,
    estimate: estimate(900_000, "global", 31),
    last_activity_at: days(4),
    reason: "task without a session",
  }),
  view({
    card: card("task_retrain", "task", projects.ranker, {
      title: "Retrain the ranking model on the new dataset",
      priority: 0,
      auto_run: true,
      model: "sonnet",
      run_prompt: "Retrain the ranking model on the September dataset. Report the offline metrics in RESULTS.md.",
      created_at: days(1),
    }),
    title: "Retrain the ranking model on the new dataset",
    state: "ready",
    column_id: "ready",
    project: projects.ranker,
    estimate: estimate(2_800_000, "project", 5),
    last_activity_at: hours(14),
    reason: "may run unattended, prompt set",
  }),
  view({
    card: card("sess_batch", "session", null, { session_id: S.batch, created_at: hours(4), updated_at: min(2) }),
    title: "Cut the memory use of the batch job",
    state: "working",
    column_id: "working",
    project: projects.batch,
    session: session(S.batch, projects.batch, {
      title: "Cut the memory use of the batch job",
      first_prompt: "The nightly batch job needs 14 GB. Find where the memory goes and cut it.",
      last_prompt: "Good. Now stream the parquet files instead of loading them.",
      first_at: hours(4),
      last_at: min(2),
      turns: 16,
      turn_closed: false,
      branch: "perf/stream-parquet",
      tokens: { input: 180_000, output: 33_000, cache_read: 4_200_000, cache_write: 260_000, messages: 84 },
    }),
    live: live(S.batch, projects.batch, "busy", 8842),
    summary: summary(S.batch, "The job loaded every parquet file at once. The session rewrites the reader to stream and measures the peak again.", "working", 0.88, {
      at: min(6),
    }),
    last_activity_at: min(2),
    reason: "registry status busy",
  }),
  view({
    card: card("task_smoke", "task", projects.batch, {
      title: "Add a smoke test for the deploy script",
      session_id: S.smoke,
      priority: 0,
      auto_run: true,
      run_prompt: "Add a smoke test that runs the deploy script against the staging config. It must fail on a missing secret.",
      bg_job_id: "job_5c1d80",
      last_job_state: "done",
      last_job_at: min(18),
      created_at: days(2),
    }),
    title: "Add a smoke test for the deploy script",
    state: "validate",
    column_id: "validate",
    project: projects.batch,
    session: session(S.smoke, projects.batch, {
      title: "Add a smoke test for the deploy script",
      first_prompt: "Add a smoke test that runs the deploy script against the staging config.",
      last_reply: "The test runs the script with a staging config and fails when a secret is missing. It runs in 9 seconds.",
      first_at: min(44),
      last_at: min(18),
      turns: 1,
      branch: "test/deploy-smoke",
      permission_mode: "bypassPermissions",
      models: ["claude-sonnet-5"],
      tokens: { input: 21_000, output: 4_100, cache_read: 540_000, cache_write: 33_000, messages: 12 },
    }),
    bg_job: {
      id: "job_5c1d80",
      session_id: S.smoke,
      cwd: projects.batch.cwd,
      kind: "background",
      state: "done",
      status: "completed",
      waiting_for: null,
      name: "Add a smoke test for the deploy script",
      pid: null,
      started_at: min(44),
    },
    summary: summary(S.smoke, "A smoke test for the deploy script exists and it fails when a secret is missing. Nobody checked the result yet.", "validate", 0.9, {
      next_step: "Read the test and run it once against staging.",
      at: min(17),
    }),
    estimate: estimate(800_000, "project", 3),
    last_activity_at: min(18),
    reason: "background job done",
  }),
];

/** Every card carries the node it comes from. */
const tag = (list, node) => list.map((c) => ({ ...c, node_id: node.id, node_name: node.name }));
const cards = [...tag(localCards, NODES[0]), ...tag(labCards, NODES[1])];

const readyItems = localCards.filter((c) => c.state === "ready");
const proposal = {
  id: "prop_01",
  created_at: min(2),
  trigger: "manual",
  reason: "The 5-hour window has 62% free and resets in 2h 10m",
  items: readyItems.map((c) => ({
    card_id: c.card.id,
    title: c.title,
    project_name: c.project_name,
    prompt: c.card.run_prompt,
    model: c.card.model,
    estimate: c.estimate,
    job_id: null,
    error: null,
  })),
  budget_pct: 32,
  used_pct_before: 38,
  total_pct: readyItems.reduce((n, c) => n + c.estimate.pct_five_hour, 0),
  used_pct_after: 38 + readyItems.reduce((n, c) => n + c.estimate.pct_five_hour, 0),
  skipped: 0,
  expires_at: ahead(58),
  state: "open",
  auto: false,
  accepted_at: null,
};

/** The dry run of a node's planner: what it would run next, in order, and when.
 *  `studio` has a plan open, so its steps wait for a click. `lab` waits for the
 *  weekly trigger, and its last card does not fit the budget. */
function queueOf(nodeCards, usedPct, budgetPct, trigger, openProposal) {
  const ready = nodeCards.filter((c) => c.state === "ready" || c.state === "backlog").slice(0, 4);
  let total = 0;
  return {
    steps: ready.map((c) => {
      const cost = c.estimate?.pct_five_hour ?? 4;
      const fits = total + cost <= budgetPct;
      if (fits) total += cost;
      return {
        card_id: c.card.id,
        title: c.title,
        project_name: c.project_name,
        model: c.card.model,
        estimate: c.estimate ?? estimate(2_000_000, "project", 2),
        window_after_pct: usedPct + total,
        fits,
        starts_at: fits ? (openProposal ? min(0) : trigger) : null,
        reason: fits ? "now" : "does not fit the budget",
      };
    }),
    budget_pct: budgetPct,
    used_pct: usedPct,
    next_check_at: ahead(1),
    next_trigger_at: trigger,
    next_trigger: "weekly_reset",
    mode: "ask",
    blocked: null,
    open_proposal: openProposal,
  };
}

const board = {
  columns,
  nodes: NODES,
  cards,
  quotas: [
    {
      node_id: "local",
      node_name: "studio",
      quota: {
        at: min(1),
        five_hour: { used_percentage: 38, resets_at: ahead(130) },
        seven_day: { used_percentage: 52, resets_at: ahead(3 * 24 * 60 + 15) },
        source: "statusline",
      },
      calibration,
    },
    {
      node_id: "lab",
      node_name: "lab",
      quota: {
        at: min(2),
        five_hour: { used_percentage: 14, resets_at: ahead(96) },
        seven_day: { used_percentage: 27, resets_at: ahead(4 * 24 * 60) },
        source: "statusline",
      },
      calibration,
    },
  ],
  queues: [
    { node_id: "local", node_name: "studio", queue: queueOf(localCards, 38, 17, min(0), true) },
    { node_id: "lab", node_name: "lab", queue: queueOf(labCards, 14, 41, ahead(14 * 60), false) },
  ],
  proposals: [{ node_id: "local", node_name: "studio", proposal }],
  generated_at: min(0),
  scanning: false,
  herdr_connected: true,
  hooks_installed: true,
  hooks_port: 47311,
};

const settings = {
  node_name: "studio",
  history_days: 30,
  done_after_days: 3,
  stale_after_days: 14,
  terminal_app: "iTerm",
  default_permission_mode: "auto",
  default_run_model: "",
  max_parallel_bg: 2,
  summaries_enabled: true,
  summaries_per_hour: 6,
  summary_model: "haiku",
  summary_recent_hours: 48,
  hooks_port: 47311,
  usage_endpoint_enabled: false,
  proposals_enabled: true,
  weekly_unused_pct: 40,
  weekly_hours_before_reset: 36,
  five_hour_idle_pct: 30,
  idle_minutes: 45,
  working_hours_start: 8,
  working_hours_end: 20,
  working_hours_reserve_pct: 30,
  fill_ceiling_pct: 85,
  autopilot: false,
  autopilot_max_jobs: 1,
  prefer_herdr: true,
  weekly_warn_unused_pct: 25,
};

const jobLog = {
  task_crash: [
    { at: min(25), job_id: "job_2b9e41", card_id: "task_crash", state: "done", detail: "job completed, PR opened" },
    { at: min(31), job_id: "job_2b9e41", card_id: "task_crash", state: "working", detail: "tests pass, pushing" },
    { at: min(57), job_id: "job_2b9e41", card_id: "task_crash", state: "working", detail: "started by plan prop_00 with model sonnet" },
    { at: min(58), job_id: "job_2b9e41", card_id: "task_crash", state: "queued", detail: "claude --bg accepted the job" },
  ],
  task_smoke: [
    { at: min(18), job_id: "job_5c1d80", card_id: "task_smoke", state: "done", detail: "job completed, test added" },
    { at: min(30), job_id: "job_5c1d80", card_id: "task_smoke", state: "working", detail: "test runs against the staging config" },
    { at: min(44), job_id: "job_5c1d80", card_id: "task_smoke", state: "queued", detail: "claude --bg accepted the job" },
  ],
  task_flaky: [
    { at: min(3), job_id: "job_7f3a2c", card_id: "task_flaky", state: "working", detail: "second suite run" },
    { at: min(37), job_id: "job_7f3a2c", card_id: "task_flaky", state: "working", detail: "started from the card drawer with model haiku" },
    { at: min(38), job_id: "job_7f3a2c", card_id: "task_flaky", state: "queued", detail: "claude --bg accepted the job" },
  ],
};

export function writeFixtures(out = "docs/demo") {
  mkdirSync(out, { recursive: true });
  const write = (name, data) => writeFileSync(join(out, name), JSON.stringify(data, null, 2) + "\n");
  write("board.json", board);
  write("settings.json", settings);
  write("job-log.json", jobLog);
  console.log(`wrote board.json, settings.json and job-log.json to ${out}/`);
}

if (import.meta.main) writeFixtures(process.argv[2]);
