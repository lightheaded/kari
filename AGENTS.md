# Agent instructions

kari (Estonian: herd) turns Claude Code sessions into cards on a Kanban board.
`README.md` says what it does. `DESIGN.md` says how it is built.
`CONTRIBUTING.md` holds the development commands, the checks and the release
process. Read those first.

This file holds what kari is, and the rules that are easy to get wrong and
expensive to correct after a push.

## What kari is, and what must stay true

kari is a state store with a board on it. It is not a view of live
connections. Every session kari has seen keeps its card and its last known
state. A host that sleeps, that is off the network, or that has no window open
keeps its cards on the board.

Four rules follow. A change that breaks one is wrong, however good the rest of
it is.

1. **The sessions of this machine are on its board before any network
   answers.** The app opens and the local cards are there. This needs no
   server, no pairing and no reachable peer.
2. **A new way to share state adds a source. It never replaces one.** A server
   carries state between devices. It never becomes the only party that holds
   the state. If the new path is down, the old path must still answer.
3. **No node needs to reach another node.** State moves through the store on
   each host, and through a server when one is configured. kari never needs
   every machine to be reachable at one time.
4. **A daemon watches each host, whether or not a window is open.** State
   accrues while nobody looks at it. A window is a client of that daemon, and
   never a second copy of it.

kari must also work as soon as it is installed, with nothing configured. The
server, the other nodes, the hooks and the status line are each optional. The
default path is not a fallback from a better arrangement. It is the
arrangement.

### How these rules were broken once

Version 3 moved the hub to a server, so that clients with no route to each
other could read one board. The change also made the desktop app a pure client
of that server: `open_hub` returned a remote hub, and the local engine left the
board. A Mac with a server configured then showed no cards of its own, and a
new session on that Mac appeared nowhere.

Every step of that change had a reason, and together they broke rules 1 and 2.
The reason in the commit message was that a fallback to the local hub would
"quietly restore one-hub-per-client". That treats the local node as a fallback.
The local node is the base.

So before you centralize anything, name the rule above that the change touches.
Then say how the local path still answers while the central part is down.

## The commit message is the release note

`scripts/release-notes.sh` builds the GitHub release page from git history.
There is no separate release-notes step, and nobody writes the notes
afterwards. A weak commit message is a missing release note.

- The commit **subject** becomes one line of the `## TL;DR` list.
- The commit **body** becomes one section of `## What changed`, under a
  heading that is the subject.
- The body of a `Release X.Y.Z` commit becomes the headline of the release.
- Merge commits and `Release X.Y.Z` commits stay out of both lists.
- The `Co-Authored-By:`, `Claude-Session:` and `Signed-off-by:` trailers are
  removed before publication.

So write every commit message for a person who runs kari and wants to know
what changed and whether to install it.

**The subject** must stand alone in a list of changes. Say what the change
does, on one line, in the imperative. Do not add a type prefix or a ticket
number. `Let a device be pointed at a server from its own screen` is a
subject. `fix bug`, `wip`, `update files` and `address review comments` are
not.

**The body** must explain the change to a reader who was not there. Say what
the problem was, what the change does, and why it is done this way instead of
the obvious way. Name the failure that a reader would otherwise meet. Write it
in Simplified Technical English: short sentences, active voice, and the
condition before the command.

A commit with a one-line body, or with no body, leaves its section of the
release page empty.

`fix bug` and `address review comments` are acceptable inside a branch. They
must not reach `main` as the last word on a change. Squash or reword first.

Read what a release will say at any time:

```
scripts/release-notes.sh
```

## A squash merge writes its own body, and it is not a release note

GitHub fills the body of a squash commit with a bullet list of the commits it
squashed: `* <subject>`, then that commit's body, once for each one. That list
is not a release note. It buries the real description under a bullet, and if
the branch carried a commit from another pull request, the release page
repeats a change that an earlier release already described.

Release 0.7.3 shows the failure. Its page carried the whole body of an
unrelated earlier change inside the section of the change being merged, so
the page read as one section where there were two.

`scripts/release-notes.sh` now takes such a bullet list apart. The bullet
that names the commit becomes its body, and a bullet that names another
change in the same release is dropped, because that change writes its own
section. That repairs the 0.7.3 shape, and it is all a script can do: when
several bullets describe this one change, it keeps them all and warns,
because only a person can write one description out of several.

So when you squash a pull request, delete the body that GitHub offers and
write the body of the change. One subject and one body, as above. Read the
preview and act on the warning.

## A commit message is public

kari is a public repository. Every commit body now also reaches a public
release page, where more people read it than read the history.

Never put these in a commit message, a pull request or a release:

- An absolute home path. Use `~`, or a placeholder such as `/Users/you/`.
- An email address, a personal account name or a session id.
- A private network address or an internal host name.
- The name of a host, a repository or a tool that is managed outside this
  repository, or any description of how that estate is arranged. Write
  "managed outside this repository" and nothing more. A maintainer who needs
  the detail reads it where the thing is owned.

`scripts/check-privacy.sh` runs in CI over the tracked files, and the release
workflow runs it again over the generated notes. A hit fails the release
instead of publishing it.

The check is a net, not a proof. It finds a home path, an address and an
internal host name by pattern. It cannot find a machine that carries a name
only its owner knows. The rule above is the control. The script is the
backstop.

A pushed commit message is public, and a force push does not take it back:
the old commit stays readable by its hash, and the push event already carries
the message. Write it correctly the first time.

## Before you open a pull request

Run the checks under Development in `CONTRIBUTING.md`. CI runs the same ones.
