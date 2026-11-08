# Run from source with hot reload.
run: model
    pnpm tauri dev

# Fetch the on-device search model into models/ (pinned and checksummed).
model:
    tools/fetch-model.sh

# Build target/release/bundle/macos/kwek.app. Signed with the "kwek local" identity when
# there is one (tools/make-signing-cert.sh), so every build is the same app to the Keychain
# and the saved sign-in stays usable without a prompt. macOS asks to use the signing key on
# each build (Allow). Without the identity, the build is unsigned.
bundle: model
    pnpm install --frozen-lockfile
    pnpm tauri build
    if security find-identity -p codesigning | grep -q '"kwek local"'; then \
        codesign --force --sign "kwek local" target/release/bundle/macos/kwek.app \
            || echo "warning: signing with \"kwek local\" failed; the build is unsigned"; \
    fi

# Build the app and install it in ~/Applications.
install: bundle
    rm -rf ~/Applications/kwek.app
    mkdir -p ~/Applications
    cp -R target/release/bundle/macos/kwek.app ~/Applications/
