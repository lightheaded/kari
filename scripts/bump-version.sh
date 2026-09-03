#!/bin/bash
# Set the app version in every manifest, then refresh the lock files.
#
# Usage: scripts/bump-version.sh 0.2.0
#
# Then review, commit, tag `v0.2.0` and push the tag. The release workflow
# builds the bundles and publishes the GitHub release.
set -euo pipefail

v="${1:?usage: scripts/bump-version.sh X.Y.Z}"
case "$v" in
  [0-9]*.[0-9]*.[0-9]*) ;;
  *) echo "version must look like X.Y.Z"; exit 1 ;;
esac

cd "$(dirname "$0")/.."

jq --arg v "$v" '.version = $v' package.json > package.json.tmp && mv package.json.tmp package.json
jq --arg v "$v" '.version = $v' src-tauri/tauri.conf.json > src-tauri/tauri.conf.json.tmp && mv src-tauri/tauri.conf.json.tmp src-tauri/tauri.conf.json
sed -i '' -E "s/^version = \"[0-9]+\.[0-9]+\.[0-9]+\"$/version = \"$v\"/" Cargo.toml

cargo update --workspace --quiet
bun install --silent

echo "version set to $v in package.json, Cargo.toml and src-tauri/tauri.conf.json"
echo "next: git commit -am \"Release $v\" && git tag v$v && git push && git push origin v$v"
