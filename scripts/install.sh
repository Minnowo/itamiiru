#!/usr/bin/env bash
# Installs itamiiru for the current user: the explorer, the picker and the
# xdg-desktop-portal backend, so file dialogs in other apps show itamiiru.
#
# Works from a source checkout (builds with cargo) or from a release
# tarball made by scripts/package.sh (uses its prebuilt bin/ folder).
#
# Usage: scripts/install.sh [--no-env] [--uninstall]
#   --no-env     don't touch the session environment (~/.xsessionrc or
#                ~/.config/environment.d); set GTK_USE_PORTAL yourself
#   --uninstall  remove everything this script installed
#
# Run as your normal user, not with sudo. It asks for your password once,
# for the one file that must go in /usr/share.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/.." && pwd)"

USE_ENV=1
UNINSTALL=0
for arg in "$@"; do
  case "$arg" in
    --no-env) USE_ENV=0 ;;
    --uninstall) UNINSTALL=1 ;;
    -h|--help) sed -n '2,14p' "$0"; exit 0 ;;
    *) echo "Unknown option: $arg (try --help)" >&2; exit 1 ;;
  esac
done

if [ "$(id -u)" -eq 0 ]; then
  echo "Run this as your normal user, not with sudo." >&2
  echo "It installs into your home folder and asks for sudo itself for the one system file." >&2
  exit 1
fi

BIN_DIR="$HOME/.local/bin"
DATA_HOME="${XDG_DATA_HOME:-$HOME/.local/share}"
CONFIG_HOME="${XDG_CONFIG_HOME:-$HOME/.config}"
SERVICE_DIR="$DATA_HOME/dbus-1/services"
SERVICE_FILE="$SERVICE_DIR/org.freedesktop.impl.portal.desktop.itamiiru.service"
PORTAL_FILE=/usr/share/xdg-desktop-portal/portals/itamiiru.portal
PORTALS_CONF="$CONFIG_HOME/xdg-desktop-portal/portals.conf"
ENV_D_FILE="$CONFIG_HOME/environment.d/60-itamiiru.conf"
XSESSIONRC="$HOME/.xsessionrc"
# Marks the lines this script adds to files it doesn't own.
BEGIN_MARK="# >>> itamiiru >>>"
END_MARK="# <<< itamiiru <<<"

say() { printf '%s\n' "$*"; }

# Stops a running backend so the next file dialog starts the new one.
# Anchored so it can't match this script's own command line.
restart_portal() {
  pkill -f '^[^ ]*xdg-desktop-portal-itamiiru( |$)' || true
  if ! systemctl --user restart xdg-desktop-portal 2> /dev/null; then
    say "NOTE: couldn't restart xdg-desktop-portal; log out and back in instead."
  fi
}

# Removes the marked block from a file, if it's there.
remove_block() {
  local file="$1"
  if [ -f "$file" ] && grep -qF "$BEGIN_MARK" "$file"; then
    sed -i "/^$BEGIN_MARK\$/,/^$END_MARK\$/d" "$file"
    say "Removed the itamiiru lines from $file"
    # Nothing else was in it: it was ours.
    if [ ! -s "$file" ]; then
      rm -f "$file"
    fi
  fi
}

if [ "$UNINSTALL" -eq 1 ]; then
  rm -f "$BIN_DIR/itami" "$BIN_DIR/xdg-desktop-portal-itamiiru" "$SERVICE_FILE"
  if [ -f "$PORTALS_CONF" ] && grep -q "^# Created by itamiiru's install.sh" "$PORTALS_CONF"; then
    rm -f "$PORTALS_CONF"
  elif [ -f "$PORTALS_CONF" ]; then
    sed -i '/^org\.freedesktop\.impl\.portal\.FileChooser=itamiiru$/d' "$PORTALS_CONF"
  fi
  if [ -f "$PORTALS_CONF.before-itamiiru" ]; then
    say "Your portals.conf from before the install is in $PORTALS_CONF.before-itamiiru"
  fi
  remove_block "$XSESSIONRC"
  if [ -f "$ENV_D_FILE" ] && grep -qF "$BEGIN_MARK" "$ENV_D_FILE"; then
    rm -f "$ENV_D_FILE"
  fi
  if [ -f "$PORTAL_FILE" ]; then
    say "Removing $PORTAL_FILE (needs sudo)"
    sudo rm -f "$PORTAL_FILE" || say "NOTE: remove $PORTAL_FILE by hand."
  fi
  restart_portal
  say "Uninstalled. Your settings in $CONFIG_HOME/itamiiru were left alone."
  say "Log out and back in to drop GTK_USE_PORTAL from the session."
  exit 0
fi

# --- Find or build the binaries ---------------------------------------------
if [ -x "$ROOT/bin/itami" ] && [ -x "$ROOT/bin/xdg-desktop-portal-itamiiru" ]; then
  BUILT="$ROOT/bin"
elif [ -f "$ROOT/Cargo.toml" ]; then
  # rustup's cargo lives in ~/.cargo/bin, which some shells don't add to PATH.
  if ! command -v cargo > /dev/null && [ -f "$HOME/.cargo/env" ]; then
    . "$HOME/.cargo/env"
  fi
  if ! command -v cargo > /dev/null; then
    say "cargo not found. Install Rust from https://rustup.rs, or use a release tarball." >&2
    exit 1
  fi
  (cd "$ROOT" && cargo build --release -p itamiiru -p itamiiru-portal)
  BUILT="$ROOT/target/release"
else
  say "No bin/ folder and no Cargo.toml next to this script; nothing to install." >&2
  exit 1
fi

