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

# Cross-build a Windows installer on this Mac, into
# target/x86_64-pc-windows-msvc/release/bundle/nsis/. Experimental; see docs/development.md.
windows: model
    pnpm install --frozen-lockfile
    PATH="$(brew --prefix llvm)/bin:$PATH" pnpm tauri build --runner cargo-xwin --target x86_64-pc-windows-msvc

# Build a debug APK for Android (arm64) and install it on the running emulator or the
# connected phone. Needs the Android SDK with an NDK; see docs/development.md.
android:
    #!/usr/bin/env bash
    set -euo pipefail
    export ANDROID_HOME="${ANDROID_HOME:-/opt/homebrew/share/android-commandlinetools}"
    export NDK_HOME="${NDK_HOME:-$(ls -d "$ANDROID_HOME"/ndk/* | sort -V | tail -1)}"
    [ -d src-tauri/gen/android ] || pnpm tauri android init --ci
    pnpm tauri android build --debug --apk --target aarch64
    adb install -r src-tauri/gen/android/app/build/outputs/apk/universal/debug/app-universal-debug.apk

# Build the app and install it in ~/Applications.
install: bundle
    rm -rf ~/Applications/kwek.app
    mkdir -p ~/Applications
    cp -R target/release/bundle/macos/kwek.app ~/Applications/
