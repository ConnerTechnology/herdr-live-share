# Changelog

What changed in the most recent releases, for the people who install them. `scripts/release` moves the Unreleased section under the new version, publishes it as that release's notes on GitHub, and keeps only the last 5 releases here. Every release's notes stay on GitHub.

## Unreleased

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

## 0.1.6 (2026-11-26)

- The share dialogs' version line now sits in the footer next to **Close**, and **Check for updates** is underlined so it reads as a link. Only the link is clickable.

## 0.1.5 (2026-11-26)

- **Live share updates** in the command palette, laid out like Herdr's Settings → Updates: the mods' version with **What's new** and **Check now** (or **Update to …**), and the engine's version on every connected machine with **Install on …** or **Update on …** when needed.
- Every share dialog ends with "herdr live share 0.1.4 · Up to date · **Check for updates**" (or a violet "Version 0.1.5 is available · **See update**"). Clicking it opens Live share updates, where **Check now** asks GitHub right away and **Update to …** installs.

## Older releases

This file keeps only the last 5 releases. Every release and its notes are on GitHub: https://github.com/ConnerTechnology/herdr-live-share/releases

To fetch all of them, newest first:

```sh
gh api repos/ConnerTechnology/herdr-live-share/releases --paginate \
  --jq '.[] | "## \(.tag_name) (\(.published_at[:10]))\n\n\(.body)\n"'
```

Or one release: `gh release view v0.1.0 --repo ConnerTechnology/herdr-live-share`.
