#!/usr/bin/env bash
# Development install of the itamiiru portal backend. Re-run after every rebuild.
# Run as your normal user, not with sudo: it builds with your own cargo and
# asks for your password only to install the .portal file into /usr/share.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(id -u)" -eq 0 ]; then
  echo "Run this as your normal user, not with sudo." >&2
  echo "It uses your cargo, and asks for sudo itself for the one system file." >&2
  exit 1
fi
# rustup's cargo lives in ~/.cargo/bin, which some shells don't add to PATH.
if ! command -v cargo > /dev/null && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi

cargo build --release -p itamiiru -p itamiiru-portal

BIN_DIR="$HOME/.local/bin"
SERVICE_DIR="${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services"
CONF_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/xdg-desktop-portal"
mkdir -p "$BIN_DIR" "$SERVICE_DIR" "$CONF_DIR"

# The portal finds the picker next to its own executable.
install -m755 target/release/itami "$BIN_DIR/itami"
install -m755 target/release/xdg-desktop-portal-itamiiru "$BIN_DIR/xdg-desktop-portal-itamiiru"

# Per-user D-Bus activation file.
sed "s|@BINDIR@|$BIN_DIR|" data/org.freedesktop.impl.portal.desktop.itamiiru.service \
  > "$SERVICE_DIR/org.freedesktop.impl.portal.desktop.itamiiru.service"

# The frontend reads .portal files from its system data directory.
sudo install -Dm644 data/itamiiru.portal /usr/share/xdg-desktop-portal/portals/itamiiru.portal

if [ ! -f "$CONF_DIR/portals.conf" ]; then
  cp data/portals.conf.example "$CONF_DIR/portals.conf"
  echo "Created $CONF_DIR/portals.conf. Edit 'default=' to your usual backend."
elif ! grep -q 'FileChooser=itamiiru' "$CONF_DIR/portals.conf"; then
  echo "NOTE: add this line under [preferred] in $CONF_DIR/portals.conf:"
  echo "  org.freedesktop.impl.portal.FileChooser=itamiiru"
fi

# Stop any running backend so the new binary is used (-f: names over 15 chars).
pkill -f xdg-desktop-portal-itamiiru || true
systemctl --user restart xdg-desktop-portal
echo "Installed. Test with: GTK_USE_PORTAL=1 GDK_DEBUG=portals zenity --file-selection"
