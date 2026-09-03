# kari — design

kari (Estonian: herd) is a macOS tray app that turns Claude Code sessions into cards on a Kanban board, keeps the board current without manual work, and uses left-over subscription quota to start backlog work.

Status: design accepted on 2026-09-02. Decisions in this document come from the maintainer. See "Decisions" at the end.

## 1. Problem

A developer who runs many Claude Code sessions in parallel, in bare terminals and in herdr workspaces, meets three problems:

1. No overview. Which sessions wait for a decision, which run, which are finished and will never be touched again?
2. Manual bookkeeping. A task tracker that must be updated by hand falls behind within a day.
3. Wasted quota. The 5-hour and 7-day windows of a Claude subscription often reset with unused capacity, while a backlog of unattended work waits.

## 2. Market check (2026-09-02)

Fifteen tools were reviewed. None combines the three requirements.

| Tool | Auto board from sessions | Backlog that launches sessions | Quota-aware scheduling |
|---|---|---|---|
| claude-code-kanban | yes (hooks, observe only) | no | no |
| kandev | manual status | yes | no |
| Superset | agent status in sidebar | yes (kanban over MCP) | usage gauge only |
| opcode (Claudia) | session browser | no | local cost analytics |
| herdr | pane states idle/working/blocked | workspaces, no backlog | no |
| CCSeva, ccusage, claude-monitor | no | no | quota display only |
| vibe-kanban, Crystal, Terragon | — | — | sunset or deprecated |

Result: build kari. Reuse herdr for terminal management and Claude Code background agents for unattended runs.

## 3. Goals and non-goals

Goals:

- Every Claude Code session on this Mac appears as a card within seconds of its first prompt.
- Card state follows the session automatically. Manual moves are allowed and hold until the session produces a stronger signal.
- Cards that wait on the user are split by what they need: a decision between options, an approval, or plain input.
- Backlog tasks that are not sessions yet can be added, estimated, and started as sessions.
- kari proposes a plan to use left-over quota. The user confirms. Automatic starts exist only behind the Autopilot switch, which is off by default.
- One click opens the session where it lives: the herdr pane, or a new iTerm2 window.

Non-goals for version 1:

- Multi-machine merge of boards.
- Automatic starts without confirmation.
- Replacing herdr or the Claude Code agent view.

## 4. Platform choice: Tauri 2 with a Rust core

A web app needs a local daemon for file access and process control in any case, and a browser tab cannot notify when closed. Electron gives the same result as Tauri at 200 MB. Native SwiftUI needs Xcode and locks the tool to one platform.

Tauri 2 gives:

- A tray-resident process that runs watchers and the scheduler while the window is closed.
- Native notifications with click actions that jump into a session.
- Direct file system and process access from Rust, no sandbox negotiations.
- One small binary. The core is a plain Rust crate, so a headless `kari` CLI is cheap later.

Frontend: React 19, TypeScript, Vite, dnd-kit for drag and drop.

## 5. Data sources

kari never asks the user for information that Claude Code already writes to disk.

| Source | Path or command | Gives | Freshness |
|---|---|---|---|
| Live session registry | `~/.claude/sessions/<pid>.json` | pid, session id, cwd, display name, name source, status `idle` / `busy` / `shell`, start time | file watch, instant |
| Transcripts | `~/.claude/projects/<slug>/<session-id>.jsonl` | AI title, custom title, prompts, per-message token usage, model, git branch, PR links, turn durations, pending tool calls | file watch, tail parse |
| Background agents | `claude agents --json --all`, `~/.claude/jobs/<id>/state.json` | job id, state `working` / `blocked` / `done` / `failed` / `stopped`, `waitingFor` reason | 15 s poll plus file watch |
| Hooks | `Notification`, `Stop`, `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse` as `command` hooks that run `~/.config/kari/hook.sh`, which posts the payload to `127.0.0.1:47311/kari/hook` | exact events: `permission_prompt`, `idle_prompt`, `agent_needs_input`, turn start and end, tool runs | instant, optional |
| Status line | wrapper script writes `~/.config/kari/rate-limits.json` | `rate_limits.five_hour` and `seven_day`: `used_percentage`, `resets_at` | each status line refresh |
| OAuth usage endpoint | `GET api.anthropic.com/api/oauth/usage` | same windows, server truth, includes other devices | fallback poll every 3 min when no status line sample for 5 min |
| herdr | `~/.config/herdr/herdr.sock`, newline JSON | workspaces, tabs, panes, agent status `idle` / `working` / `blocked` / `done` | socket poll every 15 s |

Transcript format is internal to Claude Code and can change. The parser is tolerant: unknown record types are skipped, and every field is optional.

## 6. Domain model

```
Project      cwd, slug, display name, herdr workspace id
Session      session id, project, transcript path, pid, registry status, names,
             started, last activity, turns, tokens (in, out, cache read, cache
             write), models, branch, PR links, pending interaction, bg job
Card         id, kind (session | task), title, column, session id, project,
             priority, auto_run, run prompt, permission override, estimate,
             manual lock, tags, notes, done at
Summary      session id, narrative, open questions, next step, judged state,
             generated at, source (haiku | heuristic)
QuotaSample  time, five_hour %, five_hour reset, seven_day %, seven_day reset, source
Column       id, name, order, accepted states, WIP limit, color
Proposal     created, trigger, tasks with estimates, budget used, expiry
```

