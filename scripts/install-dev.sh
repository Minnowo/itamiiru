#!/usr/bin/env bash
# Development: builds this checkout and puts the two programs in
# ~/.local/bin, then restarts the portal backend so the next file dialog
# uses the new build. The rest of the setup (menu entry, D-Bus and .portal
# files, portals.conf) is done once by hand, as in docs/linux-setup/README.md.
#   scripts/install-dev.sh            OpenGL build
#   scripts/install-dev.sh --vulkan   Vulkan (wgpu) build
set -euo pipefail
cd "$(dirname "$0")/.."

features=()
if [ "${1:-}" = "--vulkan" ]; then
  features=(--no-default-features --features wgpu)
fi
cargo build --release -p itamiiru-portal
cargo build --release -p itamiiru "${features[@]}"

mkdir -p ~/.local/bin
install -m755 target/release/itami target/release/xdg-desktop-portal-itamiiru ~/.local/bin/
echo "Installed itami and xdg-desktop-portal-itamiiru in ~/.local/bin"

# The backend is started again on the next file dialog. The pattern is
# anchored so it can't match this script's own command line.
pkill -f '^[^ ]*xdg-desktop-portal-itamiiru( |$)' || true
