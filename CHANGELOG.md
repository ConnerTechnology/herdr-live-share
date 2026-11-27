# Changelog

What changed in the most recent releases, for the people who install them. `scripts/release` moves the Unreleased section under the new version, publishes it as that release's notes on GitHub, and keeps only the last 5 releases here. Every release's notes stay on GitHub.

## Unreleased

## 0.1.16 (2026-11-26)

- The **Who can join** menu no longer has an **Anyone with repo access** row. Leave everyone unticked to let anyone with repo access join, as the button says.

## 0.1.15 (2026-11-26)

- The **Who can join** menu looks like herdr's own menus. It has a search row, an **Anyone with repo access** choice, and checkmarks next to the people you pick.
- **Copy** now has a copy icon. After you click it, it shows **Copied** with a checkmark for 2 seconds, and the button no longer changes size.

## 0.1.14 (2026-11-26)

- Space settings switches slide to their new position as soon as you click them and pulse until the change is done. Extra clicks while a change is running are ignored, and a switch slides back if the change fails.

## 0.1.13 (2026-11-26)

- The palette commands now start with **Live Share:** — Share this space, Join a shared space, Activity and Updates.
- In Space settings, the **Who can join** menu opens right under its button instead of at the bottom of the window, the join link stays on one line, and the Who can join text no longer wraps.

## 0.1.12 (2026-11-26)

- **Live share updates** is now just two buttons: **Update mods and engines**, greyed out when there's nothing to update, and **What's new**. It checks GitHub each time it opens, so **Check now** is gone.

## Older releases

This file keeps only the last 5 releases. Every release and its notes are on GitHub: https://github.com/ConnerTechnology/herdr-live-share/releases

To fetch all of them, newest first:

```sh
gh api repos/ConnerTechnology/herdr-live-share/releases --paginate \
  --jq '.[] | "## \(.tag_name) (\(.published_at[:10]))\n\n\(.body)\n"'
```

Or one release: `gh release view v0.1.0 --repo ConnerTechnology/herdr-live-share`.
