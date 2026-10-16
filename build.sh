#!/usr/bin/env bash
# build.sh — release spellbook; install --house → faeOS engine path
set -euo pipefail
ROOT="$(cd "$(dirname "$0")" && pwd)"
cd "$ROOT"
cargo build --release
BIN="$ROOT/target/release/spellbook"
echo "built: $BIN"
ls -la "$BIN"

if [[ "${1:-}" != "install" ]]; then
  exit 0
fi
shift || true
HOUSE=0
if [[ "${1:-}" == "--house" ]]; then
  HOUSE=1
fi

LIB="$HOME/.local/lib/faeos"
WRAP_SRC="$ROOT/scripts/spellbook"
mkdir -p "$LIB" "$HOME/bin"
cp -f "$BIN" "$LIB/spellbook"
chmod +x "$LIB/spellbook"

install_launcher() {
  local dest="$1"
  mkdir -p "$(dirname "$dest")"
  if [[ -f "$dest" ]] && file "$dest" 2>/dev/null | grep -q 'ELF'; then
    cp -f "$WRAP_SRC" "$dest"
  else
    cp -f "$WRAP_SRC" "$dest"
  fi
  chmod +x "$dest"
}

install_launcher "$HOME/bin/spellbook"
if [[ "$HOUSE" -eq 1 ]]; then
  for kit in "$HOME/faeOS/bin" "$HOME/faeos/bin"; do
    if [[ -d "$kit" ]]; then
      install_launcher "$kit/spellbook"
    fi
  done
fi

echo "installed engine → $LIB/spellbook"
echo "launcher        → $HOME/bin/spellbook"
