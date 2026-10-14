# itamiiru: project guide for Claude Code

itamiiru is a file picker and file explorer written in Rust with egui. It has
two goals:

1. **One file picker for every Linux app.** itamiiru is a backend for
   xdg-desktop-portal's FileChooser interface, so GTK, Qt, Firefox, Chromium,
   Electron and Flatpak apps all show the same dialog.
2. **A cross-platform file explorer** with the same UI. Linux first, then
   Windows and macOS.

The layout copies the Windows 10/11 file dialog.

## Read this first

| File                 | Read it because                                                              |
|----------------------|------------------------------------------------------------------------------|
| `PLAN.md`            | Architecture, decision log, threading model, protocol spec, remaining tasks. |
| `README.md`          | User-facing overview and usage.                                              |
| `docs/linux-setup/`  | Install and xdg-desktop-portal setup on Linux.                               |
| `scripts/README.md`  | What each script does and how to run it.                                     |
| `docs/app-compat.md` | Which apps show itamiiru as their file dialog, and what they need.           |

`PLAN.md` is the source of truth for design. When a task is done, remove it
from PLAN.md. When a decision is made, add a row to the decision log.

## Naming

- The full name is **itamiiru**.
- Anything a user types uses the short form **itami**: the executable
  (`itami`, `itami --pick`) and environment variables (`ITAMI_DEBUG`,
  `ITAMI_THEME`, `ITAMI_BIN`).
- Everything else uses `itamiiru`: crates, window title, app_id, config
  folder, D-Bus names, file names in `data/`, the protocol header.

## Hard rules

Breaking one of these is a bug, even if the code works.

1. **Ask before adding any crate**, including dev and build dependencies. The
   approved list is under "Approved dependencies". If something seems to need
   a new crate, first propose a vendored alternative with a rough line count
   and let the developer decide.
2. **No `async`/`await` and no async runtimes.** Concurrency is `std::thread`
   plus `std::sync::mpsc`, with `Mutex`, `Condvar` or `AtomicBool` where
   needed. See "Threading model" in PLAN.md.
3. **The UI thread never touches the filesystem**, except to read small config
   files at startup. Listing, `stat`, thumbnailing, drive scanning and file
   operations all go through worker threads.
4. **`unsafe` only in `platform/` FFI code and the D-Bus socket code.** Every
   `unsafe` block gets a `// SAFETY:` comment. The workspace denies
   `unsafe_code`; allow it per module with `#![allow(unsafe_code)]`.
5. **Platform-specific code lives only in
   `crates/itamiiru-core/src/platform/{linux,windows,macos}.rs`** and their
   submodule folders (e.g. `platform/linux/places.rs`), plus the Linux-only
   `itamiiru-portal` crate, behind `#[cfg(target_os = "...")]`. Everything
   else must compile on all three OSes.
6. **Every vendored module has unit tests.** Use official test vectors where
   they exist (e.g. the RFC 1321 vectors for MD5).
7. **Treat untrusted input as hostile.** This covers image files, D-Bus
   messages, picker requests and anything in `/proc`. Check sizes and lengths
   *before* allocating, check bounds before indexing, guard divisions, bound
   recursion, and never `unwrap()` on data derived from them.
8. **No `unwrap()` in non-test code** unless failure is truly impossible; use
   `expect("why this can't fail")` there.
9. **Paths stay as `PathBuf`/`OsString`**, never `String`. Filenames are not
   guaranteed to be UTF-8. Convert with `to_string_lossy()` only at the point
   of display.
10. **ASCII only** in code, comments, docs, commit messages and UI strings. No
    em dashes, arrows, section signs, smart quotes, ellipsis characters or
    other Unicode glyphs; write `-`, `->`, `section 4`, `"`, `...`. Icons are
    painted (`crates/itamiiru/src/icons.rs`), never font glyphs. A test that
    needs non-ASCII input uses escapes like `"\u{e9}"`. Check:
    `git grep -nP '[^\x00-\x7F]'` prints nothing.

