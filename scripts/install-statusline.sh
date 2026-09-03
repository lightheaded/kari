#!/bin/bash
# Install the kari status line wrapper.
# The wrapper saves rate_limits from every status line refresh to ~/.config/kari/rate-limits.json,
# then runs your original status line command unchanged.
#
# Usage: scripts/install-statusline.sh            install
#        scripts/install-statusline.sh --uninstall  restore the original command
set -euo pipefail

SETTINGS="${CLAUDE_CONFIG_DIR:-$HOME/.claude}/settings.json"
KARI_DIR="$HOME/.config/kari"
WRAPPER="$KARI_DIR/statusline.sh"
ORIG_FILE="$KARI_DIR/statusline.original"

command -v jq >/dev/null || { echo "jq is required"; exit 1; }
mkdir -p "$KARI_DIR"

if [ "${1:-}" = "--uninstall" ]; then
  if [ ! -f "$ORIG_FILE" ]; then echo "nothing to restore"; exit 0; fi
  orig=$(cat "$ORIG_FILE")
  tmp=$(mktemp)
  jq --arg cmd "$orig" '.statusLine.command = $cmd' "$SETTINGS" > "$tmp" && mv "$tmp" "$SETTINGS"
  rm -f "$ORIG_FILE" "$WRAPPER"
  echo "restored status line command: $orig"
  exit 0
fi

current=$(jq -r '.statusLine.command // empty' "$SETTINGS")
if [ -z "$current" ]; then
  echo "No statusLine.command in $SETTINGS. Installing a wrapper with a minimal status line."
  current="jq -r '\"[\\(.model.display_name)] \\(.context_window.used_percentage // 0)% ctx\"'"
fi
if [ "$current" = "$WRAPPER" ]; then
  echo "already installed"
  exit 0
fi

cp "$SETTINGS" "$SETTINGS.bak-kari-$(date +%Y%m%d%H%M%S)"
printf '%s' "$current" > "$ORIG_FILE"

cat > "$WRAPPER" <<EOF
#!/bin/bash
# kari status line wrapper. Captures rate limits, then runs the original status line.
input=\$(cat)
printf '%s' "\$input" | jq -c --argjson ts "\$(date +%s)" '{ts:\$ts, session_id:.session_id, rate_limits:.rate_limits}' > '$KARI_DIR/rate-limits.json.tmp' 2>/dev/null \\
  && [ -s '$KARI_DIR/rate-limits.json.tmp' ] && mv -f '$KARI_DIR/rate-limits.json.tmp' '$KARI_DIR/rate-limits.json'
printf '%s' "\$input" | $current
EOF
chmod +x "$WRAPPER"

tmp=$(mktemp)
jq --arg cmd "$WRAPPER" '.statusLine.command = $cmd' "$SETTINGS" > "$tmp" && mv "$tmp" "$SETTINGS"
echo "installed. original command saved to $ORIG_FILE"
echo "new sessions write $KARI_DIR/rate-limits.json on every status line refresh"
