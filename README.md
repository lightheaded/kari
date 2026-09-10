# kari

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/screenshots/board-dark.png">
    <img src="docs/screenshots/board.png" alt="The kari board: one quota row per account at the top, six columns from Backlog to Done, cards with state chips, summaries and an open question" width="960">
  </picture>
</p>

kari (Estonian: herd) is a tray app for macOS and Windows that shows every Claude Code session as a card on a Kanban board. The board updates itself from local Claude Code state. Backlog tasks can start as background sessions when quota is left over. More than one machine can share the board: a Linux or macOS host runs the headless node, and the app reaches it over SSH.

kari works with the tools you already use. It reads what Claude Code writes to disk and never writes there. When [herdr](https://github.com/herdrdev/herdr) runs, kari maps sessions to herdr panes and can open new sessions in them.

The screenshot above shows the current release with a demo board. `TOUR.md` walks through every view. Design: see `DESIGN.md`.

## Install

### macOS

1. Download the `.dmg` for your Mac from the [latest release](https://github.com/lightheaded/kari/releases/latest): `aarch64` for Apple silicon, `x64` for Intel.
2. Open the image and drag kari to Applications.
3. The app is not signed. macOS will refuse to open it until you remove the quarantine flag:

```
xattr -dr com.apple.quarantine /Applications/kari.app
```

4. Start kari. It appears in the menu bar and reads your Claude Code state.
5. Optional: run `scripts/install-statusline.sh` for quota meters, and click "Install hooks" in Settings for live state. Both are described below.

### Windows

1. Download `kari_<version>_x64-setup.exe` from the [latest release](https://github.com/lightheaded/kari/releases/latest).
2. Run it. The installer is not signed, so SmartScreen asks once: More info, then Run anyway.
3. It installs for the current user, not for the machine. That is deliberate: kari reads the Claude Code state of whoever is logged in, so an all-users install would put the app in front of a profile it cannot read.
4. Start kari. It appears in the notification area and reads your Claude Code state.
5. Optional: click "Install hooks" in Settings for live state, and run `kari.exe statusline install` once from the install directory for quota meters. Both register the app binary itself with a subcommand, because Windows has no `sh` to run the wrapper script that macOS uses.

Two things a Mac has and Windows does not, both by construction:

- **"Jump in" opens nothing locally.** The terminal driver is AppleScript, and there is no Windows equivalent yet. Cards, state, quota meters and background jobs all work; a card on a *remote* node still jumps in over SSH.
- **herdr pane mapping is absent.** herdr talks over a Unix socket.

kari keeps itself current after that on both platforms. See "Updates".

## What works

- Cards appear for every session with a transcript in the history window, and for every live process.
- State comes from the live session registry, transcripts, background jobs and herdr. Columns accept sets of states and are configurable.
- Manual moves hold until a stronger signal arrives. Decision and approval signals always win.
- Jump in focuses the herdr pane, or opens a terminal window with `claude --resume`. iTerm2, Terminal and Ghostty are supported.
- Backlog tasks can start as `claude --bg` jobs from the card drawer.
- Quota meters read `rate_limits` from the Claude Code status line through a wrapper.
- Live hooks: Claude Code posts session events to kari on `127.0.0.1:47311`. A permission prompt moves the card to Approval needed within a second.
- Summaries: Haiku writes a two-sentence narrative per session after a turn ends. Calls are capped per hour.
- Estimates: kari learns how much of the 5-hour window one million weighted tokens costs, then gives each card a cost estimate with a band.
- Proposals: when quota expires unused, kari offers a plan. Start, Start all, Snooze or Dismiss.
- Model per card: a task can name the model it runs with, for example Fable for a deep review and Sonnet for operational work.
- Attachments: paste a screenshot into a card or pick a file, and every run of that card reads it. See "Attachments".
- Run log and kill switch: every background job kari starts writes a state history on its card. The tray stops all jobs after a confirm click.
- Booked runs: a card can start at a time you pick, such as after the rate limit resets, or one cycle later. The booking ignores the automation mode.
- One switch for the automatic behaviour: Off, Ask or Auto. Off keeps the quota for you. Auto starts a weekly-reset plan by itself, with a notice and a Stop button.
- A queue strip that names the next runs in order, the cost of each step, and its start time. It starts nothing.
- Remote nodes: another host runs `kari-node serve` and its cards join the same board; quota meters are grouped by Claude Code account. See "Remote nodes".

## Requirements

- macOS 12 or later, Claude Code 2.1 or later, `jq`.
- Or Windows 10 1809 or later, with the WebView2 runtime, which Windows 11 already has. `jq` is not needed there: nothing on the Windows path shells out to it.
- Optional: herdr 0.8 or later for pane mapping.
- For a remote node: an SSH login on that host, plus `kari-node` and Claude Code there.
- To build from source: Rust toolchain (`rustup`), Bun.

## Build from source

```
bun install
bun tauri dev
```

Build a bundle:

```
bun tauri build
```

Build the Android app. It needs JDK 17, the Android SDK with `platforms;android-34`, `platforms;android-36`, `build-tools;34.0.0`, `build-tools;35.0.0` and `ndk;27.2.12479018`, and the Rust target `aarch64-linux-android`. Point `JAVA_HOME`, `ANDROID_HOME` and `NDK_HOME` at them.

```
bun tauri android init                                   # once; the project it writes is not tracked
scripts/android-icons.sh                                 # after every init; init writes the Tauri logo
bun tauri android build --debug --apk --target aarch64   # a debug APK for adb install
bun tauri android build --apk --target aarch64           # an unsigned release APK; the release workflow signs it
```

The launcher icon comes from `src-tauri/icons-src`. `icon.json` names the three
adaptive layers: the green plate, the horned glyph, and the monochrome glyph for
themed icons. Edit the SVG files, then run `scripts/android-icons.sh` again. The
script leaves the tracked desktop icons in `src-tauri/icons` as they are.

## Quota tracking

The status line wrapper saves the rate-limit windows on every refresh:

```
scripts/install-statusline.sh
```

The script backs up `~/.claude/settings.json`, stores the original command in `~/.config/kari/statusline.original`, and writes samples to `~/.config/kari/rate-limits.json`. Run the script with `--uninstall` to restore the original command. New Claude Code sessions pick up the wrapper. Running sessions keep the old command until they restart.

## Live hooks

Open Settings and click "Install hooks". kari writes a relay script to `~/.config/kari/hook.sh` (on Windows it registers `kari-node.exe hooks relay` instead, which needs no shell) and registers it in `~/.claude/settings.json` for these events: SessionStart, SessionEnd, UserPromptSubmit, Stop, Notification, PreToolUse (AskUserQuestion and ExitPlanMode), PostToolUse. kari keeps a backup of the settings file in `~/.config/kari/`. The relay posts the payload with `curl` and always exits 0, so a closed kari never blocks a session. Every post carries a token from `~/.config/kari/hook-token` in the `x-kari-token` header. kari creates the token on first start with mode 0600 and refuses a post without it. A process that runs as your user can read the file, so the token keeps out other users, sandboxed apps and web pages, not your own processes. New sessions pick up the hooks. Running sessions keep the old settings until they restart. "Remove hooks" takes the entries out again and leaves other hooks in place.

The receiver also serves `GET /kari/board` (the board as JSON) and `GET /kari/health` for scripts. The board needs the same token:

```
curl -H "x-kari-token: $(cat ~/.config/kari/hook-token)" 127.0.0.1:47311/kari/board
```

## Summaries

After a turn ends, kari runs `claude -p --model haiku` with the last 30 messages of the transcript, plus every `git` and `gh` command the session ran between them, and stores the result: narrative, delivery (how far the work travelled: committed, pushed, PR open, merged, released, deployed, CI passed or failed), open questions, next step, judged state, confidence. The call uses `--no-session-persistence`, no tools, no MCP servers and no settings, so it runs no hooks and leaves no transcript. A confident judgment can move a quiet session to Waiting on others or Validate. It never overrides a hard signal such as a pending question or a busy process. Limits live in Settings: on or off, model, calls per hour (default 6), and the recent window (default 48 hours). "Summarize" in the card drawer makes one call outside the cap.

## Estimates and calibration

Rate limits are percentages. Transcripts are tokens. kari pairs the points where the reported 5-hour percent changed with the token growth it saw between them, then keeps the median. Five clean pairs replace the prior. The learned factor and the band appear in the tooltip of the quota row.

A task card estimate is the median weighted-token cost of past sessions in the same project, else the global median. A session card estimate is the cost of eight more turns at that session's own rate. Set `estimate_weighted_tokens` on a card to override it.

Weighted tokens count output five times, cache writes 1.25 times and cache reads a tenth, so the number tracks cost, not volume.

If no session refreshes the status line for 5 minutes, kari can ask the undocumented OAuth usage endpoint instead. That fallback is off by default. It reads the Claude Code login token from the keychain, asks at most once every 3 minutes with a `kari/<version>` user agent, and never writes the token to a log.

## Proposals

kari offers a plan when one of three things happens:

1. The 7-day window holds more than 40 percent unused and resets within 36 hours.
2. The 5-hour window is below 30 percent and nobody worked for 45 minutes.
3. You press "Fill" on an account's quota row. The plan runs on the first machine on that row.

The planner ranks cards by priority, then by age. A drag on the board writes that priority, so the top of a hand-ordered backlog is the first card a plan takes. It only takes cards marked "May run unattended" that carry a prompt and a project directory. It never takes a card that is working, or one that waits for you. It packs the plan into the free part of the 5-hour window, keeps 30 percent free between 08:00 and 20:00, never fills past 85 percent, and holds the parallel cap of two jobs.

The panel shows the reason and a budget bar. The bar is the 5-hour window. The gray part is what is used now. Each picked task adds a green segment. A black line marks the end of the budget. A segment past the line turns amber. The cards that did not fit are listed under a divider. Pick one to start it anyway. The Start button turns amber when the picked tasks go over the budget. Buttons: Start, Start all, Snooze 1 hour, Dismiss. Dismiss keeps the same trigger quiet until its window moves on. Every threshold is in Settings.

## Runs, run log and the kill switch

A started card runs as `claude --bg` with the permission mode of the card, or the default from Settings. New installs default to `auto`. Choose `bypassPermissions` there only if you accept that an unattended run has no permission checks in the project directory.

Each card can also name a model. The New task dialog and the card drawer offer Fable, Opus, Sonnet and Haiku, and "Default" leaves the choice to Claude Code. kari passes the name as `--model` on every start: background runs, Jump in, and a herdr pane. Settings holds the default for cards that name none. A card that names a model shows it as a chip, and the plan panel shows it per task. kari follows the job and writes one run-log line per state change. The card drawer shows the log. The job outcome stays on the card after `claude agents` forgets the job, so a finished job leaves the card in Validate and a failed job leaves it in My turn.

The prompt box in the card drawer gives a card its next prompt. A session that runs on the node takes the prompt into its own queue, over the message socket every Claude Code process opens, so the terminal you look at answers it. A session that is not running resumes as a background job with the prompt, and a task starts one. kari never resumes a running session a second time: Claude Code would start a copy, and the prompt would land in a transcript nobody is looking at.

A running session decides for itself whether it takes the prompt at once. Its inbox compares the permission mode of the sender with its own, and holds a message from the other class until the person at that terminal releases it. So kari names itself and states the mode it runs the card under, and the toast then says what happened: "Sent to the running session", or "Held for approval in that session". A prompt that waits for someone's click no longer looks like a prompt that arrived.

Stop one job from the card drawer. Stop everything from the tray: the first click arms the item, the second click within 10 seconds stops the jobs. The tray tooltip shows how many sessions work and how many need you.

## Booked runs

A plan waits for a trigger. A booked run waits for a time.

Press "Schedule" in the card drawer when the 5-hour window has no room left. The block offers three cycles and one free time:

| Choice | When the run starts |
|---|---|
| When the window resets | At the next reset of the 5-hour window, plus two minutes. |
| The cycle after that | One 5-hour window later. |
| When the week resets | At the next reset of the 7-day window, plus two minutes. |
| A time you pick | At that time. |

Each button carries the time it would book. The times come from the rate-limit sample of the node that holds the card, because the windows belong to the Claude Code account that node is signed in to. A node with no sample cannot offer a cycle. Install the status line wrapper, or pick a time.

A booked run is a manual start with a delay. The automation mode, the budget and the planner do not gate it, and a card holds one booking at a time. The one-off prompt field in the drawer goes with the booking, so you can book a follow-up on a session card instead of a fresh run.

The card shows a clock chip with the time. The queue strip lists the booking first, with a clock in place of the step number. Cancel it in the drawer.

Four rules end a booking:

- The run starts. kari clears the booking and sends a notice.
- The run cannot start, for example because the project directory is gone. kari clears the booking and says why.
- The card is busy at its time, or every job slot is full. kari holds the booking and looks again every 15 seconds.
- The booked time passed more than 24 hours ago. kari clears the booking and says so. A host that slept through the reset still runs the card when it wakes. A host that slept for a day does not.

## Attachments

A prompt sometimes needs a picture. Paste a screenshot into the prompt box of
the card drawer, or press Attach and pick a file. The New task dialog takes
files the same way.

The run reads them. kari adds the paths under the prompt, so `claude` opens the
files as part of the task. That is the reason the bytes live on the node that
owns the card: Claude Code reads a file from a path on the host that runs the
session. With a server, the server passes the file to that node and keeps no
copy. With no server, nothing changes at all.

One file is at most 4 MB. A card shows a paperclip with the count, and the
drawer shows a thumbnail of each picture.

The files sit outside the project, so kari starts the run with `--add-dir` on
their directory. Without that the default permission mode refuses the file and
an unattended job would wait for an answer that never comes. A session that
already runs cannot be given the flag, so it asks you the first time it opens
an attached file.

A move to another node takes the files with the card. If a file cannot be read
from the node it is on, the card does not move and kari says which file.

kari removes the files when it no longer needs them:

- An archive removes them at once.
- A card marked done keeps them for the days set by "Keep attachments after
  done" in Settings (7 by default).
- A deleted card keeps them for a day, so that the undo on the toast gives the
  card back its files.

## The automation switch

One control in the top bar holds three states:

| Mode | What happens |
|---|---|
| Off | No plans and no starts. The quota is yours. |
| Ask | kari offers a plan. You press Start. |
| Auto | A weekly-reset plan starts without a click, up to the autopilot job cap. |

Ask is the default. In Auto, kari still sends a notice, and the plan panel keeps "Stop these jobs" as the undo. The idle trigger and the manual button always wait for a click.

An autopilot run can do the work and open a pull request. It cannot release.
kari refuses a merge, a tag, a release, an image push, and a commit in a
directory where a commit deploys. The run reads why it was stopped, and the
card shows the command it tried. Name your deploy directories in
`autopilot_protected_paths`; the list is empty until you fill it. A session
that you start yourself is never held back.

The mode belongs to a node, because every node runs its own planner. The switch sets every node that answers at once. With a node filter on, it sets that node only. Settings holds the mode of the local machine on its own.

## The queue

The strip under the filter bar is a dry run of the planner, with the booked runs in front of it. It names each step in order: the card, the cost as a percent of the 5-hour window, the state of the window after it, and the start time. A booked run carries a clock in place of its number, and a run booked after the reset counts against an empty window. A step outside the budget says so. If nothing can run at all, the strip gives the reason: the mode is off, no quota sample arrived, every job slot is busy, the budget is too small, or no card is marked "May run unattended".

The strip starts nothing. The plan panel keeps the buttons.

## herdr as a launch target

When herdr runs and "Open new sessions in a herdr pane" is on, Jump in creates a herdr tab in the project directory and starts Claude Code in its pane, with `--resume` for a session card. If herdr refuses, kari falls back to the terminal. An existing pane is always focused instead.

Settings can also close the tab again. When "Close the herdr tab when a card is done or archived" is on, kari closes the tab of the pane that holds the card as soon as you move the card to Done or archive it. The tab closes on the machine that runs the session, and it closes even when its agent still works. Undo puts the card back, never the tab. The option is off by default.

## Remote nodes

A second machine can join the board. The app stays the only window; the other host runs the headless node.

On the other host:

```
kari-node serve                 # the engine and the API on 127.0.0.1:47311
kari-node hooks install         # live session events (optional)
kari-node statusline install    # quota meters (optional)
```

In the app, open Settings, Nodes, and add the host. Give the SSH host, which is an alias from `~/.ssh/config`. kari opens an SSH port forward to the node's loopback port and reads the node's token once over the same connection. The token goes to the macOS keychain, and the forward comes back by itself after a network break.

What this needs on the other host: an SSH login and a Claude Code that is logged in. It needs no open port, no certificate and no new secret. The node refuses to bind an address that is not loopback unless you pass `--allow-remote`.

What you see: every card carries a node badge, and a chip row filters the board to one node. Each node keeps its own backlog and plans. Quota is grouped by Claude Code account rather than by node, so two machines signed in to the same login share one row of meters — see "Quota belongs to the account". Jump in on a remote card opens your terminal and runs `ssh -t <host> ... claude --resume <session>`. The tray kill switch stops jobs on every node.

Deployment of the node is managed outside this repository. `flake.nix` builds it for a NixOS host, and each release carries a Linux tarball and a Windows zip.

A node can also replace itself:

```
kari-node update --check        # say what is out, change nothing
kari-node update                # fetch the newest release over this binary
kari-node serve --auto-update   # and keep doing it while it serves
```

`update` fetches the binary built for this host from the newest release, checks it against the checksum published beside it, and renames it over the running one. The running process keeps the old code either way, so the new version starts on the next run. `--auto-update` therefore stops the node once it has written a new binary, and the unit must bring it back: systemd needs `Restart=always`, because `Restart=on-failure` treats that clean exit as the end and leaves the host with no node. It checks a minute after start and every six hours after that (`--update-every-hours`).

A Mac fetches its node from the release like every other platform. Every other host is installed by whatever manages it.

This is off unless you ask for it. A node is installed by whatever manages its host, and that usually pins a version on purpose — a binary that changed itself under such a host would make the pin a lie. It also needs to be able to write to its own directory, which rules out a binary in the Nix store; there, update the host instead.

## A server, when the nodes cannot be reached

Adding nodes one by one works while every machine can reach every other one. It
stops working as soon as the clients outnumber the desks: a laptop that sleeps,
a laptop on someone else's network and a phone on mobile data are each
unreachable from the others, so each client draws a different board and calls
the same node offline at a different time.

A server turns the connection around. It runs on a host that stays up, the nodes
dial *it*, and it calls their APIs back down the sockets they opened. A node then
needs no reachable address, no open port and no SSH forward.

On the server's host:

```
kari-server serve            # accepts links on 0.0.0.0:47312
kari-server token            # the token a node needs; copy it to the node
kari-server nodes            # the roster, as JSON
```

On each node:

```
kari-node serve --server http://the-server:47312
```

The token reaches the node in `KARI_SERVER_TOKEN`, in `--server-token-file`, or
in `~/.config/kari/server-token`. The link is additional, never instead: the node
keeps serving loopback for the hook relay whether or not the server answers, so
nothing about the sessions or the jobs on that host depends on it. A dropped link
comes back by itself, backing off from 1 s to 60 s.

The server binds a private address and refuses a public one unless you pass
`--allow-public`. It checks one token on every route, and those routes can start
jobs, so put it on a private network or behind a VPN rather than on the internet.
It runs no Claude Code, holds no login and reads no transcript, so it suits a
container; `flake.nix` builds one as `kari-server-image`.

The app keeps its own board when it has a server. The sessions of the machine
you are looking at come from the engine in the app, so they are there whether or
not the server answers, and the server adds every other host to them. The
columns come from the server, because there is one set for every client, and the
last set it published is cached for the times it is away.

Cards of an *absent* node are the part that is still missing: the server holds
them, so they arrive while the server answers and go when it does not. See "The
server" in `DESIGN.md`.

## A daemon on every host

kari holds the state of a session whether or not a window is open, so every
host runs the node as a service of your user — the Mac you sit at included:

```
kari-node service install                       # run at login, and now
kari-node service install --server http://…     # and link it to a server
kari-node service status
kari-node service uninstall
```

On macOS that is a `launchd` agent in `~/Library/LaunchAgents`, and its log is
`~/.config/kari/node.log`. On Linux it is a `systemd --user` unit, and
`journalctl --user -u kari-node` reads it. Windows has none of this yet: run
`kari-node serve` from a scheduled task with an at-log-on trigger.

**One engine runs on a host at a time, and the window wins.** Open the app and
the daemon steps down within a few seconds; quit the app and the daemon takes
the host back. Neither needs to be told: a file in `~/.config/kari` names the
process that holds the engine, and the daemon reads it. Two engines on one host
would bind one hook port, share one node id and plan against one login, so this
is a rule rather than a preference. See "One engine per machine" in `DESIGN.md`.

### Quota belongs to the account

The 5-hour and 7-day windows belong to a Claude Code login, not to a machine. Two nodes signed in to the same account draw down one window, so the header shows one row per account and names the machines that spend it. Two rows would read as two budgets, and the planner would go after quota that is already spent.

kari reads the account from what Claude Code writes on login and groups on its id. A node whose account cannot be read — one running an older kari, or one not logged in — keeps a row of its own. That is the safe answer: an unknown account is never merged with another, because merging two budgets by guessing is the mistake that costs you a window.

Click the name on a row to give the account one of your own, such as `tom` or `work`. The name lives on this device, next to the board; it is never sent to a node and never touches the Claude account. Clear the field to go back to the name on the account.

"Fill" plans a run on the first machine on the row.

### A node on Windows

The node runs on Windows as well. Unpack `kari-node-<tag>-x86_64-pc-windows-msvc.zip` and run the same three commands. Two things differ, and both are handled for you:

- There is no `sh` and no `jq`, so `hooks install` and `statusline install` register `kari-node.exe` itself rather than a script. Keep the binary where it is: the path goes into `settings.json`, and moving it breaks the hooks until you install them again. Run the installers again after you move or upgrade it.
- herdr is a Unix program, so pane mapping and "Jump in" are not available. Cards, state, quota meters and background jobs all work; "Jump in" is offered from the desktop app over SSH.

To keep the node running without a console window, wrap it in a Windows service (`WinSW` and `NSSM` both do this) or start it from Task Scheduler at logon. Run it as the user whose Claude Code login it should read: the node reads `%USERPROFILE%\.claude`, and a service running as `LocalSystem` sees a different one.

For the app to reach it, Windows needs an SSH server (Settings, Optional features, "OpenSSH Server") or a private network address and `--private`.

### A node over a private network

A hub that cannot open an SSH forward, such as the phone, reaches a node by address. The node then answers on loopback for the hook relay and on the private addresses of the machine as well:

```
kari-node serve --private
```

The desktop app has a picker in Settings, Nodes: "Let a phone reach this machine on". Pick the VPN, and kari answers on that address. The choice is kept as a network in CIDR form, so a tunnel that comes back under another interface name still counts. An interface name works as well. `every private address` is the wide setting, and it binds every private interface, including one a corporate VPN adds, so name an interface where you can. A public address is never bound. The list is read again every 20 seconds, so a VPN that comes up later needs no restart, and an address that changes is followed. The token is the only guard on that path, so the private network carries the trust. To name one address instead, use `kari-node serve --listen 127.0.0.1:47311 --listen <vpn-ip>:47311 --allow-remote`.

Each node reports the addresses it answers on. The desktop learns them over the SSH connection it has already, and puts them in the pairing code, so the phone types no address.

## The phone

The same app builds for Android and runs as a second hub. It joins the private network, talks to every node directly, and shows the board, a "Needs you" inbox, the plans, and a task form. It runs no Claude Code, so it has no node of its own.

- Install `kari-latest.apk` from the release page, or add `https://github.com/lightheaded/kari` to Obtainium: the asset name never carries the version and the signing key never changes, so an update installs over the previous build. Then open Nodes and paste the pairing code from the desktop (Settings, Nodes, "Show pairing code"), and press "Add". The code carries each node's name, token and addresses, so there is nothing to type. The code holds the node tokens: show it at home and hide it when done. Set "Let a phone reach this machine on" to the VPN interface on the desktop first, else its own address is missing from the code. On the phone, the VPN app must also carry kari itself: a per-application tunnel that does not list kari sends its traffic around the tunnel, and every node then times out.
- "Needs you" lists every card in approval, decision, my turn, validate and waiting, with the actions on the card: an option of an open question, a reply, stop, done. A reply to a session that is alive in a terminal gets a warning first, because a second process writes into the same transcript.
- Notifications arrive while the app is open. Android stops the app in the background after a while; a foreground service is a later step.

### Who owns the columns

Two hubs can watch the same nodes. Only one pushes columns, and each node decides which: it keeps a lease. "Make this device primary", in Settings on the desktop or in Nodes on the phone, takes the lease on every online node and adopts the columns found there, so a switch changes no columns. The other hub follows, with its columns editor read-only. Nothing takes the lease without a tap. Every other action, such as add, move, start, stop, reply, allow and deny, stays open to every hub.

### Away mode

Claude Code asks for permission in the terminal, and only the terminal can answer. With Away mode on for a node, kari holds the prompt for up to 10 minutes and the phone shows Allow and Deny on the card. While kari waits, the terminal shows a spinner and no dialog, so Away mode is off at the desk and one tap flips it, per node, from the phone or the desktop. If nobody answers in time, the dialog appears as before. Background jobs kari starts never ask. Away mode needs the `PermissionRequest` hook entry: click "Install hooks" once more after an upgrade, or run `kari-node hooks install` on a node.

## Updates

kari installs new releases itself. It looks when it starts and every six hours, downloads the release, writes it beside the running app and says so in a toast with a Restart button. Nothing changes under you: replacing the bundle does not replace the running process, so the version you are using stays the one you opened until you restart. Settings, Updates has the switch, the current version and a "Check now" button.

Every release is signed with a key that is not in this repository, and kari installs an update only if the signature matches the public key built into it. So an update can only come from whoever holds that key, wherever the download came from.

The phone is the exception: it installs through Obtainium, which watches the same releases, so the app carries no updater on Android. A headless node has its own, off by default — see "Remote nodes".

## Data

- Database: `~/.config/kari/kari.db` (SQLite). It also holds the remote nodes, the column lease and the primary intent. Node tokens live in the keychain, never in the database. On the phone the store lives in the app's data directory.
- Attachments: `~/.config/kari/attachments/<card id>/`, one directory per card, on the node that owns the card. kari sweeps them; see "Attachments".
- kari reads `~/.claude/sessions`, `~/.claude/projects`, `~/.claude/jobs` and the herdr socket. It never writes to those directories. The hook and status line installers edit `~/.claude/settings.json` only when you ask, and keep a backup.

## Layout

```
crates/kari-core   readers, parser, inference, store, launcher, HTTP API, hub, SSH tunnel
crates/kari-cli    kari-node: the engine without a window; kari-server: the hub on an always-on host
src/mobile         the phone layout: inbox, one column at a time, nodes and pairing
src-tauri          Tauri shell: commands, events, tray, notifications
src                React UI
scripts            status line installer, version bump, demo board, screenshots
docs/demo          the dummy board behind the screenshots
docs/screenshots   the images in this README and in TOUR.md, retaken for each release
```

Smoke test the core against local data without the UI:

```
cargo run -p kari-core --example board        # the board as text
cargo run -p kari-core --example board -- --json > fixtures/board.json   # snapshot for `bun run dev` in a browser
cargo run -p kari-core --example estimates    # calibration and per-card estimates
cargo run -p kari-core --example plan -- /tmp # build a plan from temporary cards
cargo run -p kari-core --example jobrun -- /tmp   # start one background job and follow it
cargo run -p kari-core --example herdr_open -- /tmp   # open and close a herdr pane
cargo run -p kari-core --example node_api     # serve the local engine and call it as a node
cargo run -p kari-core --example hub_node     # add a real node to the hub, read both boards, take the lease, remove it
cargo run -p kari-core --example link_server  # link a real node to a real server and read its board over the link
cargo run -p kari-core --example one_engine   # a daemon holds the engine, a window takes it, the daemon steps down
cargo run -p kari-core --example permission_hold  # hold a permission prompt on a real node and answer it
```

The `plan`, `jobrun` and `herdr_open` examples create temporary cards, tabs or jobs and clean up after themselves. Give them a scratch directory, never a real project. `hub_node`, `permission_hold`, `link_server` and `one_engine` start a node — and `link_server` a server too — with their own home directories and remove them again; they need `cargo build -p kari-cli` first.

`bun run demo` opens the UI in a browser with the dummy board from `docs/demo`. No Rust toolchain and no Claude Code state are needed for that.

## Contributing

See `CONTRIBUTING.md`. Issues that describe a real session and what you expected are the most useful input.

## License

Apache License 2.0. See `LICENSE`.
