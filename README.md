# kwek

A keyboard-first macOS client for Google Chat. It keeps your whole history in a local cache, so switching spaces and scrolling don't wait on the network; search runs on your Mac and finds conversations, not just single messages; and everything a mouse can do has a key. Tauri 2 + Svelte 5; Rust owns all I/O, the webview only renders.

It talks only to Google's official APIs, with your own OAuth client in your own Google Cloud project. Nothing goes through a third party, and no message text leaves your machine except to Google.

## What you need

- A Google Workspace account. The Chat API does not serve personal Gmail accounts.
- A Google Cloud project in that Workspace, set up once (about 15 minutes): [docs/setup.md](docs/setup.md).
- macOS, Rust, Node with pnpm, and [just](https://github.com/casey/just).

## Build and run

After the setup, with `secrets/oauth-client.json` in place:

```sh
just install
```

That builds `kwek.app` into `~/Applications`. Open it and press **Connect to Google**. `just bundle` only builds `target/release/bundle/macos/kwek.app`; `just run` runs from source with hot reload.

The build bakes in your OAuth client and its project from `secrets/oauth-client.json` (gitignored). `KWEK_OAUTH_CLIENT_ID`, `KWEK_OAUTH_CLIENT_SECRET`, `KWEK_GCP_PROJECT` and `KWEK_PUBSUB_TOPIC` override it. The app is unsigned, which is fine on the Mac that built it.

## Search

Search matches stretches of a conversation, so words spread over a few messages still find the place, and adds related results from a small multilingual embedding model (multilingual-e5-small, int8, about 135 MB) bundled with the app and run on-device. `just model` fetches it into `models/` (pinned revision, checksummed); `just run` and `just bundle` do that for you. After sign-in, kwek downloads every conversation's full history in the background so search reaches back to the start. Type `from:` or `in:` in the search box to pick a person or conversation.

## Develop

```sh
pnpm tauri dev                                  # app with hot reload
KWEK_TOKEN_STORE=file pnpm tauri dev            # debug builds: keep the refresh token in a file, not the Keychain
cargo test -p kwek-core                         # core tests (applier property test, renderer, engine against a fake API)
cargo clippy --workspace --all-targets --features kwek-core/semantic -- -D warnings
pnpm check                                      # svelte-check
cargo run -p kwek-core --bin spike -- <command> # API spike / smoke test; see src/bin/spike.rs
```

`cargo test` also regenerates the TypeScript view types in `src/lib/bindings/`. The design record: `kwek-spec.md` (behaviour), `docs/plan.md` (order and decisions), `docs/api-findings.md` (what the APIs really do).

Data lives in `~/Library/Application Support/nl.timvancann.kwek/` (SQLite cache, media cache), logs in `~/Library/Logs/nl.timvancann.kwek/`. Message bodies and tokens are never logged at info level.

## Licence

Apache License 2.0 with the [Commons Clause](https://commonsclause.com/): see [LICENSE](LICENSE).

In plain words: you may use, modify and share kwek and forks of it for free, including at work. You may not sell kwek, a modified version or a fork, or a paid service built mainly on it.
