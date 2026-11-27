# Changelog

What changed in the most recent releases, for the people who install them. `scripts/release` moves the Unreleased section under the new version, publishes it as that release's notes on GitHub, and keeps only the last 5 releases here. Every release's notes stay on GitHub.

## Unreleased

- **Agents ask each other.** **share_ask** asks one person's agent a question, and **share_post** with kind "opinion" asks everyone for a second opinion. Answers reach the asking agent between its tool calls. If the person you ask is away, a read-only copy of their agent answers in the background. It can only read the shared folder and reply; it can't edit anything. Each person gets at most 20 background answers a day. When you come back, your agent tells you what it answered for you.

## 0.1.18 (2026-11-26)

- **Agents talk to each other.** In a shared space, Claude agents get two tools, **share_post** and **share_read**. They use them to tell the others what they're about to change, or something useful they found. News now reaches an agent between its tool calls too, not only when you send a prompt. The tools install with the hooks and never ask for permission.

## 0.1.17 (2026-11-26)

- **Join a shared space** has a **Join on** dropdown for picking the machine the session goes on, when more than one machine is connected. It replaces the "On <machine>" text, which moved to the next machine each time you clicked it.
- Space settings: when an **Update on <machine>** button shows, the footer no longer pushes **Close** out of the dialog.

## 0.1.16 (2026-11-26)

- The **Who can join** menu no longer has an **Anyone with repo access** row. Leave everyone unticked to let anyone with repo access join, as the button says.

## 0.1.15 (2026-11-26)

- The **Who can join** menu looks like herdr's own menus. It has a search row, an **Anyone with repo access** choice, and checkmarks next to the people you pick.
- **Copy** now has a copy icon. After you click it, it shows **Copied** with a checkmark for 2 seconds, and the button no longer changes size.

## 0.1.14 (2026-11-26)

- Space settings switches slide to their new position as soon as you click them and pulse until the change is done. Extra clicks while a change is running are ignored, and a switch slides back if the change fails.

## Older releases

This file keeps only the last 5 releases. Every release and its notes are on GitHub: https://github.com/ConnerTechnology/herdr-live-share/releases

To fetch all of them, newest first:

```sh
gh api repos/ConnerTechnology/herdr-live-share/releases --paginate \
  --jq '.[] | "## \(.tag_name) (\(.published_at[:10]))\n\n\(.body)\n"'
```

Or one release: `gh release view v0.1.0 --repo ConnerTechnology/herdr-live-share`.
