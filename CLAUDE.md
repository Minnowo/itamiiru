# itamiiru: project guide for Claude Code

itamiiru is a file picker and file explorer written in Rust with egui. It has two goals:

1. **One file picker for every Linux app.** itamiiru provides a backend for xdg-desktop-portal's FileChooser interface. GTK, Qt, Firefox, Chromium, Electron and Flatpak apps then all show the same dialog.
2. **A cross-platform file explorer** sharing the same UI. Linux comes first, then Windows and macOS.

The layout copies the Windows 10/11 file dialog.

`PLAN.md` holds the architecture, decisions, protocol specs and the phased task list. **Work through PLAN.md one phase at a time. Tick off each task (`- [x]`) as it's done, and add notes to the decision log when a decision is made.**

> **Naming:** the full name is **itamiiru**. Anything a user types uses the short form **itami**: the executable (`itami`, `itami --pick`) and environment variables (`ITAMI_DEBUG`, `ITAMI_THEME`, `ITAMI_BIN`). Everything else uses `itamiiru`: crates, window title, app_id, config folder, D-Bus names, file names in `data/`, the protocol header.

## About the developer

- **New to Rust.** When you write or change code, briefly explain any Rust concept that isn't obvious. Examples: why something is borrowed vs. cloned, `Arc<Mutex<_>>`, trait bounds, lifetimes, `?`, pattern-matching tricks. Put short explanations in chat, not essays in code comments.
- **"Keep it simple, stupid."** Prefer plain, explicit code. A `for` loop beats a clever iterator chain if it reads better. Avoid generics, trait hierarchies and macros unless they clearly make the code simpler.
- **Prefers vendoring** small, well-understood code over adding a library.
- Is a Golang developer, does not like complex code, simple is better. Rust is a very complex ugly language, so try to avoid fancy stuff when there is a more traditional alternative(assuming it doesn't have a performance impact).

## Hard rules

1. **Ask before adding any crate.** The approved list is below. If something seems to need a new crate, first propose a vendored alternative with a rough line count, and let the developer decide.
2. **No `async`/`await` and no async runtimes.** Concurrency means `std::thread` plus `std::sync::mpsc`, with `Mutex`, `Condvar` or `AtomicBool` where needed. See "Threading model" in PLAN.md.
3. **The UI thread never touches the filesystem**, except to read small config files at startup. Directory listing, `stat`, thumbnailing, drive scanning and file operations all go through worker threads.
4. **`unsafe` only in `platform/` FFI code and the D-Bus socket code.** Every `unsafe` block gets a `// SAFETY:` comment. The workspace denies `unsafe_code`; allow it per module with `#![allow(unsafe_code)]`.
5. **Platform-specific code lives only in `crates/itamiiru-core/src/platform/{linux,windows,macos}.rs`** (and their submodule folders, e.g. `platform/linux/places.rs`) (plus the Linux-only `itamiiru-portal` crate), behind `#[cfg(target_os = "...")]`. Everything else must compile on all three OSes.
6. **Every vendored module has unit tests.** Use official test vectors where they exist, such as the RFC 1321 vectors for MD5.
7. **Treat untrusted input as hostile.** This covers image files, D-Bus messages, picker requests and anything in `/proc`. Check sizes and lengths *before* allocating, and never `unwrap()` on data derived from them.
8. **No `unwrap()` in non-test code** unless failure is truly impossible; use `expect("why this can't fail")` there.
9. **Paths stay as `PathBuf`/`OsString`**, never `String`. Filenames are not guaranteed to be UTF-8. Convert with `to_string_lossy()` only at the point of display.

## Approved dependencies

| Crate | Used by | Why |
|---|---|---|
| `eframe` (default features off; wayland, x11, default_fonts, plus the renderer from the `itamiiru` crate's features: `glow` by default, or `wgpu`; **verify current feature names** in eframe's Cargo.toml) | itamiiru | Window + egui. glow (OpenGL) is the default because it pulls in far fewer crates; `--features wgpu` (Vulkan, about 37 more crates) is an opt-in build for release tarballs, see PLAN.md D25. |
| `egui_extras` (default features off) | itamiiru | Only for `TableBuilder` in the details view. Same repo as egui. |
| `png` | itamiiru-core | PNG decoding, and encoding for the thumbnail cache. |
| `zune-jpeg` **or** `jpeg-decoder` | itamiiru-core | JPEG. Pick one in the Phase 5 spike. |
| `gif` | itamiiru-core | First frame of GIFs. |
| `image-webp` | itamiiru-core | WebP. |
| `libc` (unix targets only) | itamiiru-core, itamiiru-portal | `statvfs`, `getuid`, `kill`. Already a transitive dependency of winit. |

Everything else is vendored or hand-written. Specifically **not** wanted: clap, serde, toml, anyhow, thiserror, log, env_logger, tokio, zbus, notify, dirs, walkdir, tempfile, image (the umbrella crate), windows (the crate).

After any `Cargo.toml` change, run this and record the number in PLAN.md's dependency budget:
```
cargo tree -e normal --prefix none | sort -u | wc -l
```

## Repository layout

```
Cargo.toml                 workspace
CLAUDE.md                  this file
PLAN.md                    architecture + phased tasks
crates/
  itamiiru-core/           library: no egui. Filesystem, workers, thumbnails, protocol, vendored helpers
    src/platform/            the only place for OS-specific code
                             (linux.rs, plus linux/ for its submodules)
  itamiiru/                egui binary "itami": explorer mode (default) and picker mode (--pick)
  itamiiru-portal/         Linux-only binary "xdg-desktop-portal-itamiiru": the D-Bus backend
data/                      .portal file, D-Bus service file, example configs
scripts/install-dev.sh     build and copy the two programs to ~/.local/bin (setup is by hand, README)
scripts/package.sh         release tarball for machines without Rust
README.md                  user-facing install and troubleshooting
tests/requests/            sample picker requests (protocol in PLAN.md)
```

## Commands

```
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p itamiiru                                        # explorer mode
cargo run -p itamiiru -- --pick < tests/requests/open-images.req   # picker mode
cargo check -p itamiiru-core -p itamiiru --target x86_64-pc-windows-gnu
    # needs: rustup target add x86_64-pc-windows-gnu
cargo check -p itamiiru-core --target aarch64-apple-darwin  # macOS code in platform/
cargo clippy -p itamiiru --no-default-features --features wgpu -- -D warnings
    # the Vulkan (wgpu) renderer build; the default is glow (PLAN.md D25)
    # needs: rustup target add aarch64-apple-darwin
./scripts/install-dev.sh                                  # rebuild and update ~/.local/bin (--vulkan)
./scripts/package.sh                                      # release tarball in target/
./scripts/test-portal.sh, ./scripts/test-file-manager.sh  # D-Bus tests on a private bus
ITAMI_DEBUG=1 cargo run -p itamiiru                          # debug logging to stderr
```

Run `cargo test`, `cargo clippy` and the Windows `cargo check` before calling a task done.

## Conventions

- Rust edition 2024, latest stable toolchain.
- Errors: a small `enum` per module, or `Box<dyn std::error::Error>` at binary entry points. No error crates.
- Logging: one tiny `debug!` macro in itamiiru-core that prints to stderr when `ITAMI_DEBUG=1`. This is the one approved macro. When the portal is D-Bus-activated, its stderr goes to the journal (`journalctl --user -f`).
- Arguments: parse `std::env::args()` by hand. The CLI is tiny.
- Config: hand-parsed `key=value` text files. No TOML or JSON.
- Commit after each completed task, prefixing the message with its ID, e.g. `P1.4: natural sort`.
- **ASCII only**, in code, comments, docs and UI strings. No em dashes, arrows, section signs, smart quotes or Unicode glyphs; write `-`, `->`, `section 4`, `"`. Icons are painted (`crates/itamiiru/src/icons.rs`), never font glyphs. A test that needs non-ASCII input uses escapes like `"\u{e9}"`. Check: `git grep -nP '[^\x00-\x7F]'` prints nothing.
- **Explicit returns, Go-style.** A function that returns a value ends with `return x;`, never a bare tail expression. Instead of a final `if a { x } else { y }`, write `if a { return x; } return y;`. Value-producing `if`/`match` is fine on the right side of a `let` (`let n = if a { 1 } else { 2 };`) and as `return match ... { ... };`. One-line closures may stay implicit (`|a, b| a.cmp(b)`); multi-line closures that return a value use `return`. clippy's `needless_return` is allowed in the workspace for this. To list violations: `cargo clippy --workspace --all-targets -- -A clippy::all -W clippy::implicit_return` (ignore the one-line closures it reports).

## UI style

The UI follows the ponkotsu design system (https://github.com/Minnowo/ponkotsu-css), translated to egui in `crates/itamiiru/src/theme.rs`. See PLAN.md section 6 Theme.

- Never write a color literal in UI code. Use a `Theme` role. Text on fill `X` is `on_X`.
- Depth comes from `theme.surface(n)` fills, not borders or shadows. Bump `n` for each nested box.
- At most one `filled_button` per group, with the main action last in the row. Cancel is a plain `ui.button`.
- Dialog buttons go in the bottom-right corner: lay them out with `theme::dialog_buttons` (right to left, so the main action is added first).
- Spacing comes from the theme constants (`GAP_SMALL` 4, `GAP` 8, `GAP_LARGE` 16, `PADDING` 12). Don't invent other values.
- Corner radius: `RADIUS_SMALL` (4) for controls, `RADIUS_MEDIUM` (8) for containers. Nothing else.
- Muted text is `ui.weak(..)`; the colors behind it come from the theme.

## Reference specs (fetch when working on that area)

- Portal backend API: https://flatpak.github.io/xdg-desktop-portal/docs/ (impl.portal.FileChooser). The XML source of truth is `data/org.freedesktop.impl.portal.FileChooser.xml` in the xdg-desktop-portal GitHub repo.
- D-Bus wire protocol: https://dbus.freedesktop.org/doc/dbus-specification.html
- Thumbnail cache: https://specifications.freedesktop.org/thumbnail-spec/latest/
- Trash: https://specifications.freedesktop.org/trash-spec/latest/
- Shared MIME info (globs2 format): https://specifications.freedesktop.org/shared-mime-info-spec/latest/
- XDG user dirs: https://www.freedesktop.org/wiki/Software/xdg-user-dirs/
- File manager D-Bus interface (FileManager1): https://www.freedesktop.org/wiki/Specifications/file-manager-interface/

When a spec and PLAN.md disagree, the spec wins. Fix PLAN.md and note the change in the decision log.
