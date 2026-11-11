#!/usr/bin/env bash
# Creates the self-signed "kwek release" code-signing certificate and hands it to GitHub
# as the release workflow's signing secrets, once. Every release is then signed by the
# same certificate, so the Keychain keeps trusting kwek's saved sign-in across updates.
# The certificate never touches disk outside a temp folder that is removed at the end, and
# isn't kept anywhere else: losing it means one more Keychain prompt per user, not more.
# A Developer ID (docs/releasing.md) replaces it by setting the same secrets.
#
# Running it again makes a new certificate, which costs every user one Keychain prompt
# after the next update, so it refuses while the secrets exist. `--replace` overrides that
# (also the way out if a run stopped between the three secrets).
# Needs OpenSSL 3 (`brew install openssl@3`) and gh, signed in with access to the repo.
set -euo pipefail

if [ "${1:-}" != "--replace" ] && gh secret list --json name --jq '.[].name' | grep -qx MACOS_SIGNING_IDENTITY; then
    echo "The signing secrets already exist. Run with --replace to make a new certificate;" >&2
    echo "each user then confirms kwek's saved sign-in once after the next update." >&2
    exit 1
fi

# macOS's own openssl is LibreSSL, which can't write the -legacy p12 below.
openssl_bin=""
for candidate in "$(brew --prefix openssl@3 2>/dev/null || true)/bin/openssl" "$(command -v openssl || true)"; do
    if [ -x "$candidate" ] && "$candidate" version | grep -q '^OpenSSL 3'; then
        openssl_bin="$candidate"
        break
    fi
done
if [ -z "$openssl_bin" ]; then
    echo "Needs OpenSSL 3: brew install openssl@3" >&2
    exit 1
fi

umask 077
tmp="$(mktemp -d)"
trap 'rm -rf "$tmp"' EXIT

cat > "$tmp/cert.cnf" <<'EOF'
[req]
distinguished_name = dn
x509_extensions = ext
prompt = no
[dn]
CN = kwek release
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF

"$openssl_bin" req -x509 -newkey rsa:2048 -days 7300 -nodes -config "$tmp/cert.cnf" \
    -keyout "$tmp/key.pem" -out "$tmp/cert.pem"
password="$("$openssl_bin" rand -hex 16)"
# -legacy: macOS's `security import` can't read OpenSSL 3's default encryption.
"$openssl_bin" pkcs12 -export -legacy -out "$tmp/kwek.p12" -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
    -name "kwek release" -passout "pass:$password"

# Three separate writes: if one fails, the secrets don't match. Say so instead of leaving
# a release to fail at signing.
trap 'rm -rf "$tmp"; echo "Stopped partway: the signing secrets may not match. Run again with --replace." >&2' ERR
base64 -i "$tmp/kwek.p12" | gh secret set MACOS_CERTIFICATE
printf '%s' "$password" | gh secret set MACOS_CERTIFICATE_PASSWORD
printf '%s' "kwek release" | gh secret set MACOS_SIGNING_IDENTITY

echo "Done. The next release is signed as \"kwek release\"."
