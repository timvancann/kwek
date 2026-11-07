# Run from source with hot reload.
run:
    pnpm tauri dev

# Build target/release/bundle/macos/kwek.app.
bundle:
    pnpm install --frozen-lockfile
    pnpm tauri build

# Build the app and install it in ~/Applications.
install: bundle
    rm -rf ~/Applications/kwek.app
    mkdir -p ~/Applications
    cp -R target/release/bundle/macos/kwek.app ~/Applications/
