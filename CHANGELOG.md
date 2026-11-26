# Changelog

What changed in each release, for the people who install it. `scripts/release` moves the Unreleased section under the new version, and that section becomes the GitHub release notes.

## Unreleased

- Share dialogs offer **Update to <version>** when a newer release exists, and **Update on <machine>** when a machine's engine is older than the mods. Nothing updates without a click.
- `scripts/release` bumps the version everywhere, tags and pushes; release notes come from this file.
- The installer replaces the engine with an atomic rename, so a Claude hook that is running during an update finishes on the old version.

## 0.1.0 (2026-11-26)

First release: the installer and a scaffold. Nothing is shared yet.

- **One-line install.** Paste `Read ~/.config/herdr-app/AGENTS.md, then add this mod: https://github.com/ConnerTechnology/herdr-live-share` into an agent in herdr-app, or run `curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh`.
- **Remote machines.** Run on the computer with herdr-app, the installer adds the mods there and the engine on every connected machine. Run on a remote machine, it installs the engine and says what to run on the herdr-app computer.
- **Command palette.** **Share this space**, **Join a space** and **Activity** open placeholder dialogs that show whether the engine is installed on the space's machine, with **Install on <machine>** when it is missing.
- **Engine binaries** for Linux and macOS (x86_64 and arm64). `herdr-session version` works; other commands answer "not built yet".