State lives in SQLite at `~/.config/kari/kari.db`. Transcripts are not copied. kari stores byte offsets per transcript and parses only appended lines.

## 7. State inference

kari derives one state per card from the sources above. Columns are configurable. Each column accepts a set of derived states.

| Derived state | Signals, in priority order |
|---|---|
| `working` | registry status `busy`, or bg job `working` |
| `needs_decision` | last assistant message has an `AskUserQuestion` call without a result, or bg `waitingFor` is `input needed` |
| `needs_approval` | `permission_prompt` notification without a following turn, pending `ExitPlanMode`, or bg `waitingFor` is `permission prompt`, `sandbox request`, `dialog open` |
| `my_turn` | process alive, status `idle`, last turn finished. The user is between prompts |
| `waiting_on_others` | manual move, or the Haiku summary judges that an external party must act (review, reply, deploy) |
| `validate` | Haiku judges the task complete but unverified, or a PR is open on the session branch, or a bg job reached `done` |
| `done` | manual move, or the PR is merged, or judged done with no live process and no activity for 3 days |
| `stale` | no live process and no activity for 14 days, not judged done. Hidden by default |
| `backlog` | task card without a session |
| `ready` | backlog card with `auto_run` set and a run prompt |

Rules:

- A manual move sets a lock. The lock holds until a signal with higher priority than the locked column arrives. `needs_decision` and `needs_approval` always break a lock, because they need the user.
- Titles: custom title, else AI title, else registry name, else first prompt truncated.
- Two records in the transcript decide "between prompts": a `system` record with subtype `turn_duration` after the last assistant message, and registry status `idle`.

### Default columns

| Column | Accepts |
|---|---|
| Backlog | `backlog` |
| Ready | `ready` |
| Working | `working` |
| My turn | `my_turn` |
| Decision needed | `needs_decision` |
| Approval needed | `needs_approval` |
| Waiting on others | `waiting_on_others` |
| Validate | `validate` |
| Done | `done` |

Columns can be renamed, merged (one column accepts several states), reordered, hidden, and given WIP limits. The mapping is stored in the database and exported as JSON.

## 8. Summaries

After a turn ends, kari asks Haiku for a summary of the session, throttled to at most 6 calls per hour and at most one per session per 10 minutes.

Command:

```
claude -p --model haiku --no-session-persistence --setting-sources "" --tools "" \
  --strict-mcp-config --mcp-config '{"mcpServers":{}}' --output-format json --max-turns 1 \
  --append-system-prompt "<kari summary schema>" < transcript-excerpt.txt
```

`--bare` is not used: in Claude Code 2.1.258 it skips the OAuth credentials and the call fails with "Not logged in". The flags above give the same effect: no hooks, no tools, a system prompt of about 7 000 tokens per call.

The excerpt holds the last 30 messages, trimmed to text. The answer is JSON: `narrative` (2 sentences), `open_questions` (list), `next_step`, `judged_state` (one of the derived states or `unknown`), `confidence`. The card shows the narrative and the open questions. A low confidence never overrides a hard signal.

Heuristics fill the card when the throttle blocks a call.

## 9. Quota model and scheduler

### Samples

The status line wrapper writes a sample on every refresh. The sample holds both windows with `used_percentage` and `resets_at`. When no session refreshed the status line for 5 minutes, kari polls the OAuth usage endpoint.

### Estimates

Rate limits are percentages, not tokens. kari learns a calibration factor: percent of the 5-hour window per million weighted tokens.

The status line reports whole percent steps and refreshes every few seconds, so consecutive samples usually repeat the same number. kari first reduces the series to the points where the number changed, then pairs those points. Each pair takes the percent step and the weighted-token growth kari recorded in the same interval, from the `token_deltas` table. Pairs that span a window reset, that are more than 30 minutes apart, or that hold less than 20 000 weighted tokens are dropped, and so are ratios outside 0.05 to 50. The median of the rest is the factor, with the quartiles as the band. Fewer than five pairs means the prior of 3.0 percent per million weighted tokens holds.

Per-task estimate: median weighted tokens of finished sessions in the same project, else the global median, else 4M. A session card is a continuation, so its estimate is eight turns at that session's own rate per turn. Estimates show a band.

### Proposals

Triggers:

1. The 7-day window has more than X percent unused and fewer than Y hours to reset. Defaults: 40 percent, 36 hours.
2. The 5-hour window is below Z percent and no interactive session was active for N minutes. Defaults: 30 percent, 45 minutes.
3. The user clicks "Fill the quota".

The planner ranks `ready` cards by priority, then age. It packs them into the budget with a greedy fit and a headroom reserve. During working hours (default 08:00 to 20:00) the plan keeps at least 30 percent of the 5-hour window free for interactive work. Parallelism is capped at 2 background jobs.

