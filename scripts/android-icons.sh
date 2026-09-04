#!/bin/bash
# Put kari's launcher icon into the generated Android project.
#
# `tauri android init` writes the Tauri logo into
# src-tauri/gen/android/app/src/main/res. That directory is generated, so it is
# not tracked and the logo comes back on every init. Run this script after
# every `tauri android init` and before `tauri android build`.
#
# Usage: scripts/android-icons.sh
#
# The manifest src-tauri/icons-src/icon.json names the three adaptive layers:
# the green plate, the horned glyph, and the monochrome glyph for themed icons.
#
# `tauri icon` writes the desktop icons and the Android mipmaps in one pass, and
# the --output option moves both. The desktop icons in src-tauri/icons are
# tracked and must not change, so this script saves that directory and puts it
# back afterwards.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ ! -d src-tauri/gen/android ]; then
  echo "src-tauri/gen/android is missing. Run 'bun tauri android init' first."
  exit 1
fi

saved=$(mktemp -d)
trap 'rm -rf "$saved"' EXIT
cp -R src-tauri/icons/. "$saved"/

bun tauri icon src-tauri/icons-src/icon.json

# The icon command also writes desktop and iOS icons. Drop them and restore the
# tracked set, so the only change is inside src-tauri/gen/android.
rm -rf src-tauri/icons
mkdir src-tauri/icons
cp -R "$saved"/. src-tauri/icons/

res=src-tauri/gen/android/app/src/main/res
for f in ic_launcher.png ic_launcher_background.png ic_launcher_foreground.png ic_launcher_monochrome.png; do
  if [ ! -f "$res/mipmap-xxxhdpi/$f" ]; then
    echo "missing $res/mipmap-xxxhdpi/$f"
    exit 1
  fi
done
if [ ! -f "$res/mipmap-anydpi-v26/ic_launcher.xml" ]; then
  echo "missing $res/mipmap-anydpi-v26/ic_launcher.xml"
  exit 1
fi

echo "Wrote the launcher icon into $res."
