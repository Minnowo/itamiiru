# scripts

Helper scripts for working on itamiiru. Run them from anywhere; each one
moves to the repository root itself where it needs to.

| Script                 | What it does                                                   | Needs                                    |
|------------------------|----------------------------------------------------------------|------------------------------------------|
| `install-dev.sh`       | Build this checkout and install it to `~/.local/bin`           | the one-time Linux setup                 |
| `test-portal.sh`       | End-to-end test of the FileChooser portal backend              | `dbus-run-session`, `gdbus`              |
| `test-file-manager.sh` | End-to-end test of the FileManager1 ("Show in folder") service | `dbus-run-session`, `gdbus`, `dbus-send` |
| `make-test-dirs.sh`    | Make folders with 50,000 files, odd names, links, no access    | -                                        |
| `make-test-photos.sh`  | Make a folder of photo-sized JPEGs for thumbnail testing       | `ffmpeg`                                 |
| `package.sh`           | Build the release tarballs in `target/`                        | the musl Rust target                     |

## install-dev.sh

```
scripts/install-dev.sh            # OpenGL (glow) build, the default
scripts/install-dev.sh --vulkan   # Vulkan (wgpu) build
```

Builds both programs in release mode, copies `itami` and
`xdg-desktop-portal-itamiiru` to `~/.local/bin`, and stops the running
portal backend so the next file dialog starts the new build.

It only replaces the programs. Do the rest of the setup (menu entry, D-Bus
service, `.portal` file, `portals.conf`) once by hand first, as in
`docs/linux-setup/README.md`.

## test-portal.sh

```
scripts/test-portal.sh
```

Builds the portal backend and tests it over D-Bus without opening a window:

- Runs on a private session bus, so an installed backend can keep running.
- A fake picker stands in for `itami --pick` and answers each request.
- Checks OpenFile, SaveFile, SaveFiles, cancelling, closing a request
  mid-dialog, Ping and an unknown method.

Prints `ok` or `FAIL` per check and exits non-zero on any failure, then
dumps the backend's log.

## test-file-manager.sh

```
scripts/test-file-manager.sh
```

Builds the portal binary and tests its FileManager1 service (ShowItems,
ShowFolders, ShowItemProperties, Introspect, bad arguments):

- Runs on a private bus, so it works even while Thunar owns the name on the
  real one.
- The service is started by D-Bus activation from `data/`'s service file.
- A fake `itami` records the windows it would have opened.

Prints `ok` or `FAIL` per check and exits non-zero on any failure.

## make-test-dirs.sh

```
scripts/make-test-dirs.sh [DEST]          # default DEST: /tmp/itamiiru-test
cargo run -p itamiiru -- /tmp/itamiiru-test
```

Makes folders for checking listings by hand:

- `50k-files/`: 50,000 empty files, for scrolling and sorting speed.
- `odd-names/`: non-UTF-8, Japanese, Chinese, Korean and emoji names,
  spaces, hidden files, natural-sort cases, an executable, and file,
  folder and broken symlinks.
- `no-access/`: a folder you can't open, for the inline error.

To fake a slow drive, add `ITAMI_DEBUG_SLOW_FS=3000` before `cargo run`.
Running it again is safe; the 50,000 files are only made once.

## make-test-photos.sh

```
scripts/make-test-photos.sh [COUNT] [DEST]   # defaults: 2000, /tmp/itamiiru-photos
cargo run --release -p itamiiru -- /tmp/itamiiru-photos
```

Makes COUNT photo-sized JPEGs (1920x1440, about 300 KB each) plus a
portrait and a panorama. Open the folder, pick View: Large icons and check
that scrolling stays smooth, on-screen thumbnails fill in first, and a
second visit loads from the cache.

## package.sh

```
rustup target add x86_64-unknown-linux-musl   # once
scripts/package.sh
```

Builds the release tarballs in `target/`, one per renderer:
`itamiiru-<version>-<arch>-linux-opengl.tar.gz` and `...-vulkan.tar.gz`.
Each holds the stripped programs in `bin/`, `data/`, the docs and the
license.

- `xdg-desktop-portal-itamiiru` is linked statically (musl) and runs on any
  Linux.
- `itami` can't be static: winit and glow `dlopen` the X11, Wayland and GL
  libraries. It needs a glibc at least as new as the build machine's; the
  script prints the version.

Releases are not made by hand. The `release` workflow
(`.github/workflows/release.yml`) runs this script on Ubuntu 20.04 (glibc
2.31) and publishes the result: in the repository's Actions tab, pick
"release" -> "Run workflow" and enter the version from `Cargo.toml`. It
fails if the version doesn't match or the tag already exists.
