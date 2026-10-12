#!/usr/bin/env bash
# End-to-end test of the FileManager1 service ("Show in folder", P7.5) on a
# private bus, so it runs even while Thunar owns the name on the real one.
# The service is started by D-Bus activation from data/'s service file, and
# a fake itami records the windows it would open.
# Usage: scripts/test-file-manager.sh   (builds the portal first)
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q -p itamiiru-portal
if [ "${1:-}" != "--inside" ]; then
  DIR="$(mktemp -d)"
  trap 'rm -rf "$DIR"' EXIT
  mkdir -p "$DIR/services"
  sed "s|@BINDIR@|$PWD/target/debug|" data/org.freedesktop.FileManager1.service \
    > "$DIR/services/org.freedesktop.FileManager1.service"
  cat > "$DIR/bus.conf" <<CONF
<!DOCTYPE busconfig PUBLIC "-//freedesktop//DTD D-BUS Bus Configuration 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/busconfig.dtd">
<busconfig>
  <type>session</type>
  <listen>unix:tmpdir=$DIR</listen>
  <servicedir>$DIR/services</servicedir>
  <policy context="default">
    <allow send_destination="*" eavesdrop="true"/>
    <allow eavesdrop="true"/>
    <allow own="*"/>
  </policy>
</busconfig>
CONF
  # The fake itami: one file per window, holding its arguments and the
  # startup id it was given.
  cat > "$DIR/fake-itami.sh" <<FAKE
#!/bin/sh
out="\$(mktemp "$DIR/window.XXXXXX")"
printf '%s\n' "\$@" "startup=\$DESKTOP_STARTUP_ID" > "\$out"
FAKE
  chmod +x "$DIR/fake-itami.sh"
  export ITAMI_BIN="$DIR/fake-itami.sh" TEST_DIR="$DIR"
  dbus-run-session --config-file="$DIR/bus.conf" -- "$0" --inside
  exit $?
fi

# --- From here on we're inside the private bus. ---
DIR="$TEST_DIR"
DEST="--session --dest org.freedesktop.FileManager1 --object-path /org/freedesktop/FileManager1"
IFACE="org.freedesktop.FileManager1"
FAILED=0
check() {
  if [ "$2" = "$3" ]; then
    echo "ok   $1"
  else
    echo "FAIL $1"
    echo "  expected: $(printf '%s' "$3" | tr '\n' '|')"
    echo "  got:      $(printf '%s' "$2" | tr '\n' '|')"
    FAILED=1
  fi
}
# Waits for N window files, then prints them sorted and removes them.
windows() {
  for _ in $(seq 50); do
    [ "$(ls "$DIR" | grep -c '^window\.')" -ge "$1" ] && break
    sleep 0.1
  done
  sleep 0.2
  cat "$DIR"/window.* 2> /dev/null | sort
  rm -f "$DIR"/window.*
}

# The first call starts the service through its activation file.
gdbus call $DEST --method "$IFACE.ShowItems" \
  "['file:///home/me/Downloads/b.zip', 'file:///home/me/Downloads/a%20b.pdf', 'file:///etc/hosts']" \
  "launch-1" > /dev/null
check "ShowItems: one window per folder, items selected" "$(windows 2)" "$(printf '%s\n' \
  /etc --select /etc/hosts startup=launch-1 \
  /home/me/Downloads --select '/home/me/Downloads/b.zip' --select '/home/me/Downloads/a b.pdf' startup=launch-1 | sort)"

gdbus call $DEST --method "$IFACE.ShowFolders" "['file:///tmp/some%20folder', 'http://x/']" "" > /dev/null
check "ShowFolders: folders only, other URIs skipped" "$(windows 1)" "$(printf '%s\n' '/tmp/some folder' startup= | sort)"

gdbus call $DEST --method "$IFACE.ShowItemProperties" "['file:///etc/fstab']" "" > /dev/null
check "ShowItemProperties shows the item" "$(windows 1)" "$(printf '%s\n' /etc --select /etc/fstab startup= | sort)"

XML="$(gdbus call $DEST --method org.freedesktop.DBus.Introspectable.Introspect)"
check "Introspect lists the interface" "$(echo "$XML" | grep -c 'interface name=.org.freedesktop.FileManager1.')" "1"

# dbus-send, because gdbus checks the types itself before sending.
ERR="$(dbus-send --session --print-reply --dest=org.freedesktop.FileManager1 \
  /org/freedesktop/FileManager1 "$IFACE.ShowItems" string:a string:b 2>&1 || true)"
check "wrong arguments are an error" "$(echo "$ERR" | grep -c InvalidArgs)" "1"

check "nothing else was opened" "$(windows 0)" ""
exit "$FAILED"
