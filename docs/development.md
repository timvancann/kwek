# Developing kwek

Tauri 2 + Svelte 5. Rust owns all I/O (`crates/kwek-core` has no Tauri dependency); the webview only renders. You need Rust, Node with pnpm, and [just](https://github.com/casey/just), plus the [Google Cloud setup](setup.md).

## Build

`just install` builds `kwek.app` into `~/Applications`; `just bundle` only builds it; `just run` runs from source with hot reload.

Unsigned, every build is a different app to the Keychain, so after each rebuild macOS asks before kwek can read its saved sign-in. A self-signed "kwek local" certificate fixes that: create it once with `tools/make-signing-cert.sh` (needs OpenSSL 3, `brew install openssl`), and `just bundle` signs with it from then on, so every build is the same app. It isn't trusted system-wide and doesn't need to be; it only has to stay the same. Two prompts to expect: each `just bundle` asks to use the signing key (choose Allow, not Always Allow, so no other program can sign itself as kwek and read its saved sign-in), and the first signed build asks once more for the sign-in saved by the last unsigned one (Always Allow is fine there).

Without anything extra, kwek asks for the user's client file on first start and keeps its details in the data folder (`google-client.json`). With your client file saved as `secrets/oauth-client.json` (gitignored), the build bakes it in instead and skips that step, which suits development. `KWEK_OAUTH_CLIENT_ID`, `KWEK_OAUTH_CLIENT_SECRET`, `KWEK_GCP_PROJECT` and `KWEK_PUBSUB_TOPIC` (default `chat-events`) override the file. A client file given in the app wins over the baked-in one.

Related search uses [multilingual-e5-small](https://huggingface.co/intfloat/multilingual-e5-small) (int8, about 135 MB), bundled with the app and run on-device with [fastembed](https://github.com/Anush008/fastembed-rs). `just model` fetches it into `models/` (pinned revision, checksummed); `just run` and `just bundle` do that for you.

## Windows (experimental)

`just windows` cross-compiles an unsigned NSIS installer on the Mac. It needs `brew install llvm nsis`, `cargo install cargo-xwin` and `rustup target add x86_64-pc-windows-msvc`; cargo-xwin downloads the Windows SDK and CRT headers on first use. Like `just bundle`, it bakes in `secrets/oauth-client.json` when that exists, so don't publish those installers. On Windows the refresh token goes into Credential Manager and shortcuts use Ctrl instead of ⌘. Notifications are Windows toasts; they need the Start menu shortcut the installer makes, so a debug build's toasts show up as PowerShell's. Like builds from source on the Mac, these installers don't update themselves; the Windows installer from a GitHub release does (`docs/releasing.md`), at start and twice a day, with no Check for Updates menu.

## Android (experimental)

`just android` builds a debug APK and installs it on the running emulator or a connected phone. It needs the Android SDK (`brew install --cask android-commandlinetools`, then `sdkmanager "ndk;29.0.14206865" "platforms;android-36" "build-tools;36.0.0"`) and JDK 17; `ANDROID_HOME` defaults to Homebrew's location. The Android build leaves out search by meaning, system notifications, the updater and window plugins, and keeps the refresh token in a file in the app's private storage.

## Commands

```sh
pnpm tauri dev                                  # app with hot reload
KWEK_TOKEN_STORE=file pnpm tauri dev            # debug builds: keep the refresh token in a file, not the Keychain
cargo test -p kwek-core                         # core tests (applier property test, renderer, engine against a fake API)
cargo clippy --workspace --all-targets --features kwek-core/semantic -- -D warnings
pnpm check                                      # svelte-check
cargo run -p kwek-core --bin spike -- <command> # API spike / smoke test; see src/bin/spike.rs
```

`cargo test` also regenerates the TypeScript view types in `src/lib/bindings/`.

## Where things are

- Design notes (`kwek-spec.md`, `docs/plan.md`, `docs/api-findings.md`, `docs/releasing.md`, `docs/superpowers/`) are kept out of the repo: only `setup.md`, this file and `docs/images/` are tracked.
- Data: `~/Library/Application Support/nl.timvancann.kwek/` (SQLite cache, media cache). Logs: `~/Library/Logs/nl.timvancann.kwek/`. Message bodies and tokens are never logged at info level.
