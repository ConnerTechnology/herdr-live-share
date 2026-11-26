# Changelog

What changed in the most recent releases, for the people who install them. `scripts/release` moves the Unreleased section under the new version, publishes it as that release's notes on GitHub, and keeps only the last 5 releases here. Every release's notes stay on GitHub.

## Unreleased

- The update check runs each time a share dialog opens (at most every 10 minutes) instead of once a day, and falls back to `curl` when herdr-app's own request fails. Install this release with the install line once; later releases arrive through **Update to …**.

## 0.1.2 (2026-11-26)

- **The session ledger.** Each machine keeps a signed event stream in hidden git refs on the repo's remote; the engine combines everyone's streams into one session state the same way on every machine, whatever order they arrive in. Events from a newer release are skipped, not fatal.
- **Claude catch-up.** In a shared checkout, the Claude prompt and session-start hooks fetch the session and give the agent what is new since its last turn: pushes, tasks, chat, who is editing what. Each Claude conversation keeps its own place. Offline, it works from local state and says so.
- Not wired up yet: starting or joining a session from the dialogs, and installing the hooks.

## 0.1.1 (2026-11-26)

- Share dialogs offer **Update to <version>** when a newer release exists, and **Update on <machine>** when a machine's engine is older than the mods. Nothing updates without a click.
- `scripts/release` bumps the version everywhere, tags and pushes; release notes come from this file.
- `CHANGELOG.md` keeps the last 5 releases; every release's notes are on GitHub, and the file's footer shows how to fetch them all.
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
