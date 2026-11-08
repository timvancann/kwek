#!/usr/bin/env bash
# Creates a self-signed "kwek local" code-signing certificate in your login Keychain, once.
# `just bundle` signs with it, so every local build is the same app to the Keychain.
# Needs OpenSSL 3 (`brew install openssl`); macOS's own LibreSSL lacks `pkcs12 -legacy`.
set -euo pipefail

if security find-identity -p codesigning | grep -q '"kwek local"'; then
    echo "A \"kwek local\" signing identity is already in your Keychain."
    exit 0
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
CN = kwek local
[ext]
basicConstraints = critical,CA:false
keyUsage = critical,digitalSignature
extendedKeyUsage = critical,codeSigning
EOF

openssl req -x509 -newkey rsa:2048 -days 3650 -nodes -config "$tmp/cert.cnf" \
    -keyout "$tmp/key.pem" -out "$tmp/cert.pem" 2>/dev/null
password="$(openssl rand -hex 16)"
openssl pkcs12 -export -legacy -out "$tmp/kwek.p12" -inkey "$tmp/key.pem" -in "$tmp/cert.pem" \
    -name "kwek local" -passout "pass:$password"
security import "$tmp/kwek.p12" -k "$HOME/Library/Keychains/login.keychain-db" \
    -P "$password" -T /usr/bin/codesign

echo "Done. The next \`just install\` signs kwek with it."
