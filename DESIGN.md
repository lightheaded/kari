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

- Automatic starts without confirmation.
- Replacing herdr or the Claude Code agent view.

Multi-machine work arrived in version 2. See "Remote nodes".

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
| Hooks | `Notification`, `Stop`, `SessionStart`, `SessionEnd`, `UserPromptSubmit`, `PreToolUse`, `PostToolUse` as `command` hooks that run `~/.config/kari/hook.sh`, which posts the payload to `127.0.0.1:47311/kari/hook` with a token from `~/.config/kari/hook-token` | exact events: `permission_prompt`, `idle_prompt`, `agent_needs_input`, turn start and end, tool runs | instant, optional |
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
| `ready` | backlog card with `auto_run` set and a title or a run prompt |

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
| Needs me | `needs_approval`, `needs_decision`, `my_turn`, `unknown` |
| Review | `validate`, `waiting_on_others` |
| Done | `done` |

Six columns fit a 1440-pixel window with no sideways scroll. Version 0.4.1 and
earlier shipped nine, one per state, and that no longer fits. A column that
accepts more than one state groups its cards by state inside itself, most urgent
first, with a collapsible sub-header per group.

Columns can be renamed, split again, reordered, hidden, and given WIP limits.
The mapping is stored in the database and exported as JSON.

kari replaces a stored nine-column layout with the six once, at start-up, and
only when that layout still matches the old defaults exactly. A layout the user
changed is left alone. Manual locks follow their column: `my_turn`, `decision`
and `approval` become `needs_me`, and `waiting` and `validate` become `review`.
The app says so once in a toast. "Reset to defaults" gives the six back.

### Board scrolling

Three ways sideways, because a mouse has no horizontal wheel:

- A trackpad and Shift with the wheel already send a horizontal delta.
- A plain wheel over a column that cannot scroll further moves the board instead.
- A drag on the ground between the columns pans the board.

The board also keeps a visible scrollbar, because macOS hides an overlay one.

### Card order inside a column

Every card carries a priority. Zero means the automatic order: the urgency of
the derived state, then recency. A non-zero priority means the user placed the
card by hand, and every placed card sorts above every automatic one.

A drag writes the priorities. Dropping a card places it and everything above it
in the column, counting down to the lowest card that was placed already.
Everything below that keeps priority zero, so a card nobody dragged never jumps.
The priority box in the drawer still works and writes the same number, and the
planner reads it, so the top of a placed backlog is also the first card a plan
takes. A column that holds a placed card shows a reset in its header, which
gives the whole column back to the automatic order.

Priorities live in the store of the node that owns the card, so a reorder writes
to one node at a time.

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

### The automation mode

One three-state mode says how much of this runs:

| Mode | `proposals_enabled` | `autopilot` |
|---|---|---|
| `off` | false | untouched |
| `ask` | true | false |
| `auto` | true | true |

The mode is derived from those two flags, never stored on its own. A stored
field cannot work here: `Settings` carries `#[serde(default)]`, so a record
written before the field existed comes back holding the struct default, and that
is indistinguishable from a mode the user chose. Deriving it means a record from
an older kari keeps saying exactly what it always said.

`off` leaves `autopilot` alone, so the flag still holds what the user asked for
the last time plans were on. Only `proposals_enabled` gates the planner, so
nothing runs while the mode is `off` either way.

The mode is per node, because each node runs its own planner. The control in the
top bar sets every node that answers at once, or the one node the filter names.
Settings holds the mode of the local node on its own.

### The queue

The queue is a dry run of the planner. It starts nothing and stores nothing:
`planner::queue` answers from the board it is handed, and the answer travels
with `BoardView`, so the strip needs no extra call.

Each step names the card, the cost as a percent of the 5-hour window, the state
of the window after it, whether it fits the budget, and the start time. That
time comes from `planner::next_trigger_at`: the weekly trigger fires a set
number of hours before the 7-day window resets, and the idle trigger fires once
nobody worked for long enough. The earlier of the two wins. A trigger that is
live already reads as "now".

The strip also names why nothing can run at all: the mode is off, no quota
sample arrived, every job slot is busy, the budget is too small, or no card is
marked "may run unattended".

### Proposals

Triggers:

1. The 7-day window has more than X percent unused and fewer than Y hours to reset. Defaults: 40 percent, 36 hours.
2. The 5-hour window is below Z percent and no interactive session was active for N minutes. Defaults: 30 percent, 45 minutes.
3. The user clicks "Fill the quota".

