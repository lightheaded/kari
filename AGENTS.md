# Agent instructions

kari (Estonian: herd) turns Claude Code sessions into cards on a Kanban board.
`README.md` says what it does. `DESIGN.md` says how it is built.
`CONTRIBUTING.md` holds the development commands, the checks and the release
process. Read those first.

This file holds the two rules that are easy to get wrong and expensive to
correct after a push.

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
unrelated earlier change inside the section of the change being merged.

So when you squash a pull request, delete the body that GitHub offers and
write the body of the change. One subject and one body, as above.
`scripts/release-notes.sh` warns about a body that starts with a bullet, but
only a person reading the preview can act on it.

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
