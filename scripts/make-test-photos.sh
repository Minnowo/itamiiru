#!/usr/bin/env bash
# Builds a folder of photo-sized JPEGs for checking Phase 5 thumbnails by
# hand (PLAN.md section 11): scrolling stays smooth, on-screen thumbnails
# fill in first, and a second visit comes from the cache.
# Usage: scripts/make-test-photos.sh [COUNT] [DEST]
#   (defaults: 2000 and /tmp/itamiiru-photos; about 300 KB per photo)
# Then: cargo run --release -p itamiiru -- /tmp/itamiiru-photos
# and pick View: Large icons.
set -euo pipefail
COUNT="${1:-2000}"
DEST="${2:-/tmp/itamiiru-photos}"

if ! command -v ffmpeg > /dev/null; then
  echo "Needs ffmpeg (apt install ffmpeg)." >&2
  exit 1
fi
mkdir -p "$DEST"
# Each frame of ffmpeg's moving test pattern becomes one photo. A little
# noise makes them decode like real photos rather than flat colors.
ffmpeg -loglevel error -y -f lavfi -i "testsrc2=s=1920x1440:r=25" \
  -frames:v "$COUNT" -vf "noise=alls=6:allf=t,format=yuvj420p" -q:v 4 \
  "$DEST/photo%05d.jpg"
# A few tall ones and a wide one, so the thumbnails aren't all one shape.
ffmpeg -loglevel error -y -f lavfi -i "testsrc2=s=1080x1920" -frames:v 1 \
  -q:v 4 "$DEST/portrait.jpg"
ffmpeg -loglevel error -y -f lavfi -i "testsrc2=s=4000x800" -frames:v 1 \
  -q:v 4 "$DEST/panorama.jpg"
echo "Made $COUNT photos in $DEST"