## Code style

### Keep it simple

The developer's words: "Keep it simple, stupid." Rust is a complex language;
write it as if it were Go, with plain control flow a reader can follow top to
bottom.

- A plain `for` loop, `if` and early `return` beat iterator chains,
  combinators and clever matches when the plain version reads better.
- No generics, trait hierarchies or macros unless they clearly make the code
  simpler. The one approved macro is `debug!` (see Conventions).
- One way of doing a thing. No abstraction until there is a second real user.
  No speculative options, flags or extension points.
- Prefer vendoring small, well-understood code over adding a library.
- Handle errors where they occur. Do not swallow them.
- No side effects inside `debug_assert!` or `debug!` arguments.

### Explicit returns

A function that returns a value ends with `return x;`, never a bare tail
expression.

```rust
// Yes
fn pick(a: bool) -> u32 {
    if a {
        return 1;
    }
    return 2;
}

// No
fn pick(a: bool) -> u32 {
    if a { 1 } else { 2 }
}
```

- Value-producing `if`/`match` is fine on the right side of a `let`
  (`let n = if a { 1 } else { 2 };`) and as `return match ... { ... };`.
- One-line closures may stay implicit (`|a, b| a.cmp(b)`). Multi-line
  closures that return a value use `return`.
- clippy's `needless_return` is allowed in the workspace for this. To list
  violations:
  `cargo clippy --workspace --all-targets -- -A clippy::all -W clippy::implicit_return`
  (ignore the one-line closures it reports).

### Modules and files

- One concept per file, named after what it does. No catch-all `utils.rs`,
  `helpers.rs` or `misc.rs`.
- Dependencies point one way: `itamiiru` and `itamiiru-portal` depend on
  `itamiiru-core`, never the reverse. `itamiiru-core` has no egui.
- Order each file top to bottom, important things first: header comment,
  imports, constants, the main type or function, supporting types, then
  methods (constructors, public, private helpers; callers above callees).
- Tests: constants and fixtures first, then test cases, then helpers last.

### Comments

- Every file starts with a short `//!` header: what the file is for.
- Public functions and types get a `///` doc comment: what it does, what it
  returns, and any hidden semantics (blocking, thread safety, units, side
  effects, errors).
- Inside function bodies, comment only when something is done in a
  non-obvious way or for a non-obvious reason. Say why, not what.
- No comments that restate the code or narrate history ("now uses X",
  "fixed bug").
- If a workaround needs a paragraph to justify it, fix the code instead.

## Conventions

- Rust edition 2024, latest stable toolchain.
- Errors: a small `enum` per module, or `Box<dyn std::error::Error>` at binary
  entry points. No error crates.
- Logging: the `debug!` macro in itamiiru-core, which prints to stderr when
  `ITAMI_DEBUG=1`. No `println!`/`eprintln!` outside tests and CLI output.
  When the portal is D-Bus-activated, its stderr goes to the journal
  (`journalctl --user -f`).
- Arguments: parse `std::env::args()` by hand. The CLI is tiny.
- Config: hand-parsed `key=value` text files. No TOML or JSON.
- Markdown tables are space-padded so columns line up in plain text.
- Commit after each completed task, prefixing the message with its ID, e.g.
  `P1.4: natural sort`.

## Testing

- **Fix logic bugs test-first:** write a test that reproduces the bug, run it
  and confirm it fails for the expected reason, fix the bug, then confirm it
  and the rest of the suite pass. Skip this only where a test makes no sense
  (UI styling, layout, wording).
- New features come with at least one test that would fail without them.
- Prefer tests at the outside of a module (its public functions, the picker
  protocol, files written, D-Bus messages) over tests of private internals.
- Never weaken, skip or delete a test or a check to make something pass.
- Check that tests actually ran. A filtered or empty run is not a pass.

## Read the spec

When working on a format or protocol, read the authoritative spec before
writing code. Do not guess field names, limits, encodings or error semantics
from memory or examples. When a spec and PLAN.md disagree, the spec wins: fix
PLAN.md and note the change in the decision log.

