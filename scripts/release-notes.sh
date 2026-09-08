#!/bin/bash
# Write the GitHub release body for a tag to stdout.
#
# Usage: scripts/release-notes.sh v0.7.3 [previous-tag]
#        scripts/release-notes.sh              # preview the pending release from HEAD
#
# The notes come from git history. Commit subjects since the previous release tag
# become the TL;DR, and commit bodies become the "What changed" section. There is
# no separate release-notes step: the commit message IS the release note. See
# CONTRIBUTING.md, Releases.
#
# The `Release X.Y.Z` commit is not a change. Its body is the headline of the
# release, and it is left out of the two lists below.
#
# A squash body is taken apart rather than printed as it stands: see unsquash.
#
# Preview the notes before you tag. Every commit body goes onto a public page,
# so text that names one machine or one person must not be in a commit message
# in the first place. The release workflow runs check-privacy.sh over the result.
set -euo pipefail
cd "$(dirname "$0")/.."

tag="${1:-v$(jq -r .version src-tauri/tauri.conf.json)}"

# Preview from HEAD before the tag exists, and from the tag after it does.
if git rev-parse -q --verify "refs/tags/$tag" >/dev/null; then
  ref="refs/tags/$tag"
else
  ref=HEAD
fi

# Peeled to the commit, because kari's tags are annotated and signed. A bare
# `git rev-parse refs/tags/v0.7.3` answers with the tag OBJECT, so the header
# named a hash that is not a commit and that `git show` cannot usefully take.
# It read as correct, because it is a hash of the right length.
ref=$(git rev-parse "$ref^{commit}")

# The previous release tag, taken from the parent so that `$tag` cannot describe
# itself. A tag can be unreachable for an ordinary reason, because a force push
# can leave the last one off this history, so this degrades rather than breaks.
# The unbounded fallback is wrong on its own terms: the whole history is not a
# release note.
prev="${2:-$(git describe --tags --abbrev=0 --match 'v[0-9]*' "$ref^" 2>/dev/null || true)}"
if [ -n "$prev" ]; then
  range=("$prev..$ref")
  echo "release-notes: describing $prev..$tag" >&2
else
  range=(--max-count=30 "$ref")
  echo "release-notes: no earlier v* tag is reachable. Describing the last 30 commits." >&2
fi

# Useful in git history, not useful to somebody reading a release page.
strip_trailers() {
  grep -vE '^(Co-[Aa]uthored-[Bb]y|Claude-Session|Signed-off-by|Signed-Off-By):' |
    awk '{ if ($0 ~ /^[[:space:]]*$/) { blank++ } else { while (blank-- > 0) print ""; blank = 0; print } }' |
    # GitHub writes a `---------` rule above the trailers it adds to a squash
    # commit. With the trailers gone the rule stands under nothing, and on the
    # release page it draws a line across the section for no reason.
    awk '{ line[NR] = $0 }
         END {
           last = NR
           while (last > 0 && line[last] ~ /^[[:space:]]*$/) last--
           if (last > 0 && line[last] ~ /^-{3,}[[:space:]]*$/) {
             last--
             while (last > 0 && line[last] ~ /^[[:space:]]*$/) last--
           }
           for (i = 1; i <= last; i++) print line[i]
         }'
}

sha=$(git rev-parse --short "$ref")

# The release commit says what the release is for. Nothing else in the history
# does, because every other commit describes one change.
headline=""
if git log -1 --format='%s' "$ref" | grep -qE '^Release [0-9]'; then
  headline=$(git log -1 --format='%b' "$ref" | strip_trailers)
fi

# A `Release X.Y.Z` subject is a version bump and reads as noise in a change
# list. Filtered here rather than with `git log --invert-grep`, which matches
# the body as well and would drop a real change that names a release.
changes=$(git log --no-merges --format='%H' "${range[@]}" | while read -r c; do
  git log -1 --format='%s' "$c" | grep -qE '^Release [0-9]' || echo "$c"
done)

