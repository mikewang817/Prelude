#!/bin/bash
set -euo pipefail

REPO="mikewang817/Prelude"
INSTALL_DIR="${PRELUDE_INSTALL_DIR:-$HOME/.local/bin}"
FZF_VERSION="0.74.2"
GHOSTTY_VERSION="1.3.1"
GHOSTTY_SHA256="18cff2b0a6cee90eead9c7d3064e808a252a40baf214aa752c1ecb793b8f5f69"

say() { printf '  %s\n' "$*"; }
die() { printf 'prelude: %s\n' "$*" >&2; exit 1; }

[[ "$(uname -s)" == "Darwin" ]] || die "Prelude currently supports macOS only"
case "$(uname -m)" in
  arm64)
    PRELUDE_TARGET="aarch64-apple-darwin"
    FZF_ARCH="arm64"
    ;;
  x86_64)
    PRELUDE_TARGET="x86_64-apple-darwin"
    FZF_ARCH="amd64"
    ;;
  *) die "unsupported Mac architecture: $(uname -m)" ;;
esac

case "$INSTALL_DIR" in
  *$'\n'*|*'"'*) die "PRELUDE_INSTALL_DIR may not contain a newline or double quote" ;;
esac

TMP="$(mktemp -d "${TMPDIR:-/tmp}/prelude-install.XXXXXX")"
MOUNT=""
cleanup() {
  if [[ -n "$MOUNT" && -d "$MOUNT" ]]; then
    hdiutil detach "$MOUNT" -quiet >/dev/null 2>&1 || true
  fi
  rm -rf "$TMP"
}
trap cleanup EXIT INT TERM

verify() {
  local file="$1" expected="$2" actual
  actual="$(shasum -a 256 "$file" | awk '{print $1}')"
  [[ "$actual" == "$expected" ]] || die "checksum verification failed for $(basename "$file")"
}

printf '\nPrelude installer\n\n'
mkdir -p "$INSTALL_DIR"

ASSET="prelude-${PRELUDE_TARGET}.tar.gz"
BASE="https://github.com/${REPO}/releases/latest/download"
say "Downloading Prelude for $(uname -m)…"
curl -fL --retry 3 --progress-bar "$BASE/$ASSET" -o "$TMP/$ASSET"
curl -fsSL --retry 3 "$BASE/checksums.txt" -o "$TMP/checksums.txt"
EXPECTED="$(awk -v name="$ASSET" '$2 == name { print $1 }' "$TMP/checksums.txt")"
[[ -n "$EXPECTED" ]] || die "the release does not contain a checksum for $ASSET"
verify "$TMP/$ASSET" "$EXPECTED"
tar -xzf "$TMP/$ASSET" -C "$TMP"
install -m 755 "$TMP/prelude" "$INSTALL_DIR/prelude"

# Prelude uses modern fzf footer and transform bindings. Keep an existing fzf
# when it supports them; otherwise install a private, verified copy beside
# Prelude without requiring Homebrew or a Rust toolchain.
if ! command -v fzf >/dev/null 2>&1 || ! fzf --help 2>&1 | grep -q -- '--footer'; then
  FZF_ASSET="fzf-${FZF_VERSION}-darwin_${FZF_ARCH}.tar.gz"
  FZF_BASE="https://github.com/junegunn/fzf/releases/download/v${FZF_VERSION}"
  say "Installing fzf ${FZF_VERSION}…"
  curl -fL --retry 3 --progress-bar "$FZF_BASE/$FZF_ASSET" -o "$TMP/$FZF_ASSET"
  curl -fsSL --retry 3 "$FZF_BASE/fzf_${FZF_VERSION}_checksums.txt" -o "$TMP/fzf-checksums.txt"
  FZF_EXPECTED="$(awk -v name="$FZF_ASSET" '$2 == name { print $1 }' "$TMP/fzf-checksums.txt")"
  [[ -n "$FZF_EXPECTED" ]] || die "could not verify the fzf download"
  verify "$TMP/$FZF_ASSET" "$FZF_EXPECTED"
  tar -xzf "$TMP/$FZF_ASSET" -C "$TMP" fzf
  install -m 755 "$TMP/fzf" "$INSTALL_DIR/fzf"
else
  say "Using $(command -v fzf)"
fi

# The global panel is a Ghostty quick terminal. Install the official signed app
# in the user's Applications folder when it is not already present, so setup
# needs neither Homebrew nor administrator access.
if [[ ! -d /Applications/Ghostty.app && ! -d "$HOME/Applications/Ghostty.app" ]]; then
  say "Installing Ghostty ${GHOSTTY_VERSION} in ~/Applications…"
  DMG="$TMP/Ghostty.dmg"
  curl -fL --retry 3 --progress-bar \
    "https://release.files.ghostty.org/${GHOSTTY_VERSION}/Ghostty.dmg" -o "$DMG"
  verify "$DMG" "$GHOSTTY_SHA256"
  MOUNT="$TMP/ghostty-volume"
  mkdir -p "$MOUNT" "$HOME/Applications"
  hdiutil attach "$DMG" -nobrowse -readonly -mountpoint "$MOUNT" -quiet
  [[ -d "$MOUNT/Ghostty.app" ]] || die "the Ghostty disk image did not contain Ghostty.app"
  ditto "$MOUNT/Ghostty.app" "$HOME/Applications/Ghostty.app"
  hdiutil detach "$MOUNT" -quiet
  MOUNT=""
else
  say "Using the installed Ghostty app"
fi

# Keep both entry points after the first shell restart. The global panel below
# is started now and does not wait for that restart.
ZSHRC="$HOME/.zshrc"
MARKER="# >>> Prelude >>>"
if ! grep -Fq "$MARKER" "$ZSHRC" 2>/dev/null; then
  {
    printf '\n%s\n' "$MARKER"
    printf 'export PATH="%s:$PATH"\n' "$INSTALL_DIR"
    printf 'eval "$(prelude init zsh)"\n'
    printf '# <<< Prelude <<<\n'
  } >> "$ZSHRC"
  say "Added Ctrl+R integration to ~/.zshrc"
else
  say "Ctrl+R integration is already in ~/.zshrc"
fi

export PATH="$INSTALL_DIR:$PATH"
say "Starting the global panel…"
if (: </dev/tty) 2>/dev/null; then
  "$INSTALL_DIR/prelude" global install </dev/tty
else
  "$INSTALL_DIR/prelude" global install
fi

printf '\nReady.\n'
printf '  Cmd+Shift+Space  opens Prelude anywhere\n'
printf '  Ctrl+R           opens Prelude in a new zsh prompt\n\n'
