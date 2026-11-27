#!/bin/sh
# herdr live share installer.
#
#   curl -fsSL https://raw.githubusercontent.com/ConnerTechnology/herdr-live-share/main/install.sh | sh
#
# Run it anywhere. It works out what this computer is:
#   - herdr-app runs here: install the mods into ~/.config/herdr-app/mods, the
#     engine here, and the engine on every machine herdr is connected to
#     (hc machine ls), through `hc exec --on <machine>`.
#   - herdr-app does not run here (a remote machine): install the engine here
#     and print the line to run on the computer where you use herdr-app.
#
# Options (after `sh -s --` when piped):
#   --engine    only install the engine on this machine (used for remote machines)
#   --app       treat this computer as the herdr-app computer even if the app is not running
#   --dry-run   print what would happen, change nothing
set -eu

REPO="ConnerTechnology/herdr-live-share"
RAW="https://raw.githubusercontent.com/$REPO/main/install.sh"
BIN_DIR="$HOME/.local/bin"   # herdr-core's exec PATH includes it; ~/.cargo/bin is not
MODS="share-settings share-join share-activity share-updates"
RETIRED="share-start share-panel"   # mods earlier releases installed; removed on update

mode=auto dry=0
for arg in "$@"; do
  case "$arg" in
    --engine) mode=engine ;;
    --app) mode=app ;;
    --dry-run) dry=1 ;;
    *) echo "unknown option: $arg" >&2; exit 2 ;;
  esac
done

say() { printf '%s\n' "$*"; }
run() { if [ $dry = 1 ]; then say "would run: $*"; else "$@"; fi; }
need() { command -v "$1" >/dev/null 2>&1 || { say "herdr live share needs $1 on $(hostname)." >&2; exit 1; }; }

need curl
need tar
tmp=$(mktemp -d)
trap 'rm -rf "$tmp"' EXIT

# One version for the mods and the engine: the latest release, else main (built from source).
tag=$(curl -fsSL "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null \
  | sed -n 's/.*"tag_name": *"\([^"]*\)".*/\1/p' | head -n 1) || true
ref=${tag:-main}

fetch_source() {
  [ -d "$tmp/src" ] && return 0
  mkdir -p "$tmp/src"
  curl -fsSL "https://codeload.github.com/$REPO/tar.gz/$ref" | tar -xz -C "$tmp/src" --strip-components 1
}

install_engine() {
  os=$(uname -s | tr '[:upper:]' '[:lower:]')
  arch=$(uname -m)
  [ "$arch" = arm64 ] && arch=aarch64
  asset="herdr-session-$os-$arch.tar.gz"
  run mkdir -p "$BIN_DIR"
  if [ -n "$tag" ] && curl -fsSL -o "$tmp/$asset" "https://github.com/$REPO/releases/download/$tag/$asset" 2>/dev/null; then
    # Extract beside it, then rename: a running hook keeps the old file, the next one gets the new.
    run tar -xzf "$tmp/$asset" -C "$tmp" herdr-session
    run cp "$tmp/herdr-session" "$BIN_DIR/.herdr-session.new"
    run mv -f "$BIN_DIR/.herdr-session.new" "$BIN_DIR/herdr-session"
  else
    # rustup's tools first: herdr-core's exec PATH lacks ~/.cargo/bin, and an old
    # system rustc in /usr/bin would otherwise be used by cargo.
    PATH="$HOME/.cargo/bin:$PATH"; export PATH
    cargo=$(command -v cargo || echo "$HOME/.cargo/bin/cargo")
    if [ ! -x "$cargo" ]; then
      say "No prebuilt herdr-session for $os-$arch${tag:+ in $tag}, and Rust is not installed on $(hostname)." >&2
      say "Install Rust (https://rustup.rs), then run this again." >&2
      exit 1
    fi
    say "Building herdr-session from source ($ref) on $(hostname)…"
    fetch_source
    run "$cargo" install --quiet --locked --path "$tmp/src/engine" --root "$HOME/.local" \
      || run "$cargo" install --quiet --path "$tmp/src/engine" --root "$HOME/.local"
  fi
  [ $dry = 1 ] || say "Engine: $("$BIN_DIR/herdr-session" version) on $(hostname)"
}

install_mods() {
  home="${HERDR_APP_HOME:-$HOME/.config/herdr-app}"
  fetch_source
  run mkdir -p "$home/mods"
  for m in $MODS; do
    if [ -L "$home/mods/$m" ]; then
      say "Mod $m is a link (a developer checkout); left as is."
      continue
    fi
    run rm -rf "$home/mods/$m"
    run cp -R "$tmp/src/mods/$m" "$home/mods/$m"
    say "Mod $m installed ($ref)."
  done
  for m in $RETIRED; do
    if [ -d "$home/mods/$m" ] && [ ! -L "$home/mods/$m" ]; then
      run rm -rf "$home/mods/$m"
      say "Mod $m removed (no longer used)."
    fi
  done
}

install_remote_engines() {
  command -v hc >/dev/null 2>&1 || return 0
  # Connected, non-temporary machines from herdr-core's own list.
  names=$(hc machine ls 2>/dev/null | tr '{' '\n' \
    | grep '"state":"connected"' | grep '"temporary":false' \
    | sed -n 's/.*"name":"\([^"]*\)".*/\1/p') || true
  for m in $names; do
    say "Installing the engine on $m…"
    if [ $dry = 1 ]; then say "would run: hc exec --on $m sh -c 'curl -fsSL $RAW | sh -s -- --engine'"; continue; fi
    hc exec --on "$m" sh -c "curl -fsSL '$RAW' | sh -s -- --engine" \
      || say "Could not install on $m. Run this there: curl -fsSL $RAW | sh -s -- --engine"
  done
}

app_here() {
  [ $mode = app ] && return 0
  command -v herdr-app >/dev/null 2>&1 && herdr-app status >/dev/null 2>&1
}

case $mode in
  engine)
    install_engine
    ;;
  *)
    install_engine
    if app_here; then
      # Engines everywhere first, mods last: replacing the mods reloads the
      # open dialogs, and they check every engine as they reload.
      install_remote_engines
      install_mods
      say ""
      say "Done. In herdr-app, open the command palette and run \"Share this space\" or \"Join a space\"."
    else
      say ""
      say "herdr-app is not running on $(hostname), so the mods were not installed here."
      say "On the computer where you use herdr-app, run:"
      say "  curl -fsSL $RAW | sh"
      say "or paste this into an agent in a space on that computer (This computer in the sidebar):"
      say "  Read ~/.config/herdr-app/AGENTS.md, then add this mod: https://github.com/$REPO"
    fi
    ;;
esac
