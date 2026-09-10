#!/usr/bin/env bash
# Print the name of the local code signing identity for macOS builds, and
# create it first when it does not exist yet.
#
# Why a local build must be signed. macOS asks the user before an app reads
# another app's data, controls another app, or replaces a bundle. It remembers
# the answer against the code signature of the app that asked. A Tauri build
# with no identity is signed ad-hoc, so the only stable thing in its signature
# is the hash of the binary. Every build changes that hash, the remembered
# answer no longer applies, and macOS asks again after every install.
#
# A self-signed certificate fixes it. The signature then names the bundle
# identifier and the certificate, and neither changes when the code does:
#
#   designated => identifier "io.github.lightheaded.kari"
#                 and certificate leaf = H"..."
#
# The certificate is local, and it stays local. It gives no Gatekeeper trust
# and it is not a substitute for a Developer ID. It does one job: it holds the
# app's identity still between builds.
#
# Usage:
#   APPLE_SIGNING_IDENTITY="$(scripts/mac-signing-identity.sh)" bun run tauri build
set -euo pipefail

NAME="${KARI_SIGNING_IDENTITY:-kari local signing}"
KEYCHAIN="$HOME/Library/Keychains/login.keychain-db"
DAYS=7300

if [ "$(uname -s)" != "Darwin" ]; then
  echo "this script is for macOS only" >&2
  exit 1
fi

# An identity that is already here is the one to keep. A new certificate is a
# new identity, and every remembered answer starts again.
if security find-identity -p codesigning | grep -qF "\"$NAME\""; then
  echo "$NAME"
  exit 0
fi

echo "creating the local signing identity \"$NAME\"" >&2

tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat >"$tmp/req.cnf" <<EOF
[req]
distinguished_name = dn
x509_extensions = v3
prompt = no
[dn]
CN = $NAME
[v3]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
subjectKeyIdentifier = hash
EOF

openssl req -x509 -newkey rsa:2048 -sha256 -days "$DAYS" -nodes \
  -keyout "$tmp/key.pem" -out "$tmp/cert.pem" -config "$tmp/req.cnf" 2>/dev/null

# The password guards a file that lives for one command. The algorithms are
# named because OpenSSL 3 defaults to a bundle that macOS cannot import.
openssl pkcs12 -export -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
  -name "$NAME" -out "$tmp/id.p12" -passout pass:kari \
  -macalg sha1 -certpbe PBE-SHA1-3DES -keypbe PBE-SHA1-3DES 2>/dev/null

# -A lets codesign use the key without a dialog on every build.
security import "$tmp/id.p12" -k "$KEYCHAIN" -P kari -T /usr/bin/codesign -A >&2

if ! security find-identity -p codesigning | grep -qF "\"$NAME\""; then
  echo "the identity did not reach the keychain" >&2
  exit 1
fi

echo "$NAME"
