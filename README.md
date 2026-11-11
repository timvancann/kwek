<h1 align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/images/wordmark-on-dark.svg">
    <img src="docs/images/wordmark-on-light.svg" width="300" alt="kwek">
  </picture>
</h1>

<p align="center">A keyboard-first Google Chat client for macOS.</p>

<p align="center">
  <a href="https://github.com/timvancann/kwek/releases/latest/download/kwek-macOS.zip"><img src="https://img.shields.io/badge/Download_for_macOS-0d7266?style=for-the-badge&logo=apple&logoColor=white" alt="Download for macOS"></a>
</p>

<p align="center">
  <a href="https://github.com/timvancann/kwek/releases/latest"><img src="https://img.shields.io/github/v/release/timvancann/kwek?label=release&color=0d7266" alt="Latest release"></a>
  <a href="https://github.com/timvancann/kwek/actions/workflows/ci.yml"><img src="https://github.com/timvancann/kwek/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
</p>

![A conversation with its thread open](docs/images/kwek.png)

Google Chat's interface is no match for Slack's. It only runs in the browser, it's hard
to search and hard to navigate, and it feels more like a phone messenger than a workspace
for an organisation. kwek is the client I wanted instead.

- Everything is cached locally, so switching spaces and scrolling back is instant.
- Search finds conversations, not just single messages, and goes back through your whole
  history.
- Vim keys: `j`/`k`, `⌘K` to jump, `⌘F` to search, `r` to reply, `?` for the rest.
- Notifications, status, meeting reminders with a Join button, images and videos.
- No server in between and no telemetry.

## Installing

You need a Google Workspace account and your own Google Cloud project, about 15 minutes
of setup: [docs/setup.md](docs/setup.md).

Then [download kwek](https://github.com/timvancann/kwek/releases/latest/download/kwek-macOS.zip)
and drag it into Applications (Apple silicon, macOS 13 or newer). It isn't notarized, so
the first time you'll need System Settings, Privacy & Security, Open Anyway. After that it
updates itself.

To build it yourself you need Rust, pnpm and just: `just run` or `just install`. See
[docs/development.md](docs/development.md).

## Credits

Built with [Tauri](https://tauri.app) and [Svelte](https://svelte.dev), and mostly
written by [Claude Code](https://claude.com/claude-code). Colours after
[T3 Code](https://github.com/pingdotgg/t3code). Kwek is Dutch for quack.

## Licence

Apache 2.0 with the [Commons Clause](https://commonsclause.com/): use it, change it,
fork it, but don't sell it. See [LICENSE](LICENSE).
