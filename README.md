# herdr live share

Pair on a repo in [herdr-app](https://github.com/herdrdev/herdr-app), each person with their own AI agent. A shared space is a shared git branch plus a small event ledger kept in hidden git refs on your GitHub remote. There is no server.

Status: scaffold. The design is in [SPEC.md](SPEC.md).

## Layout

- `engine/`: `herdr-session`, a Rust command-line tool that owns all session state. Mods and Claude Code hooks call it.
- `mods/`: four herdr-app mods (`share-settings`, `share-start`, `share-join`, `share-activity`). Each `engine.js` copy must stay identical.
- `scripts/install`: links the mods into `~/.config/herdr-app/mods/` and runs `cargo install` for the engine.
- `scripts/check`: runs `herdr-app check` on each mod, compares the `engine.js` copies, and runs `cargo test`.

## Try it

```sh
scripts/install
```

Open the command palette in herdr-app. Run **Share this space** or **Join a space**.
