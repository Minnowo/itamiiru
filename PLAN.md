# itamiiru: plan

Phases 0-7 and 10 are done, Phase 8 (Windows, macOS) is deferred, and Phase 9 (polish) has a few tasks left. What's left is under "Remaining tasks"; the rest of this file is the design that still applies.

## Goals

- One file picker for as many Linux apps as possible, as an xdg-desktop-portal FileChooser backend.
- A file explorer with the same UI on Linux, Windows and macOS, laid out like the Windows 10/11 dialog.
- Few crates, small helpers vendored, image thumbnails decoded in pure Rust.
- A UI that never freezes, even on hung network mounts.

**Non-goals:** video thumbnails or C decoders; replacing the OS dialog on Windows/macOS; apps that never use the portal (Java/Swing, Tk, Wine); network browsing (mounted network drives are shown).

## Architecture

```
 App (Firefox, Chromium, GTK, Qt, Flatpak ...)
   | D-Bus: org.freedesktop.portal.FileChooser
   v
 xdg-desktop-portal (frontend, reads ~/.config/xdg-desktop-portal/portals.conf)
   | D-Bus: org.freedesktop.impl.portal.FileChooser
   v
 xdg-desktop-portal-itamiiru (crate itamiiru-portal, Linux only, no GUI)
   | spawns `itami --pick`, writes the request to its stdin (protocol below)
   v
 itami --pick (crate itamiiru, egui): shows the dialog, prints the response, exits
```

Separate processes keep the portal small and free of GUI crates, a UI crash can't take down the D-Bus service, and the picker can be tested from a terminal. Each pick is a fresh process, so startup must stay fast (measured: about 115 ms to the first frame on screen, under 3 ms of it ours).

| Crate | Kind | Contains |
|---|---|---|
| `itamiiru-core` | lib | Listing, workers, places and drives, thumbnails, protocol, vendored helpers, `platform/` (incl. the D-Bus client in `platform/linux/dbus/`) |
| `itamiiru` | bin `itami` | The egui UI. `itami [PATH]` is explorer mode; `itami --pick` is picker mode |
| `itamiiru-portal` | bin `xdg-desktop-portal-itamiiru` | FileChooser backend; with `--file-manager`, FileManager1 |

## Decision log

