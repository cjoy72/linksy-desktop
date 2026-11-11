#!/usr/bin/env bash
# Build Linksy's service, command line and menu bar app and install them for the current
# user on macOS: the programs in ~/.local/bin, the service as a launchd agent that starts at
# login, the app in ~/Applications as a login item.
# Needs Rust (https://rustup.rs) and Xcode's command line tools (`xcode-select --install`).
# Usage: scripts/install-macos.sh [--no-service] [--no-app]
set -euo pipefail
cd "$(dirname "$0")/.."

case "$(uname -s)" in
  Darwin) ;;
  *) echo "This installs on macOS; on Linux use scripts/install-local.sh" >&2; exit 1 ;;
esac

SERVICE=1
APP=1
for arg in "$@"; do
  case "$arg" in
    --no-service) SERVICE=0 ;;
    --no-app) APP=0 ;;
    *) echo "Usage: scripts/install-macos.sh [--no-service] [--no-app]" >&2; exit 2 ;;
  esac
done

BIN="$HOME/.local/bin"
LOGS="${XDG_STATE_HOME:-$HOME/.local/state}/linksy"
AGENTS="$HOME/Library/LaunchAgents"
LABEL="mg.diary.linksy.linksyd"
PLIST="$AGENTS/$LABEL.plist"
DOMAIN="gui/$(id -u)"
APPS="$HOME/Applications"
BUNDLE="$APPS/Linksy.app"

if [ "$APP" = 1 ]; then
  cargo build --release -p linksyd -p linksy -p linksy-mac
else
  cargo build --release -p linksyd -p linksy
fi

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

# The menu bar app: in ~/Applications, started now and at every login (a login item the app
# registers itself, so System Settings › General › Login Items lists it).
if [ "$APP" = 1 ]; then
  mkdir -p "$APPS"
  pkill -x linksy-mac 2>/dev/null || true
  rm -rf "$BUNDLE"
  cp -R target/release/Linksy.app "$BUNDLE"
  "$BUNDLE/Contents/MacOS/linksy-mac" --login-item || echo "Couldn't add Linksy to the login items; add $BUNDLE in System Settings › General › Login Items." >&2
  open "$BUNDLE"
  echo "The Linksy menu bar app is running from $BUNDLE."
fi

case ":$PATH:" in
  *":$BIN:"*) ;;
  *) echo "Add $BIN to your PATH to run \`linksy\` from anywhere." ;;
esac
echo "The first time a phone connects, macOS asks whether linksyd may accept incoming connections: allow it."
