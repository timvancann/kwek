# kwek

A keyboard-first macOS client for Google Chat in the example.com Workspace. Tauri 2 + Svelte 5, Rust owns all I/O; the webview only renders. Spec: `kwek-spec.md`. Plan and decisions: `docs/plan.md`.

## Use it

Build once, then open `target/release/bundle/macos/kwek.app` (drag it to Applications if you like) and press **Connect to Google**.

```sh
pnpm install
pnpm tauri build
```

The build bakes in the OAuth desktop client from `secrets/oauth-client.json` (gitignored; see `docs/setup.md` for the one-time Google Cloud setup). The app is unsigned, which is fine on the Mac that built it.

## Develop

```sh
pnpm tauri dev                                  # app with hot reload
KWEK_TOKEN_STORE=file pnpm tauri dev            # debug builds: keep the refresh token in a file, not the Keychain
cargo test -p kwek-core                         # core tests (applier property test, renderer, engine against a fake API)
cargo clippy --workspace --all-targets -- -D warnings
pnpm check                                      # svelte-check
cargo run -p kwek-core --bin spike -- <command> # API spike / smoke test; see src/bin/spike.rs
```

`cargo test` also regenerates the TypeScript view types in `src/lib/bindings/`.

Data lives in `~/Library/Application Support/nl.timvancann.kwek/` (SQLite cache, media cache), logs in `~/Library/Logs/nl.timvancann.kwek/`. Message bodies and tokens are never logged at info level.
