#!/bin/bash
# Let the Android Back button reach the web view.
#
# Tauri's activity turns back navigation off: `TauriActivity` sets
# `handleBackNavigation` to false, so a press goes straight to the system and
# leaves the app. An open card, dialog or form is then impossible to close with
# the button, and a press that was meant to close it closes kari instead.
#
# With the flag on, the activity hands the press to the web view while the web
# view has somewhere to go back to. The app puts one history entry there for
# each layer it opens (see `useBackClose` in src/hooks.ts), so Back closes the
# card first and leaves the app only when nothing is open.
#
# src-tauri/gen/android is generated, so it is not tracked and this file comes
# back as Tauri wrote it on every init. Run this script after every
# `bun tauri android init` and before `bun tauri android build`.
#
# Usage: scripts/android-back.sh
set -euo pipefail
cd "$(dirname "$0")/.."

if [ ! -d src-tauri/gen/android ]; then
  echo "src-tauri/gen/android is missing. Run 'bun tauri android init' first."
  exit 1
fi

id=$(jq -r .identifier src-tauri/tauri.conf.json)
main="src-tauri/gen/android/app/src/main/java/${id//.//}/MainActivity.kt"

if [ ! -f "$main" ]; then
  echo "missing $main"
  exit 1
fi

cat > "$main" <<KOTLIN
package $id

import android.os.Bundle
import androidx.activity.enableEdgeToEdge

class MainActivity : TauriActivity() {
  // The web view holds the history of what is open. Back must close that
  // before it leaves the app. Written by scripts/android-back.sh.
  override val handleBackNavigation: Boolean = true

  override fun onCreate(savedInstanceState: Bundle?) {
    enableEdgeToEdge()
    super.onCreate(savedInstanceState)
  }
}
KOTLIN

echo "Wrote $main with back navigation on."