# --- Check the portal frontend is there ---------------------------------------
FRONTEND=""
for path in /usr/libexec/xdg-desktop-portal /usr/lib/xdg-desktop-portal /usr/lib/xdg-desktop-portal/xdg-desktop-portal; do
  if [ -x "$path" ] && [ ! -d "$path" ]; then
    FRONTEND="$path"
  fi
done
if [ -z "$FRONTEND" ]; then
  say "WARNING: xdg-desktop-portal doesn't seem to be installed."
  say "  Debian/Ubuntu: sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk"
  say "  Fedora: sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk"
  say "  Arch: sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk"
  say "Installing anyway; dialogs will use itamiiru once it's there."
fi

# --- Binaries and the D-Bus activation file ---------------------------------
mkdir -p "$BIN_DIR" "$SERVICE_DIR"
# The backend finds the picker next to its own executable.
install -m755 "$BUILT/itami" "$BIN_DIR/itami"
install -m755 "$BUILT/xdg-desktop-portal-itamiiru" "$BIN_DIR/xdg-desktop-portal-itamiiru"
sed "s|@BINDIR@|$BIN_DIR|" "$ROOT/data/org.freedesktop.impl.portal.desktop.itamiiru.service" \
  > "$SERVICE_FILE"
say "Installed itami and xdg-desktop-portal-itamiiru in $BIN_DIR"

# --- The .portal file: the frontend only reads its own system folder -----------
if cmp -s "$ROOT/data/itamiiru.portal" "$PORTAL_FILE"; then
  say "$PORTAL_FILE is already up to date"
else
  say "Installing $PORTAL_FILE (needs sudo)"
  sudo install -Dm644 "$ROOT/data/itamiiru.portal" "$PORTAL_FILE"
fi

# --- portals.conf: itamiiru for file dialogs, everything else unchanged -------
# The default backend: whatever the desktop's own portals.conf chooses,
# else gtk.
default_backend() {
  local desktop conf value
  desktop="$(printf '%s' "${XDG_CURRENT_DESKTOP:-}" | cut -d: -f1 | tr '[:upper:]' '[:lower:]')"
  for conf in "/usr/share/xdg-desktop-portal/${desktop:-none}-portals.conf" /usr/share/xdg-desktop-portal/portals.conf; do
    if [ -f "$conf" ]; then
      value="$(sed -n 's/^default=\(.*\)$/\1/p' "$conf" | head -1)"
      if [ -n "$value" ]; then
        printf '%s' "$value"
        return
      fi
    fi
  done
  printf 'gtk'
}

LINE="org.freedesktop.impl.portal.FileChooser=itamiiru"
mkdir -p "$(dirname "$PORTALS_CONF")"
if [ ! -f "$PORTALS_CONF" ]; then
  {
    say "# Created by itamiiru's install.sh (--uninstall deletes it)."
    say "# Use itamiiru for file dialogs; everything else stays on the usual backend."
    say "[preferred]"
    say "default=$(default_backend)"
    say "$LINE"
  } > "$PORTALS_CONF"
  say "Created $PORTALS_CONF"
elif grep -qxF "$LINE" "$PORTALS_CONF"; then
  say "$PORTALS_CONF already chooses itamiiru"
else
  cp "$PORTALS_CONF" "$PORTALS_CONF.before-itamiiru"
  if grep -qx '\[preferred\]' "$PORTALS_CONF"; then
    # Remove any other FileChooser choice, then add ours under [preferred].
    sed -i '/^org\.freedesktop\.impl\.portal\.FileChooser=/d' "$PORTALS_CONF"
    sed -i "/^\[preferred\]\$/a $LINE" "$PORTALS_CONF"
  else
    printf '\n[preferred]\n%s\n' "$LINE" >> "$PORTALS_CONF"
  fi
  say "Added itamiiru to $PORTALS_CONF (old copy: $PORTALS_CONF.before-itamiiru)"
fi

# --- Session environment: make GTK apps use the portal ------------------------
if [ "$USE_ENV" -eq 1 ]; then
  if [ -f /etc/X11/Xsession ] && [ "${XDG_SESSION_TYPE:-x11}" = "x11" ]; then
    # X sessions started by /etc/X11/Xsession (XFCE, MATE, Cinnamon on
    # Debian and Ubuntu) read ~/.xsessionrc, not environment.d.
    if [ -f "$XSESSIONRC" ] && grep -q 'GTK_USE_PORTAL' "$XSESSIONRC"; then
      say "$XSESSIONRC already sets GTK_USE_PORTAL"
    else
      {
        say "$BEGIN_MARK"
        grep -v '^#' "$ROOT/data/xsessionrc.example" | sed '/^$/d'
        say "$END_MARK"
      } >> "$XSESSIONRC"
      say "Added GTK_USE_PORTAL to $XSESSIONRC"
    fi
  else
    if [ -f "$ENV_D_FILE" ]; then
      say "$ENV_D_FILE already exists"
    else
      mkdir -p "$(dirname "$ENV_D_FILE")"
      {
        say "$BEGIN_MARK (written by itamiiru's install.sh; delete to undo)"
        say "GTK_USE_PORTAL=1"
        say "GDK_DEBUG=portals"
      } > "$ENV_D_FILE"
      say "Created $ENV_D_FILE"
    fi
  fi
fi

restart_portal

case ":$PATH:" in
  *":$BIN_DIR:"*) ;;
  *) say "NOTE: $BIN_DIR isn't on your PATH; add it to run 'itami' from a terminal." ;;
esac
say ""
say "Done. Next:"
say "  1. Log out and back in, so apps pick up GTK_USE_PORTAL."
say "  2. Firefox: about:config -> widget.use-xdg-desktop-portal.file-picker = 1"
say "  3. LibreOffice: Tools > Options > General > untick 'Use LibreOffice dialogs'"
say "  4. Try it: GTK_USE_PORTAL=1 zenity --file-selection"
