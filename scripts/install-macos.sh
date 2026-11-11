#!/usr/bin/env bash
# Build Linksy's service and command line and install them for the current user on macOS:
# the programs in ~/.local/bin, the service as a launchd agent that starts at login.
# Needs Rust (https://rustup.rs) and Xcode's command line tools (`xcode-select --install`).
# Usage: scripts/install-macos.sh [--no-service]
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Darwin) ;;
  *) echo "This installs on macOS; on Linux use scripts/install-local.sh" >&2; exit 1 ;;
esac

SERVICE=1
[ "${1:-}" = "--no-service" ] && SERVICE=0

BIN="$HOME/.local/bin"
LOGS="${XDG_STATE_HOME:-$HOME/.local/state}/linksy"
AGENTS="$HOME/Library/LaunchAgents"
LABEL="mg.diary.linksy.linksyd"
PLIST="$AGENTS/$LABEL.plist"
DOMAIN="gui/$(id -u)"

cargo build --release -p linksyd -p linksy

mkdir -p "$BIN" "$LOGS" "$AGENTS"
# A running service keeps its old binary open: unload it before the files change.
if launchctl print "$DOMAIN/$LABEL" >/dev/null 2>&1; then
  launchctl bootout "$DOMAIN/$LABEL" || true
fi
install -m755 target/release/linksyd "$BIN/linksyd"
install -m755 target/release/linksy "$BIN/linksy"

sed -e "s|@BINDIR@|$BIN|g" -e "s|@LOGDIR@|$LOGS|g" packaging/macos/$LABEL.plist > "$PLIST"

if [ "$SERVICE" = 1 ]; then
  launchctl bootstrap "$DOMAIN" "$PLIST"
  launchctl kickstart "$DOMAIN/$LABEL"
  sleep 1
  if launchctl print "$DOMAIN/$LABEL" 2>/dev/null | grep -q "state = running"; then
    echo "Installed. The service is running (log: $LOGS/linksyd.log)."
  else
    echo "Installed, but the service isn't running yet: see $LOGS/linksyd.log" >&2
  fi
else
  echo "Installed to $BIN (service not started: launchctl bootstrap $DOMAIN \"$PLIST\" starts it)."
fi

case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Add $BIN to your PATH to run \`linksy\` from anywhere." ;;
esac
echo "The first time a phone connects, macOS asks whether linksyd may accept incoming connections: allow it."