The planner ranks `ready` cards by priority, then age. It packs them into the budget with a greedy fit and a headroom reserve. During working hours (default 08:00 to 20:00) the plan keeps at least 30 percent of the 5-hour window free for interactive work. Parallelism is capped at 2 background jobs.

The proposal is a notification and a panel: a budget bar, tasks with an estimate each, and the reason for the trigger. The bar fills as the user picks tasks and marks where the budget ends. Cards that did not fit stay in the proposal, unpicked, with the reason (budget or the parallel cap). The user can pick them to override the planner. A manual plan always lists every candidate, even when nothing fits. Autopilot never starts a card that did not fit. Buttons: Start, Start all, Snooze 1 hour, Dismiss. Snooze holds the trigger for an hour. Dismiss holds it until its window moves on: the weekly trigger until the window resets, the idle trigger for two hours.

Only one proposal is open at a time. An open proposal expires after two hours. An accepted proposal stays on the panel for 30 minutes so its jobs can be stopped from one place.

### Runs

Start uses Claude Code background agents:

```
cd <project cwd>
claude --bg --permission-mode <mode> [--model <model>] --name <card-slug> -- "<run prompt>"
claude --bg --resume <session-id> [--model <model>] "<continue prompt>"    # for session cards
```

The run prompt of a task card is its title, a blank line, then its body, so the
title never has to be repeated in the body. A session card sends its body alone,
because its title comes from the transcript and is not an instruction. A one-off
prompt from the drawer replaces both.

The model comes from the card, else from `default_run_model` in settings, else from Claude Code. The value is an alias (`fable`, `opus`, `sonnet`, `haiku`) or a full model name. Jump in uses the same value, both for a terminal and for a herdr pane.

The default permission mode is `auto` (until 2026-09-03: `bypassPermissions`), set in Settings and overridable per card. `--` ends the options, so a prompt that starts with `-` stays a prompt. Background agents move edits into a git worktree under `.claude/worktrees/`, so parallel runs do not collide. kari records the job id, follows `state.json`, and moves the card: `working` → `validate` on `done`, `needs_approval` on `blocked`, and a notification on `failed`.

kari writes one `job_log` row per state change and keeps the last state on the card, because `claude agents` forgets a job after a while. A card with a remembered `done` sits in Validate until the session shows newer work, or until the user moves it.

Safety: a kill switch in the tray stops all kari-started jobs with `claude stop <id>`. The first click arms the menu item, a second click within 10 seconds acts. A card must carry `auto_run` explicitly to be eligible.

Mode `auto` accepts a weekly-reset plan without a click, up to `autopilot_max_jobs`. It sends a notice and the panel keeps a Stop button. The idle trigger and the manual button always wait for a click. See "The automation mode".

## 10. Jump in

| Where the session lives | Action |
|---|---|
| herdr pane | `agent.focus` and `workspace.focus` over the herdr socket, then bring herdr's terminal to front |
| no pane, herdr running | `tab.create` in the workspace of the project cwd, then `agent.start` with kind `claude` in the new pane (`--resume <id>` for a session card). A fresh pane answers `agent_pane_busy` for a moment, so kari retries for 6 seconds. The agent name must be a slug. |
| background job | terminal window: `claude attach <job-id>` |
| exited, transcript only | terminal window in the project cwd: `claude --resume <session-id>` |

The terminal (iTerm2, Terminal or Ghostty, set in Settings) is driven with `osascript`. herdr panes are matched to sessions by the session id when the herdr Claude integration is installed, else by cwd and title.

A `tab.create` with no workspace lands in the focused workspace, so kari names
one. A herdr workspace carries no directory of its own, so kari reads the
directory of each pane and takes the workspace that already sits in the project
cwd. When more than one workspace sits there, the workspace with the most recent
agent state change wins, and the place on the workspace bar breaks a tie. When
no workspace sits there, kari creates one with `workspace.create`, names it
after the directory, and renames its first tab to the card title. That workspace
holds one tab, so `close_herdr_tab_on_done` removes the workspace with it.

The reverse step is `close_herdr_tab_on_done`, off by default. A move to a
Done column closes the tab of the pane that the board matched to the card. An
archive does the same. The engine reads the tab from the board before it writes
the card, because an archived card leaves the board. It closes the tab after
the write, so a write that fails leaves the terminal alone. The engine of the
node that owns the card does this work, and that node is the host that runs
herdr. A `tab.close` that fails is logged and changes nothing else.