# GitHub fills the body of a squash commit with `* <subject>` and that commit's
# body, once for each commit it squashed. Two things then go wrong on the
# release page. The description of the change sits under a bullet instead of
# under its own heading, and a branch that carried a commit from an earlier pull
# request repeats that change inside this one's section. v0.7.3 read as one
# section instead of two for that reason.
#
# So the bullet list is taken apart. The bullet that names this commit loses its
# marker and becomes the body. A bullet that names another commit in the same
# release is dropped, because that commit writes its own section. Any other
# bullet is kept as it stands, because a squash of commits that only ever
# existed on the branch still needs all of them, and that case still gets the
# warning: only a person can turn several bullets into one description.
#
# A subject is compared without a trailing `(#12)`, which GitHub adds to the
# squash subject and not to the bullets.
#
# The three arguments arrive in the environment rather than through `-v`. An
# `-v` assignment is parsed for escapes and cannot hold a newline at all: the
# subject list is one string of many lines, and awk answered "newline in string"
# and wrote no notes for any release that held a squash commit.
unsquash() {
  OWN="$1" OTHERS="$2" SHORT="$3" awk '
    function norm(s) {
      sub(/[[:space:]]*\(#[0-9]+\)[[:space:]]*$/, "", s)
      gsub(/^[[:space:]]+|[[:space:]]+$/, "", s)
      return s
    }
    BEGIN {
      own = norm(ENVIRON["OWN"])
      short = ENVIRON["SHORT"]
      n = split(ENVIRON["OTHERS"], o, "\n")
      for (i = 1; i <= n; i++) if (o[i] != "") drop[norm(o[i])] = 1
      count = 0
    }
    /^\* / { count++; subject[count] = norm(substr($0, 3)); next }
    { if (count > 0) text[count] = text[count] $0 "\n" }
    END {
      kept = 0
      for (i = 1; i <= count; i++) {
        mine = (subject[i] == own)
        if (!mine && (subject[i] in drop)) continue
        keep[++kept] = i
        own_block[kept] = mine
      }
      # Every bullet named another change in this release. That cannot describe
      # this commit, so the body is left as it is and the warning stands.
      if (kept == 0) { for (i = 1; i <= count; i++) { keep[++kept] = i; own_block[kept] = 0 } }
      for (j = 1; j <= kept; j++) {
        i = keep[j]
        body = text[i]
        sub(/^\n+/, "", body)
        sub(/\n+$/, "", body)
        if (j > 1) printf "\n"
        if (own_block[j] && kept == 1) print body
        else printf "* %s\n\n%s\n", subject[i], body
      }
      if (kept > 1 || !own_block[1])
        print "release-notes: WARNING: " short " has a squash bullet list as its body, not prose. Reword it before you tag." > "/dev/stderr"
    }
  '
}

# Every subject in this release, so that a squash bullet naming one of the
# others can be told from a bullet naming this change.
subjects=$(for c in $changes; do git log -1 --format='%s' "$c"; done)

tldr=""
details=""
for c in $changes; do
  subject=$(git log -1 --format='%s' "$c")
  body=$(git log -1 --format='%b' "$c" | strip_trailers)
  # A body that opens with a bullet is a squash body that nobody reworded. A
  # bullet list further down is ordinary prose and is left alone.
  case "$body" in
    '* '*) body=$(printf '%s\n' "$body" | unsquash "$subject" "$subjects" "$(git log -1 --format='%h' "$c")") ;;
  esac
  tldr+="- $subject"$'\n'
  details+="### $subject"$'\n\n'
  if [ -n "$body" ]; then
    details+="$body"$'\n\n'
  else
    details+="No description was written for this change."$'\n\n'
  fi
done

# The API caps a release body at 125000 characters, so a long run of work has to
# be cut somewhere. The TL;DR is kept whole, because it names every change. The
# detail is what gets cut, and the note says so.
detail_cap=90000
if [ "$(printf '%s' "$details" | wc -c)" -gt "$detail_cap" ]; then
  details="$(printf '%s' "$details" | head -c "$detail_cap")

The detail is cut here, because it passed the size that a release page holds.
Every change is named in the TL;DR above. \`git log $prev..$tag\` holds the rest."
fi

echo "kari $tag — built from \`$sha\`."
if [ -n "$headline" ]; then
  echo
  echo "$headline"
fi
if [ -n "$tldr" ]; then
  echo
  echo "## TL;DR"
  printf '%s' "$tldr"
  echo
  echo "## What changed"
  echo
  printf '%s' "$details"
else
  echo
  echo "No change landed since $prev. This release republishes the same code."
  echo
fi
cat <<EOF
## Install

Download the \`.dmg\` for your Mac: \`aarch64\` for Apple silicon, \`x64\` for Intel.
The app is not signed. After you copy it to Applications, run:

\`\`\`
xattr -dr com.apple.quarantine /Applications/kari.app
\`\`\`

On Windows, run \`kari_${tag#v}_x64-setup.exe\`. It installs for the current user,
because kari reads the Claude Code state of whoever is logged in. That installer is
not signed either, so SmartScreen asks once: More info, then Run anyway.

An installed kari updates itself to this release: it checks on start and every six hours,
writes the new version beside the running one and offers a restart. Settings, Updates has the switch.

Every host runs the headless node, so that its sessions stay on the board while no window is open.
A Mac takes \`kari-node-$tag-aarch64-apple-darwin\` (or the \`x86_64\` one),
a Linux host \`kari-node-$tag-x86_64-unknown-linux-gnu.tar.gz\`,
a Windows host \`kari-node-$tag-x86_64-pc-windows-msvc.zip\`.
Run \`kari-node service install\` to keep it running at login. One engine runs on a host at a time:
open the app and the node steps down, quit the app and it takes the host back.
A node updates itself only when asked: \`kari-node update\`, or \`serve --auto-update\`.

An Android phone installs \`kari-latest.apk\`. Add this repository to Obtainium: the asset name never
carries the version and the signing key never changes, so an update installs over the previous build.
The phone reaches the nodes over a private network, such as a VPN, and pairs with the code from
Settings, Nodes on the desktop. See TOUR.md.

See the README for requirements and setup.
EOF
