#!/usr/bin/env bash
# Remove what scripts/install-macos.sh installed. Keeps ~/.local/share/linksy (identity and
# paired devices) unless --purge is given.
set -euo pipefail

BIN="$HOME/.local/bin"
LABEL="mg.diary.linksy.linksyd"
PLIST="$HOME/Library/LaunchAgents/$LABEL.plist"
DOMAIN="gui/$(id -u)"

launchctl bootout "$DOMAIN/$LABEL" 2>/dev/null || true
rm -f "$PLIST" "$BIN/linksyd" "$BIN/linksy"
if [ "${1:-}" = "--purge" ]; then
  rm -rf "${XDG_DATA_HOME:-$HOME/.local/share}/linksy" "${XDG_STATE_HOME:-$HOME/.local/state}/linksy"
  echo "Removed Linksy, its identity and paired devices."
else
  echo "Removed Linksy. Its identity and paired devices stay in ${XDG_DATA_HOME:-$HOME/.local/share}/linksy (--purge removes them)."
fi