| Area                       | Spec                                                                                         |
|----------------------------|----------------------------------------------------------------------------------------------|
| Portal backend API         | https://flatpak.github.io/xdg-desktop-portal/docs/ (impl.portal.FileChooser). Source of truth: `data/org.freedesktop.impl.portal.FileChooser.xml` in the xdg-desktop-portal GitHub repo. |
| D-Bus wire protocol        | https://dbus.freedesktop.org/doc/dbus-specification.html                                     |
| Thumbnail cache            | https://specifications.freedesktop.org/thumbnail-spec/latest/                                |
| Trash                      | https://specifications.freedesktop.org/trash-spec/latest/                                    |
| Shared MIME info (globs2)  | https://specifications.freedesktop.org/shared-mime-info-spec/latest/                         |
| XDG user dirs              | https://www.freedesktop.org/wiki/Software/xdg-user-dirs/                                     |
| FileManager1 D-Bus         | https://www.freedesktop.org/wiki/Specifications/file-manager-interface/                      |

## UI style

The UI follows the ponkotsu design system
(https://github.com/Minnowo/ponkotsu-css), translated to egui in
`crates/itamiiru/src/theme.rs`. See PLAN.md's UI notes.

- Never write a color literal in UI code. Use a `Theme` role. Text on fill
  `X` is `on_X`.
- Depth comes from `theme.surface(n)` fills, not borders or shadows. Bump `n`
  for each nested box.
- At most one `filled_button` per group, with the main action last in the
  row. Cancel is a plain `ui.button`.
- Dialog buttons go in the bottom-right corner: lay them out with
  `theme::dialog_buttons` (right to left, so the main action is added first).
- Spacing comes from the theme constants (`GAP_SMALL` 4, `GAP` 8, `GAP_LARGE`
  16, `PADDING` 12). Don't invent other values.
- Corner radius: `RADIUS_SMALL` (4) for controls, `RADIUS_MEDIUM` (8) for
  containers. Nothing else.
- Muted text is `ui.weak(..)`; the colors behind it come from the theme.
- After a UI change, run the app and look at the result before calling it
  done.

## Approved dependencies

| Crate                          | Used by                        | Why                                                                                     |
|--------------------------------|--------------------------------|-----------------------------------------------------------------------------------------|
| `eframe`                       | itamiiru                       | Window + egui. See note below.                                                          |
| `egui_extras` (no defaults)    | itamiiru                       | Only for `TableBuilder` in the details view. Same repo as egui.                         |
| `png`                          | itamiiru-core                  | PNG decoding, and encoding for the thumbnail cache.                                     |
| `jpeg-decoder` (no defaults)   | itamiiru-core                  | JPEG, with downscaled decoding (PLAN.md D12).                                           |
| `gif`                          | itamiiru-core                  | First frame of GIFs.                                                                    |
| `image-webp`                   | itamiiru-core                  | WebP.                                                                                   |
| `libc` (unix only)             | itamiiru-core, itamiiru-portal | `statvfs`, `getuid`, `kill`. Already a transitive dependency of winit.                  |

`eframe` has default features off; it uses wayland, x11, default_fonts, plus
the renderer chosen by the `itamiiru` crate's features: `glow` (OpenGL) by
default because it pulls in far fewer crates, or `wgpu` (Vulkan, about 37 more
crates) as an opt-in build (PLAN.md D25). Verify current
feature names in eframe's Cargo.toml before changing them.

Everything else is vendored or hand-written. Specifically **not** wanted:
clap, serde, toml, anyhow, thiserror, log, env_logger, tokio, zbus, notify,
dirs, walkdir, tempfile, image (the umbrella crate), windows (the crate).

After any `Cargo.toml` change, run this and record the number in PLAN.md's
dependency budget:

```
cargo tree -e normal --prefix none | sort -u | wc -l
```

## Repository layout

```
Cargo.toml                 workspace
CLAUDE.md                  this file
PLAN.md                    architecture, decisions, remaining tasks
crates/
  itamiiru-core/           library, no egui: filesystem, workers, thumbnails,
                           protocol, vendored helpers
    src/platform/          the only place for OS-specific code
                           (linux.rs, plus linux/ for its submodules)
  itamiiru/                egui binary "itami": explorer mode (default) and
                           picker mode (--pick)
  itamiiru-portal/         Linux-only binary "xdg-desktop-portal-itamiiru":
                           the D-Bus backend
data/                      .portal file, D-Bus service file, example configs
docs/                      app compatibility notes
  linux-setup/             install and xdg-desktop-portal setup
scripts/                   dev install, D-Bus tests, test folders (see its README.md)
tests/requests/            sample picker requests (protocol in PLAN.md)
README.md                  user-facing overview and usage
.github/workflows/         release.yml: manual GitHub release (scripts/README.md)
```

## Commands

```
cargo build
cargo test --workspace
cargo clippy --workspace --all-targets -- -D warnings
cargo fmt --all
cargo run -p itamiiru                                              # explorer mode
cargo run -p itamiiru -- --pick < tests/requests/open-images.req   # picker mode
ITAMI_DEBUG=1 cargo run -p itamiiru                                # debug logging to stderr

cargo check -p itamiiru-core -p itamiiru --target x86_64-pc-windows-gnu
    # needs: rustup target add x86_64-pc-windows-gnu
cargo check -p itamiiru-core --target aarch64-apple-darwin
    # macOS code in platform/; needs: rustup target add aarch64-apple-darwin
cargo clippy -p itamiiru --no-default-features --features wgpu -- -D warnings
    # the Vulkan (wgpu) renderer build; the default is glow (PLAN.md D25)

./scripts/install-dev.sh                                  # rebuild and update ~/.local/bin (--vulkan)
./scripts/test-portal.sh, ./scripts/test-file-manager.sh  # D-Bus tests on a private bus
```

## Working style

- Read before you write. Match the surrounding code's naming, structure and
  idiom.
- Do what was asked. Do not refactor, rename or reformat unrelated code in the
  same change.
- Keep the build green. Each commit is a working state.
- Report honestly. A stub, partial implementation or untested path is
  reported as such, never as done. Say whether a number was measured or
  estimated.
- No stubs or placeholders to get something compiling: no empty bodies,
  hard-coded stand-in values, `todo!()` or disabled checks. If something
  cannot be finished, stop and say so.
- Prefer rules enforced by tools (types, clippy, tests, scripts) over prose.
  When a rule here can be checked automatically, propose the check.
- When the same kind of mistake happens twice, propose a rule for this file or
  a check that catches it.
- For large jobs (many errors, failing tests or files), write the work list
  down, try the approach on a small sample first, then work through the list
  in batches.

## Ask the developer first

Do not decide these alone:

- adding or upgrading a dependency;
- pushing, publishing or releasing;
- destructive git operations (force push, reset, deleting branches,
  `git stash`, `git clean`, `git checkout -- .`);
- deleting files that are untracked or not yours;
- changing the picker protocol, D-Bus interfaces, config file format or the
  thumbnail cache format;
- anything that costs money.

Stage specific files, never `git add -A` or `git add .`.

## Before you finish

- [ ] `cargo build`, `cargo test --workspace` and
      `cargo clippy --workspace --all-targets -- -D warnings` pass, and the
      tests actually ran.
- [ ] The Windows `cargo check` passes (and the macOS one if `platform/` was
      touched).
- [ ] `cargo fmt --all` is clean.
- [ ] Bug fixes have a regression test that failed before the fix.
- [ ] No stubs, placeholders or disabled checks left behind.
- [ ] `git grep -nP '[^\x00-\x7F]'` prints nothing.
- [ ] No new dependencies without approval; dependency budget updated if
      `Cargo.toml` changed.
- [ ] File header on new files, doc comments on new public items.
- [ ] PLAN.md updated: finished task removed, decisions logged.
