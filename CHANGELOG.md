# Changelog

What changed in the most recent releases, for the people who install them. `scripts/release` moves the Unreleased section under the new version, publishes it as that release's notes on GitHub, and keeps only the last 5 releases here. Every release's notes stay on GitHub.

## Unreleased

## 0.1.11 (2026-11-26)

- Back to the dialogs. **Share this space** opens Space settings, **Join a space** stays the same, and each dialog's footer links to **Live share updates**. The Live Share panel from 0.1.10 is removed on update. Docking it next to Files and Changes would have needed a change to herdr-app.
- **Live share updates** keeps the one-button update from 0.1.10: **Update to …** updates the mods and every engine, or **Update engines** when only an engine is behind. It no longer lists machines.

## 0.1.10 (2026-11-26)

- Agents in a shared space now hear each other:
  - **Before Claude edits a file**, the others' agents see "<person>'s agent is working on <file>". If someone else is already on that file, your agent gets a heads-up first.
  - **At the end of each turn**, the first line of Claude's reply goes to the others.
  - **After each turn**, with **Keep in sync** on, new commits are pushed: pull with rebase, then push. On a conflict the rebase is undone, nothing is pushed, and your agent hears about it on its next turn. herdr never commits for you.
  - **Pushes made from a terminal or an IDE** are noticed and shown to the others once.
- Each checkout signs with its own key. Two checkouts of one session on the same machine no longer compete for the same event stream.
- **The Live Share panel** replaces Space settings and Live share updates. Its **Share** tab holds the space name and every sharing setting. Its **Updates** tab holds the version and one update button. It follows the space you're looking at. Until herdr-app lets mods add a button next to Files and Changes, open it with **Live Share** in the command palette; it opens where you last left it. Otherwise the command palette is only for **Join a space**. Space settings, Live share updates and the Activity placeholder are removed on update.
- **Updates** has one button. **Update to …** updates the mods and the engine on every connected machine. When the mods are current but an engine is behind or missing, the button reads **Update engines** and updates them all. The dialog no longer lists machines.

## 0.1.9 (2026-11-26)

- **Join a space** is real. Paste a share link and five steps run one by one:
  1. Find the session on GitHub.
  2. Check your GitHub sign-in.
  3. Check your access to the repository and Who can join.
  4. Create your worktree in `~/herdr-sessions/<repo>/<branch>`.
  5. Check out the branch.

  Each failure says why and offers its fix: edit the link, sign in to GitHub, change the folder, or try again. The space then appears in the sidebar without opening a tab.
- With more than one machine connected, **On <machine>** picks where the joined copy goes.
- Sharing and joining set up the Claude hooks in `~/.claude/settings.json` on their own. Your other hooks and settings are kept. `herdr-session hooks uninstall` removes exactly ours.
- People with read-only access can join and follow along. They publish nothing.

## 0.1.8 (2026-11-26)

- Space settings always shows the settings under **Share this space**. While sharing is off, including before the first share, they're greyed out to 45% and disabled.

## 0.1.7 (2026-11-26)

- **Share this space** is real. The toggle starts a session at once. There's no check for uncommitted or unpushed work: the only requirement is that the branch is on GitHub. Turning it off closes the session, and turning it on again reopens it with the same link.
- While a space is shared, Space settings shows four things:
  - **Keep in sync**: your own setting, on by default.
  - **Close automatically**: closes the session after 14 days with no activity.
  - **Who can join**: everyone with repo access, or people you pick from the repo's collaborators.
  - **The join link**, with **Copy**.
- Renaming the space also renames the session.
- The engine adds `start`, `change`, `status` and `collaborators`.

## Older releases

This file keeps only the last 5 releases. Every release and its notes are on GitHub: https://github.com/ConnerTechnology/herdr-live-share/releases

To fetch all of them, newest first:

```sh
gh api repos/ConnerTechnology/herdr-live-share/releases --paginate \
  --jq '.[] | "## \(.tag_name) (\(.published_at[:10]))\n\n\(.body)\n"'
```

Or one release: `gh release view v0.1.0 --repo ConnerTechnology/herdr-live-share`.
