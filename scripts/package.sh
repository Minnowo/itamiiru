#!/usr/bin/env bash
# Builds release tarballs for installing on another machine without Rust,
# one per renderer:
#   target/itamiiru-<version>-<arch>-linux-opengl.tar.gz  (glow: OpenGL)
#   target/itamiiru-<version>-<arch>-linux-vulkan.tar.gz  (wgpu: Vulkan,
#       falling back to OpenGL; starts faster once the driver's cache is warm)
# On the other machine: unpack it and follow docs/linux-setup/README.md.
# The portal is linked statically against musl and runs anywhere. itami
# can't be static (winit and glow dlopen the X11, Wayland and GL
# libraries), so it needs a glibc as new as this machine's; the release
# workflow builds on Ubuntu 20.04 (glibc 2.31) for that reason.
# Needs the musl target: rustup target add $(uname -m)-unknown-linux-musl
set -euo pipefail
cd "$(dirname "$0")/.."

if ! command -v cargo > /dev/null && [ -f "$HOME/.cargo/env" ]; then
  . "$HOME/.cargo/env"
fi
VERSION="$(sed -n 's/^version = "\(.*\)"$/\1/p' Cargo.toml | head -1)"
MUSL="$(uname -m)-unknown-linux-musl"

cargo build --release -p itamiiru-portal --target "$MUSL"

# package RENDERER CARGO_FEATURES...: one tarball, named after the renderer.
package() {
  local renderer="$1"
  shift
  cargo build --release -p itamiiru "$@"
  local name="itamiiru-$VERSION-$(uname -m)-linux-$renderer"
  local stage="target/package/$name"
  rm -rf "$stage"
  mkdir -p "$stage/bin" "$stage/data" "$stage/docs/linux-setup"

  cp target/release/itami "target/$MUSL/release/xdg-desktop-portal-itamiiru" "$stage/bin/"
  # Debug symbols make up most of the size and aren't needed to run.
  if command -v strip > /dev/null; then
    strip "$stage/bin/itami" "$stage/bin/xdg-desktop-portal-itamiiru"
  fi
  cp data/* "$stage/data/"
  cp README.md LICENSE "$stage/"
  cp docs/app-compat.md "$stage/docs/"
  cp docs/linux-setup/README.md "$stage/docs/linux-setup/"

  tar -C target/package -czf "target/$name.tar.gz" "$name"
  local glibc
  glibc="$(objdump -T "$stage/bin/itami" 2> /dev/null | grep -o 'GLIBC_[0-9.]*' | sort -uV | tail -1 || true)"
  echo "Made target/$name.tar.gz (needs ${glibc:-glibc of this machine} or newer)"
}

package opengl
package vulkan --no-default-features --features wgpu
# Leave the default (glow) build in target/release, as cargo build would.
cargo build --release -p itamiiru