The proposal is a notification and a panel: tasks, estimate per task, total, budget after the run, and the reason for the trigger. Buttons: Start, Start all, Snooze 1 hour, Dismiss. Snooze holds the trigger for an hour. Dismiss holds it until its window moves on: the weekly trigger until the window resets, the idle trigger for two hours.

Only one proposal is open at a time. An open proposal expires after two hours. An accepted proposal stays on the panel for 30 minutes so its jobs can be stopped from one place.

### Runs

Start uses Claude Code background agents:

```
cd <project cwd>
claude --bg --permission-mode bypassPermissions [--model <model>] --name <card-slug> "<run prompt>"
claude --bg --resume <session-id> [--model <model>] "<continue prompt>"    # for session cards
```

The model comes from the card, else from `default_run_model` in settings, else from Claude Code. The value is an alias (`fable`, `opus`, `sonnet`, `haiku`) or a full model name. Jump in uses the same value, both for a terminal and for a herdr pane.

Default permission mode is `bypassPermissions`, overridable per card. Background agents move edits into a git worktree under `.claude/worktrees/`, so parallel runs do not collide. kari records the job id, follows `state.json`, and moves the card: `working` → `validate` on `done`, `needs_approval` on `blocked`, and a notification on `failed`.

kari writes one `job_log` row per state change and keeps the last state on the card, because `claude agents` forgets a job after a while. A card with a remembered `done` sits in Validate until the session shows newer work, or until the user moves it.

Safety: a kill switch in the tray stops all kari-started jobs with `claude stop <id>`. The first click arms the menu item, a second click within 10 seconds acts. A card must carry `auto_run` explicitly to be eligible.

Autopilot (off by default) accepts a weekly-reset plan without a click, up to `autopilot_max_jobs`. It sends a notice and the panel keeps a Stop button. The idle trigger and the manual button always wait for a click.

## 10. Jump in

| Where the session lives | Action |
|---|---|
| herdr pane | `agent.focus` and `workspace.focus` over the herdr socket, then bring herdr's terminal to front |
| no pane, herdr running | `tab.create` with the project cwd, then `agent.start` with kind `claude` in the new pane (`--resume <id>` for a session card). A fresh pane answers `agent_pane_busy` for a moment, so kari retries for 6 seconds. The agent name must be a slug. |
| background job | terminal window: `claude attach <job-id>` |
| exited, transcript only | terminal window in the project cwd: `claude --resume <session-id>` |

The terminal (iTerm2, Terminal or Ghostty, set in Settings) is driven with `osascript`. herdr panes are matched to sessions by the session id when the herdr Claude integration is installed, else by cwd and title.

## 11. Notifications

- Decision needed, approval needed. Click opens the session.
- Background job finished or failed.
- Proposal ready.
- The weekly window resets within 24 hours and more than 25 percent is unused. Once per window.
- A column holds more cards than its WIP limit. Once per hour per column.

## 12. Architecture

```
crates/kari-core   plain Rust: readers, parser, inference, quota, planner, herdr client, launcher, sqlite store
src-tauri          Tauri app: commands, events to the UI, tray, notifications, hook receiver (axum on 127.0.0.1)
src                React UI: board, card drawer, quota bar, proposals, settings
scripts            statusline wrapper, version bump
```

Flow: watchers and pollers in `kari-core` emit domain events on a channel. A reducer updates the store and computes derived state. The Tauri layer forwards a `board_changed` event to the UI, which re-fetches the board through a command. The UI never reads files.

## 13. Milestones

1. Board from local data: registry, transcripts, herdr mapping, configurable columns, backlog cards, manual moves, jump in, quota bar from the status line. Read only, no Claude calls.
2. Hooks receiver, decision and approval detection, Haiku summaries, notifications, tray. Built 2026-09-02. The relay is a `command` hook, not an `http` hook, so a closed kari never shows an error in a session.
3. Estimates and calibration, proposals, background runs, job tracking, kill switch. Built 2026-09-03.
4. Automatic starts on a schedule, herdr as a launch target. Built 2026-09-03. Multi-machine merge is still open.

## 14. Risks

| Risk | Mitigation |
|---|---|
| Transcript and registry formats are internal and change between releases | Tolerant parser, optional fields, a format test per Claude Code version, hooks as a second source |
| The OAuth usage endpoint is undocumented and rate limited | Status line is the primary source. The endpoint is a fallback with a 3-minute floor and the Claude Code user agent |
| `bypassPermissions` in unattended runs | Worktree isolation by background agents, explicit `auto_run` per card, kill switch, a run log per job |
| Percent-to-token calibration is noisy | Confidence bands, conservative headroom, the planner never fills past 85 percent of a window |
| Haiku summaries spend quota | Hard throttle, heuristics fallback, off switch |

## 15. Decisions

Made on 2026-09-02:

- Name: kari.
- Stack: Tauri 2 with a Rust core, React and TypeScript frontend.
- Runner: Claude Code background agents (`claude --bg`).
- Default permission mode for unattended runs: `bypassPermissions`, overridable per card.
- Scope: this Mac only. Sync later.
- Jump in: herdr pane when present, else iTerm2.
- Scheduler: propose, the user confirms.
- Summaries: throttled Haiku.