## 11. Notifications

- Decision needed, approval needed. Click opens the session.
- Background job finished or failed.
- Proposal ready.
- The weekly window resets within 24 hours and more than 25 percent is unused. Once per window.
- A column holds more cards than its WIP limit. Once per hour per column.

## 12. Architecture

```
crates/kari-core   plain Rust: readers, parser, inference, quota, planner, herdr client, launcher,
                   sqlite store, the HTTP API (axum), the API client, the hub, the SSH tunnel
crates/kari-cli    `kari-node`: the same engine without a window, plus the installers
                   `kari-server`: the hub on a host that stays up, and the node links into it
src-tauri          Tauri app: commands over the hub, events to the UI, tray, notifications
src                React UI: board, card drawer, stats strip, queue strip, proposals, settings, nodes
scripts            statusline wrapper, version bump
```

The window keeps its size and its place across a restart, in
`~/.config/kari/window.json`. Only those two, and kari writes them itself.
`tauri-plugin-window-state` does the same job, but it hides the window before it
restores and shows it again only with its `VISIBLE` flag; kari lives in the
tray, so whether the window was open at exit must not decide whether it opens
at the next start. The write waits for the last event of a drag, so one drag
costs one write. A saved rectangle that no display covers any more keeps its
size and lets the system place the window, so a window never opens off screen
after a monitor goes away.

Flow: watchers and pollers in `kari-core` emit domain events on a channel. A reducer updates the store and computes derived state. The hub turns an event of the local engine, or of a remote node, into a `board_changed` event for the UI, which re-fetches the merged board through one command. The UI never reads files and never opens a socket.

## 13. Milestones

1. Board from local data: registry, transcripts, herdr mapping, configurable columns, backlog cards, manual moves, jump in, quota bar from the status line. Read only, no Claude calls.
2. Hooks receiver, decision and approval detection, Haiku summaries, notifications, tray. Built 2026-09-02. The relay is a `command` hook, not an `http` hook, so a closed kari never shows an error in a session.
3. Estimates and calibration, proposals, background runs, job tracking, kill switch. Built 2026-09-03.
4. Automatic starts on a schedule, herdr as a launch target. Built 2026-09-03.
5. Remote nodes: the headless node, the hub, one board over many hosts. Built 2026-09-03. See "Remote nodes".
6. The server: nodes that dial out, one hub off the client, the lease removed, quota per account across nodes. See "The server".
7. One engine per machine: a daemon on every host, and the claim that stops it running beside a window. See "One engine per machine".

## 14. Remote nodes

A developer works on more than one machine: a laptop and a server that runs
unattended jobs. Version 2 shows every machine on one board.

Version 3 keeps this section's engine, API and card rules and changes who dials
whom. Where the two disagree — the direction of a connection, who owns the
columns, and the primary lease — "The server" is the current design and this
section is the history.

### Parts

| Part | Runs where | Does |
|---|---|---|
| Node daemon `kari-node serve` | every host, including a server without a screen | The same `kari-core` engine, without a window. Serves the board and every action over HTTP on `127.0.0.1`. |
| Hub | inside the desktop app | Holds the local engine and one client per remote node. Merges the boards. Routes each action to the node that owns the card. |
| Desktop app | the machine the user sits at | The board, the drawer, the tray, the notifications. |

The engine did not change. The Tauri layer wraps each engine method in one
command, and the node wraps the same methods in one HTTP route.

### Transport

The node binds loopback only. The desktop app opens an SSH port forward to it:

```
ssh -N -o ExitOnForwardFailure=yes -o ServerAliveInterval=15 \
    -L 127.0.0.1:<free local port>:127.0.0.1:47311 <host>
```

The host is an alias from `~/.ssh/config`, so keys, user names and jump hosts
stay in one place. A node needs no open port, no certificate and no new secret.
`kari-node serve` refuses an address that is not loopback unless the flag
`--allow-remote` names one.

