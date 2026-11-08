# kwek

*kwek* (Dutch, roughly "quack", or what you call endless chatter) is a fast, keyboard-first macOS client for Google Chat.

![kwek with a conversation and its thread open](docs/images/kwek.png)

- **Instant.** Your whole history lives in a local cache; live updates stream in from Google.
- **Search that finds conversations.** "demo data" finds the chat where someone said "demo" in one message and "dataset" in the next, plus related talk found by a small model running on your Mac. Narrow down with `from:` and `in:` ([screenshot](docs/images/kwek-search.png)).
- **Vim keys.** `j`/`k`, `⌘K` to jump, `⌘F` to search, `r` to reply, `?` for the rest.
- **Yours.** Your own Google Cloud project, no server in between, no telemetry.

## Inspiration

The layout is **Slack**'s, minus the feature list. The colours follow **[T3 Code](https://github.com/pingdotgg/t3code)**'s idea of one grey scale with a single accent (a mallard teal here, because of the name). The keys come from **Vim**. Most of the code was written by **[Claude](https://claude.com/claude-code)**, with Tim writing the spec and making the calls.

## Get started

You need a Google Workspace account (the Chat API skips personal Gmail) and macOS. Do the 15-minute [Google Cloud setup](docs/setup.md), then:

```sh
just install
```

Open kwek from `~/Applications` and press **Connect to Google**. Building from source and contributing: [docs/development.md](docs/development.md).

## Licence

Apache 2.0 with the [Commons Clause](https://commonsclause.com/) ([LICENSE](LICENSE)): use, change and share it freely, including at work, but don't sell it or a fork of it.
