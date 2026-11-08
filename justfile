# Run from source with hot reload.
run: model
    pnpm tauri dev

# Fetch the on-device search model into models/ (pinned and checksummed).
model:
    tools/fetch-model.sh

# Build target/release/bundle/macos/kwek.app.
bundle: model
    pnpm install --frozen-lockfile
    pnpm tauri build

# Build the app and install it in ~/Applications.
install: bundle
    rm -rf ~/Applications/kwek.app
    mkdir -p ~/Applications
    cp -R target/release/bundle/macos/kwek.app ~/Applications/
