# kwek

A keyboard-first macOS client for Google Chat in the example.com Workspace. Tauri 2 + Svelte 5, Rust owns all I/O; the webview only renders. Spec: `kwek-spec.md`. Plan and decisions: `docs/plan.md`.

## Use it

Build and install into `~/Applications`, then open kwek and press **Connect to Google**.

```sh
just install
```

`just bundle` only builds `target/release/bundle/macos/kwek.app`; `just run` runs from source with hot reload.

The build bakes in the OAuth desktop client from `secrets/oauth-client.json` (gitignored; see `docs/setup.md` for the one-time Google Cloud setup). The app is unsigned, which is fine on the Mac that built it.

Search runs on this Mac: keyword relevance over conversation stretches, plus related results from a small multilingual embedding model (multilingual-e5-small, int8, ~135 MB) bundled with the app. `just model` fetches it into `models/` (pinned revision, checksummed); `just run` and `just bundle` do that for you. A plain `cargo build -p kwek` needs it there too, since Tauri bundles it as a resource. After sign-in, kwek downloads every conversation's full history in the background so search reaches back to the start.

## Develop

```sh
pnpm tauri dev                                  # app with hot reload
KWEK_TOKEN_STORE=file pnpm tauri dev            # debug builds: keep the refresh token in a file, not the Keychain
cargo test -p kwek-core                         # core tests (applier property test, renderer, engine against a fake API)
cargo clippy --workspace --all-targets --features kwek-core/semantic -- -D warnings
pnpm check                                      # svelte-check
cargo run -p kwek-core --bin spike -- <command> # API spike / smoke test; see src/bin/spike.rs
```

`cargo test` also regenerates the TypeScript view types in `src/lib/bindings/`.

Data lives in `~/Library/Application Support/nl.timvancann.kwek/` (SQLite cache, media cache), logs in `~/Library/Logs/nl.timvancann.kwek/`. Message bodies and tokens are never logged at info level.