A client that cannot open an SSH forward, such as a phone app, reaches a node
over a private network instead. The node record then carries an `address`
(`host:port`) and the token comes with it, from a pairing code. The node
answers on loopback for the hook relay, and on the addresses named by the
setting `listen_on`: empty for loopback only, an interface name such as
`utun5` for the private addresses of that interface, or `*` for every private
address (`kari-node serve --private`, or the picker "Let a phone reach this
machine on" in the desktop Settings). An interface name is the setting to
prefer: `*` also binds an interface a corporate VPN adds, and then that
address reaches the pairing code. A public address is never bound, so a laptop
that visits other networks stays closed there. The listener reads the address
list again every 20 seconds: a VPN interface that comes up later is bound
without a restart, one that goes away is dropped, and an address that changes
is followed.
The token is the only guard on that path, so the private network is what
carries the trust.

Each node reports the addresses it bound in its identity. A hub keeps the list
in the node record, tries the addresses in order on the next connection, and
saves the one that answered. The desktop learns a node's private address over
the SSH connection it has already, so a pairing code carries every node's
addresses and the phone types none. An address that changes costs one failed
connection, not a re-pair.

Pairing is one SSH call: the app reads `~/.config/kari/hook-token` from the
node and keeps it in the macOS keychain, one item per node under the service
`kari-node`. SSH is the authentication. The token then keeps other local
processes on the node out, the same job it does on the desktop.

The hub restarts a dead forward with a backoff from 1 s to 60 s. A node that
does not answer shows as offline, and its last board stays on the screen,
dimmed, with the time it was last seen.

### API v1

`GET /kari/health` answers without a token: node id, node name, platform,
version and `api_version`. The hub refuses a node with a different API version
and says so in Settings. Every other route needs the token in the
`x-kari-token` header:

| Route | Engine method |
|---|---|
| `GET /kari/v1/board` | `board()` |
| `GET /kari/v1/events` | server-sent events: `board_changed`, `notice`, `lease_changed` |
| `POST /kari/v1/cards` | `add_task()` |
| `PATCH`, `DELETE /kari/v1/cards/{id}` | `patch_card()`, `delete_card()` |
| `POST /kari/v1/cards/restore` | `restore_card()`: the undo of a delete, with the whole card in the body |
| `POST /kari/v1/cards/{id}/move`, `/start`, `/stop`, `/summarize`, `/jump` | the card actions |
| `GET /kari/v1/cards/{id}/jobs` | `job_log()` |
| `GET`, `PUT /kari/v1/columns`, `/settings` | columns and settings; `PUT /columns` needs the lease |
| `GET`, `POST`, `DELETE /kari/v1/lease` | the column lease: read, claim or renew, release |
| `GET /kari/v1/permissions`, `POST /kari/v1/permissions/{id}` | the permission prompts a node holds, and their answer |
| `/kari/v1/proposal`, `/proposals/{id}/accept`, `/snooze`, `/dismiss`, `/stop` | the proposal methods |
| `GET /kari/v1/quota`, `/calibration`, `/projects` | quota, calibration, projects |
| `POST /kari/v1/stop-all` | `stop_all()` |

`/kari/hook` keeps taking hook payloads, and the old `/kari/board` stays for
scripts.

### One board

The board the UI receives holds the columns of the local node, a status per
node, and every card with its node id and node name. Rules:

- A card is `(node id, card id)`. Every action routes by node.
- Each card shows a node badge. A chip row filters the board to one node.
- Columns live in the hub's store. The primary hub pushes them to each node on
  connect and on every change. A remote card that carries an unknown column
  falls back to the column that accepts its state.
- A task belongs to the host that holds its project directory. Cards do not
  move between nodes.

### The primary lease

Two hubs can watch the same nodes: the desktop and a phone. Only one pushes
columns, else they fight. The nodes arbitrate. Each node keeps one lease row:
hub id, hub name, claim time, renewal time. `POST /kari/v1/lease` claims it.
The claim succeeds when the lease is free, expired (no renewal for 10 minutes),
already this hub's, or when the claim carries `take`. `PUT /kari/v1/columns`
needs the header `x-kari-hub` with the holder's id and answers 409 to anyone
else. The node emits `lease_changed` on its event stream.

A hub that wants to be primary renews on every node about once a minute. A
refused renewal, a 409, or a `lease_changed` that names another hub drops it to
follower mode with one notice. "Make this device primary" claims with `take`
on every online node and adopts the columns it finds on the node whose foreign
lease was renewed last, so a switch changes no columns. Offline nodes get the
claim when they reconnect. No hub takes over on its own: an expired lease is
claimable without `take`, but only a user's tap claims. A desktop whose local
lease is free on start is primary, as every kari before the lease was. The
intent survives a restart in the `kv` table.

A hub without a local engine, `Hub::without_local`, shows only remote nodes.
That is the phone.

### Away mode: a permission prompt answered from the phone

Claude Code runs a `PermissionRequest` hook before it shows a permission
dialog. A command hook that prints a decision settles the prompt, and the
dialog never appears. kari uses that gap. The relay script posts the payload to
the node and, for this one event, prints the node's answer. With Away mode off
the node answers at once with no decision, and the dialog appears as before.
With Away mode on the node keeps the response open: it stores the prompt, sends
a notice with the tool and the card, and waits for `POST
/kari/v1/permissions/{id}` with `allow` or `deny`, up to `away_hold_secs` (600
by default). A hub answers through that route; the phone shows Allow and Deny
on the card. If nobody answers in time, or the session moves on, the response
carries no decision and the terminal shows the dialog. Nothing is lost.

While kari waits, the terminal shows a spinner and no dialog. So Away mode is
per node, off by default, and one tap on the phone or the desktop flips it.
Background jobs kari starts run with `bypassPermissions` and never ask. The
hook entry for this event has a 660 s timeout; an older install lacks it, so
"Install hooks" must run once more.

### Jump in

The node works out what to run and does the part that lives there, such as
focusing a herdr pane. The desktop app then opens its terminal and runs the
plan over SSH, for example
`ssh -t <host> -- sh -lc 'cd <project> && claude --resume <session>'`.

### Quota per node

Each node has its own Claude Code login, so each keeps its own windows,
calibration, backlog and proposals. The header shows one quota bar per node.
The plan panel shows every open proposal with its node name. The tray kill
switch stops kari-started jobs on every node.

The gain: a node with unused quota can work through its backlog while the
machine the user sits at is busy.

### Deployment

`kari-node serve` reads the same files as the app: the session registry, the
transcripts, the job state, the herdr socket, the status line samples. On a
host with many sessions, install both quota sources: the status line wrapper
with `kari-node statusline install`, and the usage endpoint with
`--usage-endpoint`. `kari-node hooks install` registers the hook relay, and
`--install-hooks` does it at every start, which suits a service that a
configuration manager rebuilds. Deployment of the node is managed outside this
repository.

## 15. The server

A peer-to-peer board works while every machine can reach every other machine.
That holds for one laptop and one server on one network. It stops holding as
soon as the clients outnumber the desks: a laptop that sleeps, a laptop on a
café network, and a phone on a mobile network are each unreachable from the
others, so each client draws a different board and calls the same node offline
at a different time. The always-on host is the only party everyone can reach,
and in version 2 it was a node like any other, dialled *in to* rather than
dialled *out from*.

Version 3 adds an optional server. It is the same `kari-core` engine wearing the
hub role on a host that stays up. Two things change:

- **Nodes dial out.** A node opens one outbound connection to the server and
  serves its API back down that connection. A node needs no reachable address,
  no port, and no SSH forward, so a laptop behind a hotel NAT is on the board.
- **The hub moves off the client.** The server holds the columns, the node
  registry and the last board of every node. Clients render what the server
  merged; they store nothing durable.

The server is optional and off by default. With no server configured, kari is
exactly the version-2 desktop app: a local engine, a local hub, one machine, one
account, no daemon to install. That path is not a fallback, it is the default.

### Parts

| Part | Runs where | Does |
|---|---|---|
| Node daemon `kari-node serve` | every host with Claude Code on it | The engine. Serves its API on loopback, and, when a server is configured, down one outbound link to it. |
| Server `kari-server` | one host that stays up | The hub. Accepts node links, merges the boards, owns the columns, serves the hub API to clients. Runs no Claude Code and holds no account. |
| Client | every screen: desktop app, phone | The board, the drawer, the tray, the notifications. Talks to the server, or, with no server, to its own local hub. |

The desktop app is both a client and a node: it keeps its local engine, so the
sessions on that machine are on the board whether or not the server answers.

### The link

The node opens a WebSocket to `/kari/v1/link` and holds it, reconnecting with
the same 1 s to 60 s backoff the hub used for a dead forward. Frames are JSON,
one object each:

| Frame | Direction | Carries |
|---|---|---|
| `hello` | node → server | the first frame: the protocol version and the node's identity, so the server never has to call back to learn who dialled |
| `req` | server → node | id, method, path, body: one call on the node API |
| `res` | node → server | id, status, body |
| `evt` | node → server | the events the node already publishes on `/events` |

Liveness is the WebSocket's own ping and pong rather than a frame of kari's:
the server pings every 20 s, the same cadence the SSE keepalive used, and the
node's stack answers without waking any kari code.

A `req` is dispatched into `api::router()` as a plain tower service, with no
listener in front of it. The node therefore serves one API, not two: every route
in "API v1" is reachable over the link, in the same shape, with the same
handlers. Adding a route adds it to both transports at once.

`evt` is a push, not a stream the server subscribes to, because a request and
response pair cannot carry a stream. The node forwards its own engine events up
the link as they happen, and the server turns each into the hub event it would
have made from an SSE message.

The link replaces the SSH forward, the port walk and the address list for any
node that uses a server. `kari-node serve` still binds loopback for the hook
relay, and still takes `--listen` and `--private` for a setup with no server.

### Reaching the server

The server binds a private address. Everything that talks to it — nodes and
clients alike — is expected to be on that private network, over a VPN when it is
not on the LAN. That is the same trust model version 2 gave a node on a private
address: the network carries the trust and the token keeps other processes on
the host out. The server refuses a public address unless `--allow-public` names
one, and that flag is not a supported deployment.

### Enrolment

The reverse of version-2 pairing, because the party that dials is now the node.

1. `kari-server enrol node` and `kari-server enrol client` each print a
   short-lived code. The desktop Settings shows the same codes.
2. `kari-node serve --server <url> --enrol <code>` presents the code once. The
   server records the node id and its name, issues a long-lived node token, and
   the node keeps it beside its own token. Later starts need only `--server`.
3. A client redeems a client code the same way and keeps the token in the OS
   keychain, one item per server.

Node tokens and client tokens are separate: a node token opens a link and
nothing else, a client token calls the hub API and cannot register a node. A
code is single-use and expires in 15 minutes.

### The hub API

The server serves the methods the UI already calls, which until now were Tauri
commands over an in-process hub. They become HTTP routes under
`/kari/v1/hub/`, node-scoped in the path where the hub method takes a node:

```
GET  /kari/v1/hub/board          the merged board
GET  /kari/v1/hub/events         board_changed, notice
GET  /kari/v1/hub/nodes          every node, with its status and account
GET  /kari/v1/hub/quota          the meters, grouped by account
GET  PUT /kari/v1/hub/columns    the columns the server owns
POST /kari/v1/hub/nodes/{node}/cards
POST /kari/v1/hub/nodes/{node}/cards/{card}/move|start|stop|jump|summarize
POST /kari/v1/hub/nodes/{node}/permissions/{id}
POST /kari/v1/hub/stop-all
```

To keep one UI over two arrangements, the hub methods become a trait. `Hub`
implements it in-process, as today. A new `RemoteHub` implements it by calling
the routes above. The Tauri layer and the React UI hold the trait, so neither
knows which one it has, and the single-machine path runs the code it always ran.

### One board, still

The rules from "One board" hold, with one correction each:

- A card is still `(node id, card id)`, and every action still routes by node.
  The server does the routing the client used to do.
- Columns still live in the hub's store — now the server's, so there is one
  store rather than one per client.
- A task still belongs to the host that holds its project directory. Cards do
  not move between nodes.
- A node that is offline no longer empties the board. The server serves its last
  board, dimmed, with the time it was last seen, from its own store rather than
  from a client's memory. Actions on such a card are refused with the node's
  name, not silently dropped.

### The lease is gone

The primary lease existed because two hubs could push columns to one node and
had to arbitrate. With a server there is exactly one hub, and without a server
there is exactly one too. So the lease, `x-kari-hub`, `lease_changed`,
`claim_primary` and "Make this device primary" are all removed, along with the
mode where a phone dials nodes directly.

That is the trade the server buys: multi-client is the server's job now. A setup
that today runs several hubs against one node moves to a server, or goes back to
one hub.

### Quota across nodes on one account

Quota belongs to a Claude Code login, not to a machine, and the board already
groups the meters on the account a node reports. A server makes that grouping
complete, because it is the first party that sees every node at once. Two
consequences:

- **One meter per account.** Nodes signed in to the same login show one pair of
  windows, not one pair each. A user with a work login and a personal login sees
  two rows however many machines carry them.
- **One budget per account.** Each node still runs its own planner, but a
  planner on a shared login can no longer assume the whole window is its own.
  The server publishes the account's aggregate windows and the reservations the
  other nodes on that login already hold; a planner packs against what is left,
  not against what its own samples show. Without this, two nodes on one login
  both plan to fill the same 40 percent and together overrun it.

A node the server has never seen an account for keeps a row of its own, as every
node did before kari knew about accounts.

### Deployment

The server is a second binary from the `kari-cli` crate, so it builds and ships
with the node and shares its release. It needs a writable directory for its
database and nothing else: no Claude Code, no login, no access to a transcript.
It is therefore the one part of kari that suits a container, and deploying it is
outside this repository.

### Phases

1. The link: framing, the node's outbound client, the server's acceptor, the
   node API dispatched as a service. Nodes visible on a server. Built.
2. The hub API and the `HubApi` trait. `RemoteHub` behind it. The desktop app
   configurable against a server, the local path unchanged. Built.
3. The desktop as a client *and* a node: `SplitHub` over the local engine and
   the server, and the app linking to its server the way a daemon does. Built.
   This one was missed the first time, and the app in server mode showed no
   cards of its own — see rule 2 in `AGENTS.md`.
4. Enrolment, the two token kinds, the keychain item per server.
5. The offline board of every node from the client's own store, the way the
   server already caches an absent node. The columns are cached now; the cards
   of an absent node are not.
6. Account-aggregate quota and planner reservations.

## 16. One engine per machine

A host holds the state of its own sessions, and it must hold it whether or not
a window is open. So every host runs a daemon, the desktop included: `kari-node
serve`, kept alive by the supervisor of the user's session. A laptop that is
shut for a day comes back with the cards it earned while it was shut.

The desktop app also has an engine. Both cannot run at one time, and the reason
is not tidiness:

- **One port.** The hook relay is registered in `~/.claude/settings.json` as
  one address. The second engine to start binds nothing, and the hooks reach
  whichever won.
- **One node id.** The id lives in the store the two share. A server sees one
  node dial twice and drops each link as the other arrives, so the machine
  flickers on every other client's board.
- **One budget.** Two planners on one Claude Code login each believe the window
  is theirs, and together they overrun it.

### The claim

`~/.config/kari/engine-owner.json` names the process that runs the engine: a
pid, what it is, and the port it serves. The rule is that the window wins.

| Who | On start | While running |
|---|---|---|
| The app | Takes the claim at once, then waits for the port to close | Holds it until it quits |
| The daemon | Waits for the claim to be free, however long that takes | Reads it every 3 s and exits when it names another process |

A daemon's claim is taken at once, on purpose. The daemon steps down when it
sees the claim change, so it is waiting for exactly that write, and a window
that waited for the daemon to let go first would deadlock against it. A claim
by another *window* is waited for instead, and taken after 12 seconds: two
windows are a mistake rather than a design, and a window that refused to open
would be worse than the two engines every version before the claim already
allowed.

The daemon exits rather than close its engine in place. The engine holds
watchers, a port, a link to a server and a planner, and unwinding all of that
is more machinery than a restart. `KeepAlive` on macOS and `Restart=always` on
Linux bring it back, and it comes back into the wait. That is also why the
restart interval is 10 seconds: while a window stays open the daemon restarts,
reads the claim and waits, which costs nothing.

The claim holds a pid, and a pid is checked for life before it is believed.
That is what makes a crash safe: a process that dies without clearing its claim
leaves a pid that answers nothing, and the next reader takes over.

### Installing it

`kari-node service install` writes the service file for the platform and starts
it: a `launchd` agent under `~/Library/LaunchAgents` on macOS, a `systemd --user`
unit under `~/.config/systemd/user` on Linux. It is a service of the user, never
of the system: the node reads `~/.claude`, runs Claude Code and holds that
user's login, so it must not start before the user logs in. `service uninstall`
and `service status` are the other two.

Windows has no per-user supervisor of that shape. A scheduled task with an
at-log-on trigger is the nearest thing, and the command says so rather than
write something that looks installed and is not.

## 17. Risks

| Risk | Mitigation |
|---|---|
| Transcript and registry formats are internal and change between releases | Tolerant parser, optional fields, a format test per Claude Code version, hooks as a second source |
| The OAuth usage endpoint is undocumented and rate limited | Status line is the primary source. The endpoint is a fallback with a 3-minute floor and the Claude Code user agent |
| `bypassPermissions` chosen for unattended runs | Worktree isolation by background agents, explicit `auto_run` per card, kill switch, a run log per job |
| Percent-to-token calibration is noisy | Confidence bands, conservative headroom, the planner never fills past 85 percent of a window |
| Haiku summaries spend quota | Hard throttle, heuristics fallback, off switch |
| A remote node exposes a board and a way to start jobs | Loopback bind, an SSH forward as the only transport, a token on every route, a refusal to bind a public address without a flag |
| The server is one point of failure for every board | It is optional, and a desktop client keeps its local engine, so the machine the user sits at still shows its own sessions when the server is down. Nodes keep working: a link that drops changes nothing about the sessions or the jobs on that host |
| A node that dials out reaches further than a node that only listens | The node dials one configured URL and nothing else. The server binds a private address, so the link never leaves the private network |
| One store holds every node's board and columns | The server's database is rebuildable: nodes re-send their boards on reconnect, and columns are exported as JSON like any other layout |

## 18. Decisions

Made on 2026-09-02:

- Name: kari.
- Stack: Tauri 2 with a Rust core, React and TypeScript frontend.
- Runner: Claude Code background agents (`claude --bg`).
- Default permission mode for unattended runs: `auto` since 2026-09-03 (before: `bypassPermissions`), overridable in Settings and per card.
- Scope: this Mac only. Sync later. Since 2026-09-03: remote nodes over SSH, see "Remote nodes".
- Jump in: herdr pane when present, else iTerm2.
- Scheduler: propose, the user confirms.
- Summaries: throttled Haiku.

Made on 2026-09-03:

- Remote hosts join as nodes that run `kari-node serve`. The desktop app is the hub.
- Transport: an SSH port forward to a loopback port. No new open port, no TLS, no new secret.
- The node token lives in the macOS keychain, one item per node.
- Node names are set by the user. The default is the SSH host, else the host name.
- Columns have one owner at a time, the primary hub, decided per node by a
  lease. Any hub becomes primary with one tap. A switch adopts the columns in
  place. Nothing takes the lease without a tap (2026-09-03).
- A permission prompt is held for a remote answer only in Away mode, per node,
  off by default: a held prompt hides the terminal dialog (2026-09-03).
- A node on a private network is reached by address with a pasted or scanned
  token. A laptop listens on loopback plus one chosen address, never on every
  interface (2026-09-03).
- Quota, planner and summaries stay per node, because each node has its own login.

Made on 2026-09-06:

- An optional server takes the hub role. With none configured, kari stays the
  single-machine app it was, and that path is the default rather than a fallback.
- Nodes dial the server, the server never dials a node. Reachability was the
  thing peer-to-peer could not give: a laptop that sleeps or roams is offline to
  every peer but reachable from itself.
- One outbound WebSocket per node carries the node API unchanged, dispatched
  into the same router. One API, two transports.
- The server binds a private address. A VPN, not a public endpoint, is how a
  device off the LAN reaches it.
- Separate token kinds for nodes and clients, both redeemed from a short-lived
  enrolment code.
- The primary lease is removed, and with it the phone that dialled nodes
  directly. One hub, whether that hub is a server or a desktop app.
- Quota is aggregated per account across nodes, and a planner on a shared login
  reserves against the account budget rather than its own view of the window.

Made on 2026-09-07:

- kari updates itself, rather than leaving that to whatever installed it. A
  board that spans machines is only as current as its oldest copy, and the
  system package managers that were the alternative (Homebrew, winget) update
  everything on the machine to update one app.
- The desktop app uses the Tauri updater, signed with a minisign key held
  outside the repository. Signature over checksum: the point is that only the
  holder of the key can publish an update, not merely that the download
  arrived whole.
- The app installs without asking and then offers a restart. It never restarts
  itself: unsaved input in a card would be the cost, and no update is worth it.
- Android is left to Obtainium, which already watches the same releases. Two
  installers for one app would fight.
- The headless node updates itself only when asked (`kari-node update`, or
  `serve --auto-update`). A node is installed by whatever manages its host, and
  that usually pins a version deliberately; a binary that changed itself under
  such a host would make the pin a lie.
- The node replaces one file rather than unpacking an archive, so each release
  carries the bare binary and its checksum beside the tarball and the zip.
