#!/bin/bash
# Refuse text that points at one machine or one person.
#
# The tracked files must hold no absolute home path, no personal email address,
# no private network address and no internal host name. CI runs this script.
# Run it before a commit: scripts/check-privacy.sh
set -uo pipefail
cd "$(dirname "$0")/.."

# Binary and generated files carry no prose.
files=$(git ls-files | grep -vE '\.(png|icns|ico|jpg|gif|woff2?|ttf)$|(^|/)(bun\.lock|Cargo\.lock)$')

hits=0
check() { # $1 = rule, $2 = regex, $3 = regex of allowed matches (may be empty)
  local rule=$1 re=$2 allow=${3:-}
  local out
  out=$(printf '%s\n' "$files" | xargs grep -nHE -- "$re" 2>/dev/null || true)
  if [ -n "$allow" ] && [ -n "$out" ]; then
    out=$(printf '%s\n' "$out" | grep -vE -- "$allow" || true)
  fi
  if [ -n "$out" ]; then
    echo "$rule:"
    printf '%s\n' "$out" | cut -c1-160 | sed 's/^/  /'
    hits=1
  fi
}

# A home path names the local user. Documentation uses ~ or a placeholder.
check "absolute home path" '/(Users|home)/[A-Za-z0-9._-]+/' '/(Users|home)/(you|dev|USER|username|<[a-z-]+>|\$[A-Za-z_{}]+)/'
# An email address. The no-reply addresses of the forge and of the AI tool are fine.
check "email address" '[A-Za-z0-9._%+-]+@[A-Za-z][A-Za-z0-9-]*(\.[A-Za-z0-9-]+)*\.[a-z]{2,}\b' 'noreply@anthropic\.com|users\.noreply\.github\.com|@example\.(com|org)'
# A private network address.
check "private network address" '\b(10\.[0-9]{1,3}\.[0-9]{1,3}\.[0-9]{1,3}|192\.168\.[0-9]{1,3}\.[0-9]{1,3}|172\.(1[6-9]|2[0-9]|3[01])\.[0-9]{1,3}\.[0-9]{1,3}|100\.(6[4-9]|[7-9][0-9]|1[01][0-9]|12[0-7])\.[0-9]{1,3}\.[0-9]{1,3})\b' ''
# An internal host name.
check "internal host name" '\b[a-z0-9-]+\.(local|lan|internal|home|corp|intranet)\b' ''

if [ "$hits" = 1 ]; then
  echo
  echo "check-privacy: the lines above point at one machine or one person. Replace them with a placeholder."
  exit 1
fi
echo "check-privacy: clean"
