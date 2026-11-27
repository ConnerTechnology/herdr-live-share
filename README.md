# herdr live share

Pair on a repo in [herdr-app](https://github.com/herdrdev/herdr-app), each person with their own AI agent. A shared space is a shared git branch plus a small event ledger kept in hidden git refs on your GitHub remote. There is no server.

Status: scaffold. The design is in [SPEC.md](SPEC.md).

## Layout

- `engine/`: `herdr-session`, a Rust command-line tool that owns all session state. Mods and Claude Code hooks call it.
- `mods/`: four herdr-app mods (`share-settings`, `share-join`, `share-activity`, `share-updates`). Each `engine.js` copy must stay identical.
- `scripts/install`: links the mods into `~/.config/herdr-app/mods/` and builds the engine into `~/.local/bin`.
- `scripts/check`: runs `herdr-app check` on each mod, compares the `engine.js` copies, and runs `cargo test`.

## Install

Paste this into an agent in herdr-app:

```text
Read ~/.config/herdr-app/AGENTS.md, then add this mod: https://github.com/ConnerTechnology/herdr-live-share
```

The agent follows [RECIPE.md](RECIPE.md). Or run the installer yourself in a terminal:

```sh
curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh
```

herdr-app often runs on one computer and shows spaces from others (`hc machine ls`). The mods belong on the computer running the app. The engine belongs on every machine whose spaces you share. The installer works out which computer it's on:

- **On the computer running herdr-app:** it installs the mods, the engine there, and the engine on every connected machine (through `hc exec --on`).
- **On a remote machine:** it installs the engine there and tells you what to run on the herdr-app computer.

A machine added later gets the engine from the share dialogs, which offer **Install on <machine>** when it's missing. The engine comes from the latest GitHub release. With no release binary for a machine, it's built from source, which needs Rust there.

Then open the command palette in herdr-app and run **Share this space** or **Join a space**.

## Develop

`scripts/install --mods` links the mods from this checkout, so edits hot-reload. `scripts/install --engine` builds the engine into `~/.local/bin`. `scripts/check` runs every check. `scripts/release <x.y.z>` bumps the version everywhere, tags and pushes; the tag builds the release. Installed copies offer **Update to <x.y.z>** the next time a share dialog opens.
