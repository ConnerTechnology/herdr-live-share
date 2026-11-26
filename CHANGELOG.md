# Changelog

What changed in the most recent releases, for the people who install them. `scripts/release` moves the Unreleased section under the new version, publishes it as that release's notes on GitHub, and keeps only the last 5 releases here. Every release's notes stay on GitHub.

## Unreleased

## 0.1.1 (2026-11-26)

- Share dialogs offer **Update to <version>** when a newer release exists, and **Update on <machine>** when a machine's engine is older than the mods. Nothing updates without a click.
- `scripts/release` bumps the version everywhere, tags and pushes; release notes come from this file.
- The installer replaces the engine with an atomic rename, so a Claude hook that is running during an update finishes on the old version.

## 0.1.0 (2026-11-26)

First release: the installer and a scaffold. Nothing is shared yet.

- **One-line install.** Paste `Read ~/.config/herdr-app/AGENTS.md, then add this mod: https://github.com/ConnerTechnology/herdr-live-share` into an agent in herdr-app, or run `curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh`.
- **Remote machines.** Run on the computer with herdr-app, the installer adds the mods there and the engine on every connected machine. Run on a remote machine, it installs the engine and says what to run on the herdr-app computer.
- **Command palette.** **Share this space**, **Join a space** and **Activity** open placeholder dialogs that show whether the engine is installed on the space's machine, with **Install on <machine>** when it is missing.
- **Engine binaries** for Linux and macOS (x86_64 and arm64). `herdr-session version` works; other commands answer "not built yet".

## Older releases

This file keeps only the last 5 releases. Every release and its notes are on GitHub: https://github.com/ConnerTechnology/herdr-live-share/releases

To fetch all of them, newest first:

```sh
gh api repos/ConnerTechnology/herdr-live-share/releases --paginate \
  --jq '.[] | "## \(.tag_name) (\(.published_at[:10]))\n\n\(.body)\n"'
```

Or one release: `gh release view v0.1.0 --repo ConnerTechnology/herdr-live-share`.