| # | Decision | Choice and why | Revisit if |
|---|---|---|---|
| D1 | UI toolkit | egui via eframe: simple, self-contained, cross-platform | Accessibility or a native look become hard requirements |
| D3 | Concurrency | std threads + mpsc; file I/O isn't really async anyway | Never |
| D4 | Picker <-> portal | Separate processes, stdin/stdout line protocol | Startup time becomes noticeable |
| D5 | D-Bus | Hand-written client (about 1,100 lines with tests) | It proves flaky (fallback: zbus's blocking API, ask first) |
| D6 | Image decoding | Per-format pure-Rust crates, no `image` umbrella | - |
| D8 | Thumbnail cache | freedesktop spec, shared with Nautilus, Dolphin and others | - |
| D9 | Live updates | Poll the folder's mtime every 2 s while focused, plus F5 | inotify via libc (task below) |
| D10 | Accessibility | AccessKit off: on Linux it pulls in zbus and atspi | Before any public release |
| D11 | Details table | `egui_extras::TableBuilder` (virtualized rows) | It gets limiting |
| D12 | JPEG crate | `jpeg-decoder` 0.3, no default features. Its `scale()` decodes 12 MP at 1/8 in 35 ms into 0.5 MB; zune-jpeg has no downscaled decode | jpeg-decoder (maintenance mode) stops building or has an unfixed security issue |
| D13 | Look | ponkotsu design system in `theme.rs`; dark palette built in; a theme file (`$ITAMI_THEME` or `<config dir>/theme`) overrides roles as `role = #hex` or pasted ponkotsu CSS | egui can't express something the design needs |
| D14 | Text sizes | ponkotsu scaled to a 14 px body (heading 19, small 12) | Feels wrong in use |
| D15 | Natural sort | ASCII case-insensitive, other letters by raw bytes | Non-English names sort badly |
| D16 | UI scaling | egui zoom (Ctrl+Plus/Minus/0, 10% steps) plus status-bar buttons | - |
| D17 | File clipboard | In-app list of paths, also put on the system clipboard as text (so egui reports Ctrl+V, and paths paste into a terminal) | Interop with other file managers is wanted (`text/uri-list` targets) |
| D18 | FileManager1 | Same binary as the portal, `--file-manager`, own process and activation file | Another file manager's daemon holding the name is a common complaint |
| D19 | Tabs and split | A `Pane` is one folder view with its own channels; a `Tab` holds one or two panes; the app holds what's shared. Pane actions are returned, then applied | A third pane or saved tabs are wanted |
| D20 | Details columns | Name fills the leftover width; others sized to their longest possible text; `details_columns` picks which show | A column with very variable content is added |
| D21 | Removable drives | UDisks2 on the system bus with our D-Bus client: list, `Filesystem.Mount`, eject = unmount all then `PowerOff`/`Eject`; polkit decides (interactive auth flag) | Encrypted volumes are wanted, or polling is too slow |
| D22 | Fonts | System fonts first (fontconfig on Linux), egui's built-in next (emoji), script fallbacks last | A system font renders badly in egui |
| D23 | Custom actions | `actions` file next to the config, `.desktop`-like sections, hand-parsed; commands run directly, never through a shell; `%f %F %d %n %%` | Actions need more structure |
| D24 | Drag out | XDND on `x11rb` (already in the tree via winit) in `platform/linux/xdnd.rs`, own connection and thread, pointer polled every 10 ms | Wayland support is wanted |
| D25 | Renderer | Build-time feature: `glow` (default, 180 crates) or `wgpu` (Vulkan, 217 crates; first frame in 83-88 ms vs 115-176 ms). Releases ship both | glow is dropped by egui, or wgpu's crate count comes down |
| D26 | Installing | A short manual guide in README.md instead of an install script; packages ship per-user files in `/usr/share/itamiiru/` | Many users struggle with the manual steps |

## Threading model

The UI thread never does filesystem I/O. Workers send results over `std::sync::mpsc` and call `ctx.request_repaint()`; the UI drains them with `try_recv` each frame.

- **Listing:** one thread per navigation, so a hung mount blocks only its own thread (leaked until the kernel call returns). Results stream in batches of about 500.
- **Generation ids:** every navigation bumps a counter; results with an old id are ignored.
- **Thumbnail pool:** min(4, cores - 1) threads share a LIFO stack (`Mutex<Vec<Job>>` + `Condvar`), so what's on screen now goes first. Cleared on navigation; each decode runs in `catch_unwind`.
- **Drives:** `statvfs` on its own thread per mount (it blocks on dead network mounts).
- **Cancellation** of long jobs (search, copy) via `Arc<AtomicBool>`.
- **Test hook:** debug builds honour `ITAMI_DEBUG_SLOW_FS=<ms>`, a sleep before each folder open and each batch.

## Picker <-> portal protocol (v1)

The portal writes a request to the picker's **stdin**; the picker writes a response to **stdout** and exits. Both sides live in `itamiiru-core/src/protocol.rs`; samples are in `tests/requests/`.

- UTF-8, one record per line, `\n` endings. A keyword, then fields separated by single spaces.
- Every field is percent-encoded. Encoders must encode space, `%`, control bytes and bytes >= 0x80. `-` alone means the empty string (a literal `-` is `%2D`).
- Paths are absolute and encoded from raw bytes, so non-UTF-8 names survive.
- Unknown keywords are ignored.

**Request** (first line `itamiiru-protocol 1`, last line `end`)

| Keyword | Fields | Meaning |
|---|---|---|
| `mode` | `open` \| `save` \| `save-files` \| `folder` | Dialog type. `save-files` is a folder pick; the portal appends the names |
| `title` | text | Window title |
| `accept_label` | text | Open/Save button text |
| `multiple` | `0`\|`1` | Multiple selection (open mode) |
| `modal` | `0`\|`1` | Informational |
| `app_id` | text | Requesting app (last folder is remembered per app) |
| `parent` | e.g. `x11:1a00004` | Parent window handle (not used yet) |
| `folder` | path | Start folder |
| `name` | text | Suggested file name (save) |
| `file` | path | Existing file to save over (save) |
| `filter` | name, then `glob:PATTERN` / `mime:TYPE` ... | One filter; repeatable, order kept |
| `current_filter` | index | Filter selected at start |
| `choice` | id, label, default, then option-id/label pairs | Footer control; no pairs means a checkbox (`true`/`false`). Repeatable |

**Response** (first line `itamiiru-protocol 1`): `uri` (one per item), `filter` (index), `choice` (id, value).

**Exit codes:** 0 OK, 1 cancelled, anything else an error (portal response 2).

## UI notes

**Picker bottom row** (matches Windows):
- The filename field follows the selection (`"a.txt" "b.txt"` for several). A name copied from the selection is cleared when the selection goes away, except in save mode; a typed name stays.
- Enter on a typed path navigates to a folder or selects a file; a glob like `*.log` becomes a temporary filter.
- Save mode: suggested name prefilled with the stem selected; the active filter's single `*.ext` is appended when no extension is typed; overwrite confirmation; inline error for unwritable folders.
- Folder mode shows folders only; with nothing selected it returns the current folder.

**Keyboard**

| Key | Action |
|---|---|
| Enter | Open folder / accept |
| Backspace, Alt+Up | Up one folder (from `/` to This PC) |
| Alt+Left / Alt+Right | Back / forward |
| Ctrl+L, Alt+D, F4 | Edit address bar |
| Ctrl+F, F3 / Ctrl+Shift+F | Search row / filter row |
| Ctrl+T, Ctrl+W, Ctrl+Tab, Ctrl+Shift+Tab | Tabs: new, close, next, previous |
| Middle-click a folder | Open it in a background tab |
| F6 | Switch the active split pane |
| Ctrl+H / Alt+P | Hidden files / preview pane |
| Ctrl+Plus / Minus / 0 | Zoom |
| Ctrl+Shift+N | New folder |
| Ctrl+Comma | Settings |
| F5 | Refresh |
| Ctrl+A, typing letters | Select all, type-ahead |
| Escape | Cancel dialog / close menu / cancel a drag |
| F2, Delete, Shift+Delete, Ctrl+C/X/V | Explorer: rename, trash, delete, clipboard |

**Config** (`~/.config/itamiiru/config`, `key=value`): view, sort, `details_columns`, sidebar width, window size, zoom, hidden files, `status_fields`, `last_folder.<app_id>`, and the optional features below. Most are on the settings page.

**Optional features.** Each lives in its own module and hooks in at one call site that checks the setting; off means the hook is skipped.

| Setting | Default | Where it hooks in |
|---|---|---|
| `breadcrumb_dropdowns` | off | `separator()` in `address.rs` (not built yet) |
| `auto_refresh_folder` | on | Top of `App::check_for_changes` |
| `auto_refresh_drives` | on | Top of `App::check_places` |
| `preview_pane` | off | The `Panel::right("preview")` block in `App::ui` |
| `use_trash` | on | `App::run_command` for `Command::Trash`, and `trash_supported` |
| `drag_and_drop` | on | Top of `App::drag_start` |
| `thumbnails` | on | `Thumbs::new` starts no pool when off |

**Theme:** egui's global style is set from the theme at startup, so plain widgets follow it. Selected rows use `secondary-container`; categorical colors (`pink`, `red`, `yellow`, `green`, `blue`) are for file-type icons and drive bars, never errors. The rest of the rules are in CLAUDE.md.

**Fonts:** a font thread starts before the window; setup waits up to 200 ms for the main fonts. On Linux, `fc-match sans-serif`/`monospace` pick the main fonts and `fc-match -s ... 'sans-serif:lang=ja'` (and zh-cn, zh-tw, ko, th, he, ar, hi) the fallbacks, keeping fonts whose `lang` really covers the language. Each file is test-loaded inside `catch_unwind` because egui panics on fonts it can't parse. `itami --font-info` prints what was picked.

## Thumbnails

Pipeline on a pool worker: cache hit with matching `Thumb::MTime` -> done; skip known failures; sniff magic bytes (PNG, JPEG, GIF, WebP, BMP); skip files over 64 MB or images over 16,384 px a side / 100 MP; for JPEG try the EXIF thumbnail, then a 1/2-1/8 downscaled decode; decode in `catch_unwind`; apply EXIF orientation; box-filter to size; write the cache PNG atomically (temp + rename, 0600).

Cache: `$XDG_CACHE_HOME/thumbnails/{normal,large,x-large,xx-large}/<md5 of URI>.png`, URIs escaped exactly as GLib does so names match Nautilus. Failures go in `fail/itamiiru-<version>/`. Never thumbnail inside the thumbnail directory. The UI requests only visible items and keeps at most 512 textures (LRU).

`crates/itamiiru-core/tests/corpus.rs` damages the seeds in `tests/corpus/seeds/` thousands of ways and checks no decoder panics.

## Platform notes

**Linux:** drives from `/proc/self/mountinfo` (decode `\040` escapes; hide pseudo filesystems and bind duplicates), labels from `/dev/disk/by-label/`, usage from `statvfs`, removable drives from UDisks2 (D21). Places from `user-dirs.dirs` and `~/.config/gtk-3.0/bookmarks`. `mime:` filters via `globs2`. Trash per the freedesktop spec. Open with `xdg-open`.

**Windows (deferred):** drives via hand-declared `GetLogicalDrives`, `GetDriveTypeW`, `GetVolumeInformationW`, `GetDiskFreeSpaceExW`; places via `SHGetKnownFolderPath`; hidden via `FILE_ATTRIBUTE_HIDDEN`; trash via `SHFileOperationW` + `FOF_ALLOWUNDO`; open via `ShellExecuteW`; local time via `GetTimeZoneInformation` (dates are UTC until then).

**macOS (deferred):** drives from `/Volumes` with `statfs`; hidden also by `UF_HIDDEN`; trash undecided (`osascript` or objc FFI); open via `open`.

## Portal backend

- **D-Bus client:** address from `$DBUS_SESSION_BUS_ADDRESS` (`unix:path=`, `unix:abstract=`), else `$XDG_RUNTIME_DIR/bus`. SASL EXTERNAL, `Hello`, `RequestName(..., DO_NOT_QUEUE)`. Reads both endiannesses, writes little-endian, rejects messages over 1 MiB.
- **Objects:** `/org/freedesktop/portal/desktop` with `impl.portal.FileChooser` (`OpenFile`, `SaveFile`, `SaveFiles`), `Introspectable`, `Properties` (no properties), `Peer`. Each call's `handle` path exposes `impl.portal.Request.Close` until the reply.
- **Event loop:** a reader thread and one thread per pick feed an mpsc channel; the main thread owns the socket writer and a `handle -> Child` map. Replies are delayed until the picker exits. Close kills the child and replies 2.
- **Results:** `uris`, `current_filter` (echoed in its original form), `choices`. SaveFiles joins the folder with each name from `files`.
- **Picker lookup:** `$ITAMI_BIN`, then next to `current_exe()`, then `PATH`.

## Getting apps to use the portal

Results go in `docs/app-compat.md`.

| App / toolkit | Setting | Notes |
|---|---|---|
| Flatpak apps | none | Always use the portal |
| GTK3 | `GTK_USE_PORTAL=1` | |
| GTK4 | `GDK_DEBUG=portals` | Newer GTK may use the portal by default |
| Qt 5/6 | `QT_QPA_PLATFORMTHEME=xdgdesktopportal` | Replaces qt5ct/qt6ct/KDE theming |
| Firefox | `widget.use-xdg-desktop-portal.file-picker = 1` | |
| Chromium/Electron | Usually automatic | KDE sessions may use kdialog |
| LibreOffice | GTK VCL plugin, `GTK_USE_PORTAL=1` | |
| JetBrains, Java/Swing, Tk, Wine | not possible | Own dialogs |

Session variables: Wayland/systemd sessions use `~/.config/environment.d/`; X11 sessions read `~/.xsessionrc` (Debian, Ubuntu; runs under `set -e`, so guard every line) or `~/.xprofile` (elsewhere). The README has the lines.

## Remaining tasks

- [ ] P9.1 Attach the picker to its parent window (X11 transient-for, Wayland xdg-foreign); see what winit exposes and how COSMIC did it. Until then windows open centered on the primary monitor.
- [ ] P9.2 Accessibility decision (D10).
- [ ] P9.4 inotify live updates via libc, replacing polling (D9).
- [ ] P9.7 Packaging: an Arch PKGBUILD lives in the separate `kutsubako` repo; deb/rpm not done.
- Known gaps: encrypted volumes, drag into itamiiru and drags on Wayland, custom actions in the sidebar and This PC menus, a properties dialog for ShowItemProperties.

**Deferred: Windows and macOS.** The code keeps compiling for both (`cargo check` stays in the checklist). To do: the Windows platform module, fonts and cache location; the macOS platform module and trash; testing on real machines.

## Testing recipes

```bash
# Picker alone
cargo run -p itamiiru -- --pick < tests/requests/open-images.req; echo "exit: $?"

# Backend directly, bypassing the frontend
gdbus call --session \
  --dest org.freedesktop.impl.portal.desktop.itamiiru \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.impl.portal.FileChooser.OpenFile \
  /org/freedesktop/portal/desktop/request/1_1/test "test.app" "" "Pick something" {}

# Through the frontend (the reply arrives as a Response signal; watch it in a second terminal)
gdbus monitor --session --dest org.freedesktop.portal.Desktop
gdbus call --session --dest org.freedesktop.portal.Desktop \
  --object-path /org/freedesktop/portal/desktop \
  --method org.freedesktop.portal.FileChooser.OpenFile "" "Test" {}

# A real GTK client
GTK_USE_PORTAL=1 GDK_DEBUG=portals zenity --file-selection

# Watch which backend the frontend picks (path is /usr/libexec/ or /usr/lib/)
systemctl --user stop xdg-desktop-portal
/usr/libexec/xdg-desktop-portal -r -v

# Raw D-Bus traffic, and the backend's logs
dbus-monitor --session "interface='org.freedesktop.impl.portal.FileChooser'"
journalctl --user -f | grep -i itamiiru
```

`scripts/make-test-dirs.sh` and `scripts/make-test-photos.sh` build folders for checking big listings, odd names and thumbnails by hand.

## Gotchas

- **D-Bus activation environment:** the portal inherits the systemd/dbus manager's environment, not your shell's. Without `DISPLAY`/`WAYLAND_DISPLAY` the picker can't open a window: `dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP`.
- **portals.conf** names the backend by the `.portal` file stem (`itamiiru`). Restart the frontend after changes: `systemctl --user restart xdg-desktop-portal`.
- **`current_folder` and `current_file` are `ay`**, NUL-terminated bytes, not strings.
- **`pkill -x` matches only 15 characters**; use `pkill -f xdg-desktop-portal-itamiiru`.
- **Tiling WMs** need a float rule for app_id `itamiiru`, e.g. sway: `for_window [app_id="itamiiru"] floating enable`.
- **XFCE:** Thunar's daemon owns FileManager1 until stopped, and `exo-open` ignores `mimeapps.list` for folders (set Preferred Applications' FileManager).
- **Non-UTF-8 filenames** exist: test with `touch $'bad\xffname'`.

## Dependency budget

| Date | After | Unique crates | Notes |
|---|---|---|---|
| 2026-10-12 | D25 | 180 (glow) / 217 (wgpu) | Current |
| 2026-10-11 | P0.4 baseline | 168 | eframe 0.36.2 (glow, wayland, x11, default_fonts) |
