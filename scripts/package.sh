#!/usr/bin/env bash
# Builds a release tarball for installing on another machine without Rust:
#   target/itamiiru-<version>-<arch>-linux.tar.gz
# On the other machine: tar xf itamiiru-*.tar.gz && cd itamiiru-*/ && ./install.sh
# The binaries need glibc as new as this machine's (check: ldd --version).
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo > /dev/null && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
cargo build --release -p itamiiru -p itamiiru-portal

VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
NAME="itamiiru-$VERSION-$(uname -m)-linux"
STAGE="target/package/$NAME"
rm -rf "$STAGE"
mkdir -p "$STAGE/bin" "$STAGE/scripts" "$STAGE/data" "$STAGE/docs"

cp target/release/itami target/release/xdg-desktop-portal-itamiiru "$STAGE/bin/"
# Debug symbols make up most of the size and aren't needed to run.
if command -v strip > /dev/null; then
  strip "$STAGE/bin/itami" "$STAGE/bin/xdg-desktop-portal-itamiiru"
fi
cp scripts/install.sh "$STAGE/scripts/"
cp data/* "$STAGE/data/"
cp README.md "$STAGE/"
cp docs/app-compat.md "$STAGE/docs/"
# A shortcut at the top, so the command is just ./install.sh.
printf '#!/bin/sh\nexec "$(dirname "$0")/scripts/install.sh" "$@"\n' > "$STAGE/install.sh"
chmod +x "$STAGE/install.sh"

tar -C target/package -czf "target/$NAME.tar.gz" "$NAME"
GLIBC="$(objdump -T "$STAGE/bin/itami" 2> /dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 || true)"
echo "Made target/$NAME.tar.gz (needs ${GLIBC:-glibc of this machine} or newer)"
