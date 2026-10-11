#!/usr/bin/env bash
# End-to-end test of the portal backend on your session bus, without a
# window: a fake picker stands in for `itami --pick`.
# Usage: scripts/test-portal.sh   (builds the portal first)
# Don't run it while the real backend is active: the bus name is taken.
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build -q -p itamiiru-portal

DIR="$(mktemp -d)"
PORTAL_PID=""
cleanup() {
  if [ -n "$PORTAL_PID" ]; then kill "$PORTAL_PID" 2>/dev/null || true; fi
  rm -rf "$DIR"
}
trap cleanup EXIT

# The fake picker records the request, then answers depending on it.
cat > "$DIR/fake-picker.sh" <<FAKE
#!/bin/sh
cat > "$DIR/request.txt"
if grep -q "^title Slow" "$DIR/request.txt"; then exec sleep 30; fi
if grep -q "^title Cancel" "$DIR/request.txt"; then exit 1; fi
if grep -q "^mode save-files" "$DIR/request.txt"; then
  printf 'itamiiru-protocol 1\nuri file:///tmp/outdir\n'
  exit 0
fi
printf 'itamiiru-protocol 1\nuri file:///tmp/a%%2520b.txt\nfilter 0\nchoice enc latin1\n'
FAKE
chmod +x "$DIR/fake-picker.sh"

ITAMI_BIN="$DIR/fake-picker.sh" target/debug/xdg-desktop-portal-itamiiru 2> "$DIR/portal.log" &
PORTAL_PID=$!
gdbus wait --session --timeout 5 org.freedesktop.impl.portal.desktop.itamiiru

DEST="--session --dest org.freedesktop.impl.portal.desktop.itamiiru"
OBJ="--object-path /org/freedesktop/portal/desktop"
FC="org.freedesktop.impl.portal.FileChooser"
REQ="/org/freedesktop/portal/desktop/request/1_1"
FAILED=0

check() {
  # check NAME OUTPUT EXPECTED-SUBSTRING
  if [[ "$2" == *"$3"* ]]; then
    echo "ok    $1"
  else
    echo "FAIL  $1"
    echo "      got:      $2"
    echo "      expected: $3"
    FAILED=1
  fi
}

OUT=$(gdbus call $DEST $OBJ --method $FC.OpenFile $REQ/t1 "org.test" "x11:abc" "Pick a file" \
  "{'multiple': <true>, 'current_folder': <b'/tmp'>, 'filters': <[('Text', [(uint32 0, '*.txt')]), ('Images', [(uint32 1, 'image/png')])]>, 'choices': <[('enc', 'Encoding', [('utf8','UTF-8'),('latin1','Latin-1')], 'utf8')]>}")
check "OpenFile answer" "$OUT" "(uint32 0, {'uris': <['file:///tmp/a%20b.txt']>, 'choices': <[('enc', 'latin1')]>, 'current_filter': <('Text', [(uint32 0, '*.txt')])>})"
REQUEST=$(cat "$DIR/request.txt")
check "OpenFile request: mode" "$REQUEST" "mode open"
check "OpenFile request: folder" "$REQUEST" "folder /tmp"
check "OpenFile request: mime filter" "$REQUEST" "filter Images mime:image/png"
check "OpenFile request: choice" "$REQUEST" "choice enc Encoding utf8 utf8 UTF-8 latin1 Latin-1"

OUT=$(gdbus call $DEST $OBJ --method $FC.SaveFiles $REQ/t2 "org.test" "" "Save all" "{'files': <[b'one.txt', b'two.txt']>}")
check "SaveFiles joins names" "$OUT" "['file:///tmp/outdir/one.txt', 'file:///tmp/outdir/two.txt']"

OUT=$(gdbus call $DEST $OBJ --method $FC.SaveFile $REQ/t3 "org.test" "" "Cancel me" "{'current_name': <'x.txt'>}")
check "Cancel gives response 1" "$OUT" "(uint32 1,"

gdbus call --timeout 20 $DEST $OBJ --method $FC.OpenFile $REQ/t4 "org.test" "" "Slow" "{}" > "$DIR/slow.out" 2>&1 &
CALL_PID=$!
for _ in $(seq 1 100); do
  grep -q "^title Slow" "$DIR/request.txt" 2>/dev/null && break
  gdbus call --session --dest org.freedesktop.DBus --object-path /org/freedesktop/DBus --method org.freedesktop.DBus.GetId > /dev/null
done
gdbus call $DEST --object-path $REQ/t4 --method org.freedesktop.impl.portal.Request.Close > /dev/null
wait "$CALL_PID"
check "Close answers the open call with 2" "$(cat "$DIR/slow.out")" "(uint32 2,"
if pgrep -f "^sleep 30" > /dev/null; then
  echo "FAIL  Close kills the picker"; FAILED=1
else
  echo "ok    Close kills the picker"
fi

check "Ping" "$(gdbus call $DEST $OBJ --method org.freedesktop.DBus.Peer.Ping)" "()"
check "Unknown method" "$(gdbus call $DEST $OBJ --method $FC.Nope 2>&1 || true)" "UnknownMethod"

if [ "$FAILED" -ne 0 ]; then
  echo "--- portal log:"; cat "$DIR/portal.log"
  exit 1
fi
echo "All portal checks passed."
