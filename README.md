# kwek

*kwek* (Dutch, roughly "quack", and what you call it when people won't stop chattering) is a keyboard-first macOS client for Google Chat.

Google Chat in the browser is fine until you use it all day. Switching spaces waits on the network, search finds a word but not the conversation you meant, and your hands keep leaving the keyboard for the mouse. kwek is what happens when someone gets fed up with that and has a weekend and an AI pair programmer.

## What it does

- **Instant.** Your whole history lives in a local SQLite cache, so switching spaces and scrolling never wait on Google. Live updates arrive through Google's Workspace Events API and Pub/Sub, so messages, edits, reactions and read state stay in sync with the web and your phone.
- **Search that finds conversations.** Type "demo data" and kwek finds the stretch of chat where someone said "demo" in one message and "dataset" two messages later. A small multilingual embedding model runs on your Mac and adds results that mean the same thing without sharing your words. Narrow down with `from:` and `in:` (pick a name from the list) or `before:` and `after:`.
- **Vim fingers welcome.** `j`/`k` through messages, `J`/`K` to the next unread conversation, `⌘K` to jump anywhere, `⌘F` to search, `r` to reply in a thread, `e` to edit, `dd` to delete, `+` to react, `y` to copy a link, `⌃H`/`⌃L` between panes. Press `?` in the app for the full list.
- **Small on purpose.** Read, write, reply, react, notifications for DMs and mentions. No calls, no presence, no space administration. It does the daily thing and does it quickly.
- **Yours.** It talks only to Google's official APIs, with your own OAuth client in your own Google Cloud project. No server in between, no telemetry, and message text goes nowhere except to Google.

## Inspiration and thanks

- **Slack**, for the layout: sidebar, channel, thread panel. kwek borrows the structure and deliberately skips the feature list (no canvases, no workflows, no huddles).
- **[T3 Code](https://github.com/pingdotgg/t3code)**, for the colour thinking. Its palette is one cool grey scale where every surface gets its own step, plus a single accent. kwek uses the same idea with its own colours: white canvas, a quiet grey sidebar, hover that never pretends to be sidebar, and a mallard teal accent, because of the name.
- **Vim**, for the idea that your hands belong on the keys.
- **[Claude](https://claude.com/claude-code)**, which wrote most of the code. Tim wrote the spec, made the calls and used the app; Claude implemented it milestone by milestone, with separate review agents playing a picky UX lead and a picky Rust engineer before anything counted as done. The commit history shows how that went.
- **[multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small)**, the embedding model behind related search, small enough to ship inside the app and good with Dutch and English alike.
- Built with [Tauri 2](https://tauri.app), [Svelte 5](https://svelte.dev), [shadcn-svelte](https://shadcn-svelte.com), SQLite FTS5 and [fastembed](https://github.com/Anush008/fastembed-rs). Rust owns all I/O; the webview only renders.

## What you need

- A **Google Workspace** account. The Chat API does not serve personal Gmail accounts.
- A Google Cloud project in that Workspace, set up once in about 15 minutes: [docs/setup.md](docs/setup.md).
- macOS, Rust, Node with pnpm, and [just](https://github.com/casey/just).

## Build and run

After the setup, with `secrets/oauth-client.json` in place:

```sh
just install
```

That builds `kwek.app` into `~/Applications`. Open it, press **Connect to Google**, and kwek starts downloading your history in the background. `just bundle` only builds the app; `just run` runs from source with hot reload.

The build bakes in your OAuth client and its project from `secrets/oauth-client.json` (gitignored). `KWEK_OAUTH_CLIENT_ID`, `KWEK_OAUTH_CLIENT_SECRET`, `KWEK_GCP_PROJECT` and `KWEK_PUBSUB_TOPIC` override it. `just model` fetches the search model into `models/` (pinned revision, checksummed); `just run` and `just bundle` do that for you. The app is unsigned, which is fine on the Mac that built it.

## Develop

```sh
pnpm tauri dev                                  # app with hot reload
KWEK_TOKEN_STORE=file pnpm tauri dev            # debug builds: keep the refresh token in a file, not the Keychain
cargo test -p kwek-core                         # core tests (applier property test, renderer, engine against a fake API)
cargo clippy --workspace --all-targets --features kwek-core/semantic -- -D warnings
pnpm check                                      # svelte-check
cargo run -p kwek-core --bin spike -- <command> # API spike / smoke test; see src/bin/spike.rs
```

`cargo test` also regenerates the TypeScript view types in `src/lib/bindings/`. The design record: `kwek-spec.md` (behaviour), `docs/plan.md` (order and decisions), `docs/api-findings.md` (what the APIs really do, as opposed to what the docs suggest).

Data lives in `~/Library/Application Support/nl.timvancann.kwek/` (SQLite cache, media cache), logs in `~/Library/Logs/nl.timvancann.kwek/`. Message bodies and tokens are never logged at info level.

## Licence

Apache License 2.0 with the [Commons Clause](https://commonsclause.com/): see [LICENSE](LICENSE).

In plain words: you may use, modify and share kwek and forks of it for free, including at work. You may not sell kwek, a modified version or a fork, or a paid service built mainly on it.
