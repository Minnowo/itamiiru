#!/usr/bin/env bash
# Builds folders for checking Phase 1 by hand (PLAN.md section 11).
# Usage: scripts/make-test-dirs.sh [DEST]   (default: /tmp/itamiiru-test)
# Then: cargo run -p itamiiru -- /tmp/itamiiru-test
set -euo pipefail
DEST="${1:-/tmp/itamiiru-test}"
mkdir -p "$DEST"

# 50,000 empty files, for scrolling and sorting speed.
BIG="$DEST/50k-files"
if [ ! -d "$BIG" ]; then
  mkdir -p "$BIG"
  (cd "$BIG" && seq -f 'file%05g.txt' 1 50000 | xargs touch)
fi

# Odd names. Non-ASCII names are written as UTF-8 byte escapes, so this
# script stays ASCII.
ODD="$DEST/odd-names"
mkdir -p "$ODD"
touch "$ODD"/$'bad\xffname.txt'
touch "$ODD"/$'\xe6\x97\xa5\xe6\x9c\xac\xe8\xaa\x9e.txt'   # Japanese "nihongo"
touch "$ODD"/$'\xe4\xb8\xad\xe6\x96\x87.png'               # Chinese "zhongwen"
touch "$ODD"/$'\xed\x95\x9c\xea\xb5\xad\xec\x96\xb4.pdf'   # Korean "hangugeo"
touch "$ODD"/$'\xf0\x9f\x98\x80 smile.mp3'                  # an emoji
touch "$ODD/with spaces.zip" "$ODD/.hidden-file" "$ODD/no-extension"
touch "$ODD/file2.txt" "$ODD/file10.txt" "$ODD/File1.TXT"
printf '#!/bin/sh\necho hi\n' > "$ODD/run.sh" && chmod +x "$ODD/run.sh"
ln -sf "$ODD/file2.txt" "$ODD/link-to-file"
ln -sf "$DEST/50k-files" "$ODD/link-to-folder"
ln -sf "$ODD/missing" "$ODD/broken-link"
mkdir -p "$ODD/a folder"

# A folder you can't open, for the inline error.
mkdir -p "$DEST/no-access" && chmod 000 "$DEST/no-access"

echo "Made $DEST. Try: cargo run -p itamiiru -- $DEST"
echo "Slow drive test: ITAMI_DEBUG_SLOW_FS=3000 cargo run -p itamiiru -- $DEST"
