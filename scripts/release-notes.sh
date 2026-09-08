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
    awk '{ if ($0 ~ /^[[:space:]]*$/) { blank++ } else { while (blank-- > 0) print ""; blank = 0; print } }'
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

tldr=""
details=""
for c in $changes; do
  subject=$(git log -1 --format='%s' "$c")
  body=$(git log -1 --format='%b' "$c" | strip_trailers)
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

An installed kari updates itself to this release: it checks on start and every six hours,
writes the new version beside the running one and offers a restart. Settings, Updates has the switch.

A Linux host runs the headless node from \`kari-node-$tag-x86_64-unknown-linux-gnu.tar.gz\`,
a Windows host from \`kari-node-$tag-x86_64-pc-windows-msvc.zip\`.
Start it with \`kari-node serve\` and add it under Settings, Nodes in the app.
A node updates itself only when asked: \`kari-node update\`, or \`serve --auto-update\`.

An Android phone installs \`kari-latest.apk\`. Add this repository to Obtainium: the asset name never
carries the version and the signing key never changes, so an update installs over the previous build.
The phone reaches the nodes over a private network, such as a VPN, and pairs with the code from
Settings, Nodes on the desktop. See TOUR.md.

See the README for requirements and setup.
EOF
