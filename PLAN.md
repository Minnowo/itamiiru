# itamiiru: plan

## 1. Goals and non-goals

**Goals**
- A single, consistent file picker for as many Linux apps as possible, delivered as an xdg-desktop-portal FileChooser backend.
- A file explorer with the same UI that runs on Linux, Windows and macOS.
- A Windows 10/11-style layout: navigation pane, breadcrumb address bar, details and icon views, and a filename and filter row at the bottom.
- Drives shown with usage, plus image thumbnails decoded in pure Rust.
- Very few crates and system dependencies, with small helpers vendored.
- A UI that never freezes, even on hung network mounts.

**Non-goals (for now)**
- Video thumbnails, and any thumbnailing that needs C decoders.
- Replacing the OS file dialog on Windows or macOS. That isn't possible system-wide, so on those platforms only the explorer applies.
- Apps that never use the portal (Java/Swing, Tk, Wine, apps that embed a chooser widget). We document them; we don't fight them.
- Network browsing (SMB discovery and so on). Already-mounted network drives are shown.

## 2. Architecture

```
 App (Firefox, Chromium, GTK, Qt, Flatpak ...)
   | D-Bus: org.freedesktop.portal.FileChooser
   v
 xdg-desktop-portal (frontend, system daemon)
   | reads ~/.config/xdg-desktop-portal/portals.conf
   | D-Bus: org.freedesktop.impl.portal.FileChooser
   v
 xdg-desktop-portal-itamiiru (crate itamiiru-portal, Linux only, no GUI)
   | spawns a child process, writes the request to its stdin   (protocol: section 5)
   v
 itami --pick (crate itamiiru, egui)
   | shows the dialog, prints the response to stdout, exits
   v
 back up the chain as file:// URIs
```

**Why separate processes:** the portal stays small and has no GUI dependencies. A crash in the UI can't take down the D-Bus service. The picker runs standalone, cross-platform and is testable from a terminal. Each pick starts a fresh process, which is fine as long as startup stays under roughly 150 ms (measure it).

**Crates**

| Crate | Kind | Contains |
|---|---|---|
| `itamiiru-core` | lib | Directory listing and entry model, worker pools, drives and places, thumbnails and cache, picker protocol, vendored helpers, `platform/` |
| `itamiiru` | bin `itami` | The egui UI. `itami [PATH]` gives explorer mode; `itami --pick` gives picker mode, reading its request from stdin. |
| `itamiiru-portal` | bin `xdg-desktop-portal-itamiiru` | Hand-written D-Bus client, FileChooser backend, later FileManager1 |

**Vendored modules (in itamiiru-core unless noted)**

| Module | Purpose | Approx. size |
|---|---|---|
| `percent.rs` | Percent-encoding, and `file://` URI <-> path conversion from raw bytes | 80 |
| `natsort.rs` | Natural sort, so `file2` comes before `file10` | 50 |
| `glob.rs` | `*`, `?` and `[...]` matching, used for filters | 80 |
| `md5.rs` | Thumbnail cache file names | 120 |
| `exif.rs` | Orientation tag and embedded JPEG thumbnail | 200 |
| `resize.rs` | Box-filter downscaling of RGBA images | 40 |
| `protocol.rs` | Picker <-> portal messages | 200 |
| `platform/linux.rs` | mountinfo, statvfs, user-dirs, GTK bookmarks, MIME globs2, trash | 500 |
| `itamiiru-portal/src/dbus/` | Connection, auth, marshalling, message loop | 1000-1500 |

## 3. Decision log

| # | Decision | Choice | Why | Revisit if |
|---|---|---|---|---|
| D1 | UI toolkit | egui via eframe | Easiest for a Rust beginner; self-contained binary; cross-platform; redraws only on events | Accessibility or native look become hard requirements |
| D2 | Renderer | glow (OpenGL) | Far fewer crates than wgpu | A GL driver issue on some target |
| D3 | Concurrency | std threads + mpsc channels | File I/O isn't truly async anyway; no runtime; easy to reason about | Never, for this project |
| D4 | Picker <-> portal | Separate processes, stdin/stdout line protocol | Isolation, simplicity, standalone testing | Startup time becomes noticeable |
| D5 | D-Bus | Hand-written minimal client (done in Phase 3: about 1,100 lines with tests) | Few crates; the portal surface is tiny | It takes more than about a week or proves flaky. Fallback: zbus's blocking API behind the same `dbus` module interface (ask first) |
| D6 | Image decoding | Individual pure-Rust format crates, no `image` umbrella | Memory safety on untrusted files; fewer crates | - |
| D7 | Video thumbnails | Out of scope | No pure-Rust decoders | Later, as an optional sandboxed subprocess |
| D8 | Thumbnail cache | freedesktop spec format (own directory on Windows and macOS) | Shared with Nautilus, Dolphin and others | - |
| D9 | Live directory updates | Poll the directory's mtime every 2 s while focused, plus F5 | No crate, works everywhere | inotify via libc, in Phase 9 |
| D10 | Accessibility (AccessKit) | Off at first | On Linux it pulls in zbus and atspi, a large dependency cost | Decide before any public release. A system-wide picker that screen readers can't use is a real problem. |
| D11 | Details table | `egui_extras::TableBuilder` | Virtualized rows and resizable columns; same repo as egui | Replace with hand-written code if it's limiting |
| D12 | JPEG crate | *TBD in the P5.1 spike* | Pick whichever supports fast downscaled (1/2, 1/4, 1/8) decoding | - |
| D13 | Look and colors | ponkotsu design system (simplified Material 3, https://github.com/Minnowo/ponkotsu-css) in `crates/itamiiru/src/theme.rs`. All ~90 color roles; built-in default is ponkotsu's dark palette. A theme file (`$ITAMI_THEME` or `<config dir>/theme`) overrides any role, as `role = #hex` lines or pasted ponkotsu CSS (`--color-c-role: #hex;`) | One consistent design system; themes are just data | egui can't express something the design needs |
| D14 | Text sizes | ponkotsu's web sizes scaled from a 16 px to a 14 px body (heading 19, small 12) | A file manager is denser than a web form | Text feels too small or too large in use |
| D15 | Natural sort case | ASCII letters compare case-insensitively; other letters by raw bytes | Simple and allocation-free | Non-English names sort badly in practice |
| D16 | UI scaling | egui's built-in zoom (Ctrl+Plus, Ctrl+Minus, Ctrl+0; 10% steps), plus `[-] 100% [+]` buttons in the status bar, the percentage resets | One knob scales text, icons and spacing together, like a browser; painted icons stay sharp at any size | - |
| D17 | File clipboard | In-app list of cut/copied paths; the paths also go on the system clipboard as text | egui's clipboard is text-only, and egui only reports Ctrl+V when the clipboard holds text, so putting the paths there makes Ctrl+V reach us and lets them paste into a terminal. No file interop with other file managers | Interop with other file managers is wanted (needs `text/uri-list` / `x-special/gnome-copied-files` clipboard targets) |

## 4. Threading model

The UI thread never does filesystem I/O. Workers do, and each one wakes the UI with `ctx.request_repaint()` after sending a result.

```
UI thread --Job--> [listing worker]      --Done--> UI thread (try_recv each frame)
          --Job--> [drive/places worker] --Done-->
          --Job--> [thumbnail pool: 2-4] --Done-->
          --Job--> [file-op worker]      --Done-->   (explorer mode)
```

- **Channels:** `std::sync::mpsc`. A worker loop ends when the UI drops its `Sender`. A worker stops if `send` fails, which means the UI is gone.
- **Generation ids:** every navigation increments `current_gen`. Jobs carry `gen`, and the UI ignores results whose `gen` doesn't match. We can't kill a thread stuck in a hung `read_dir`, but we can make its result harmless.
- **One listing worker per request.** Spawn a thread per navigation rather than using one long-lived worker, so a hung mount blocks only its own thread and never the next navigation. Threads stuck in the kernel are leaked until the call returns. That's acceptable.
- **Streaming:** listings arrive in batches of about 500 entries (`Done::Batch`, then `Done::Finished`).
- **Thumbnail pool:** N threads (N = min(4, cores - 1), at least 1) share a **LIFO stack** (`Mutex<Vec<Job>>` + `Condvar`). The most recently requested thumbnail, which is what's on screen, gets handled first. Keep an in-flight set to avoid duplicates, and clear the stack on navigation.
- **Cancellation** of long jobs (search, copy) uses an `Arc<AtomicBool>` checked inside the loop.
- **Panics:** each thumbnail decode is wrapped in `std::panic::catch_unwind`, so one bad file doesn't kill a pool thread.
- **Drive scanning** (`statvfs`) also runs on a worker, one thread per mount. `statvfs` on a dead network mount can block.
- **Test hook:** in debug builds, `ITAMI_DEBUG_SLOW_FS=<ms>` makes workers sleep before opening a folder and before each further batch, to prove the UI stays responsive. (Sleeping before *every* call, as first planned, made a 3000 ms test take minutes per folder.)

## 5. Picker <-> portal protocol (v1)

The portal writes a request to the picker's **stdin**. The picker writes a response to **stdout** and then exits. Both sides are implemented once, in `itamiiru-core/src/protocol.rs`.

**Encoding rules**
- UTF-8 text, one record per line, `\n` line endings.
- A line is a keyword followed by fields, separated by single spaces.
- Every field is percent-encoded. Encoders **must** encode space, `%`, control bytes (< 0x20, 0x7F) and every byte >= 0x80, and **may** encode anything else. Decoders decode every `%XX`.
- The single token `-` means the empty string; a literal "-" is written as `%2D`.
- Paths are absolute and percent-encoded from their **raw bytes**, so non-UTF-8 filenames survive the round trip.
- Unknown keywords are ignored, which leaves room for forward compatibility.

**Request** (first line `itamiiru-protocol 1`, last line `end`)

| Keyword | Fields | Meaning |
|---|---|---|
| `mode` | `open` \| `save` \| `save-files` \| `folder` | Dialog type. `save-files` is a folder pick; the portal appends the names. |
| `title` | text | Window title |
| `accept_label` | text | Text of the Open/Save button |
| `multiple` | `0`\|`1` | Allow selecting several files (open mode) |
| `modal` | `0`\|`1` | Informational only for now |
| `app_id` | text | Requesting app, used to remember the last folder per app |
| `parent` | e.g. `x11:1a00004` or `wayland:...` | Parent window handle (used in Phase 9) |
| `folder` | path | Start folder |
| `name` | text | Suggested file name (save mode) |
| `file` | path | Existing file to save over (save mode) |
| `filter` | name, then one or more `glob:PATTERN` / `mime:TYPE` | One file-type filter; repeatable, order kept |
| `current_filter` | index | Filter selected at start |
| `choice` | id, label, default, then pairs of option-id and option-label | Extra control in the footer. No pairs means a checkbox whose default is `true` or `false`. Repeatable. |

**Response** (first line `itamiiru-protocol 1`)

| Keyword | Fields | Meaning |
|---|---|---|
| `uri` | `file://` URI | One per selected item, repeatable |
| `filter` | index | Filter that was active |
| `choice` | id, value | Final value of each choice |

**Exit codes:** 0 means OK, 1 means cancelled, anything else is an error (portal response 2). Samples are in `tests/requests/`.

## 6. UI specification

### Layout (picker mode; explorer mode drops the bottom row)

```
+----------------------------------------------------------------------------+
| [<] [>] [^]   Home > me > Documents > Projects          [ Search Projects  ]|
| [New folder]                                    [View: Details v] [Hidden] |
+-------------------+--------------------------------------------------------+
| Quick access      | Name ^                 Date modified     Type      Size|
|   Home            | [D] assets             2026-10-01 14:22  Folder        |
|   Desktop         | [D] src                2026-10-09 11:03  Folder        |
|   Documents       | [F] notes.txt          2026-10-10 18:03  TXT File  2 KB|
|   Downloads       | [I] photo.jpg          2026-10-09 11:47  JPG File 3 MB |
|   Pictures        |                                                        |
| Bookmarks         |                                                        |
|   work            |                                                        |
| This PC           |                                                        |
|   System (/) ##.  |                                                        |
|   Data     #..    |                                                        |
|   USB Stick ###.  |                                                        |
| Network           |                                                        |
+-------------------+--------------------------------------------------------+
| File name: [ notes.txt                             ] [ Text files (*.txt) v]|
| Encoding:  [ UTF-8 v ]                                  [ Open ] [ Cancel ]|
+----------------------------------------------------------------------------+
```

- **Navigation pane:**
  - *Quick access* lists Home and the XDG user dirs.
  - *Bookmarks* come from `~/.config/gtk-3.0/bookmarks`, the same list GTK apps use.
  - *This PC* shows mounted drives with usage bars. Clicking the heading opens a drive-tiles page, like Windows.
  - *Network* shows gvfs mounts under `/run/user/$UID/gvfs`, plus network filesystems.
- **Address bar:** clickable breadcrumb segments. Clicking empty space, Ctrl+L, Alt+D or F4 switches it to a text field. Enter navigates; `~` expands; Escape reverts.
- **File list views:** Details (default), List, and Medium/Large/Extra-large icons, which show thumbnails. In Details, headers sort (click again to reverse), columns resize, and folders always come first. Sorting is natural.
- **Type column:** Windows-style `"PNG File"` (uppercase extension + " File"), `"Folder"`, or `"File"` when there's no extension.
- **Icons:** painted in code with egui's painter (folder, generic file, image, archive, text, executable badge). No icon assets and no icon-theme lookup.
- **Preview pane:** optional, toggled from the toolbar. Shows a large thumbnail and file details.

### Behaviour of the bottom row (picker mode, matching Windows)

- The **filename field** follows the selection. With several items selected it shows `"a.txt" "b.txt"`.
- Typing a **path** (absolute, or relative to the current folder) and pressing Enter either navigates there (folder) or selects it (file).
- Typing a **glob** such as `*.log` and pressing Enter applies it as a temporary filter.
- **Save mode:**
  - The suggested name is prefilled, with only the stem selected.
  - If the typed name has no extension and the active filter has exactly one `*.ext` glob, that extension is appended.
  - Choosing an existing file prompts an overwrite confirmation.
  - If the folder isn't writable, show an inline error rather than returning.
- **Folder mode:** shows folders only. The button reads "Select Folder". With nothing selected, it returns the current folder.
- **Filter dropdown:** lists the request's filters, applied live.
- **Choices:** shown as combo boxes, or checkboxes, in the footer to the left of the buttons.

### Keyboard

| Key | Action |
|---|---|
| Enter | Open folder / accept |
| Backspace, Alt+Up | Up one folder (from `/`, to This PC) |
| Alt+Left / Alt+Right | Back / forward |
| Ctrl+L, Alt+D, F4 | Edit address bar |
| Ctrl+F, F3 | Focus search |
| Ctrl+H | Toggle hidden files |
| Ctrl+Plus / Ctrl+Minus / Ctrl+0 | Zoom in / out / reset, 10% steps (section 3 D16) |
| Ctrl+Shift+N | New folder |
| F5 | Refresh |
| Ctrl+A | Select all |
| Typing letters in the list | Jump to the first matching name (type-ahead) |
| Escape | Cancel the dialog / close the menu |
| F2, Delete, Shift+Delete, Ctrl+C/X/V | Explorer mode: rename, trash, delete permanently, copy/cut/paste |

### Persistence (`~/.config/itamiiru/config`, key=value)

View mode, sort column and direction, column widths, sidebar width, window size, zoom level, hidden-files toggle, and the last folder per `app_id`.

### Optional features (settings)

Features the developer may want to switch on or off. Each one lives in its own module, hooks into the rest of the UI at a single call site, and is checked against a `key=value` setting in the config file (section 6 Persistence). With the setting off, the hook is skipped and no other code changes behaviour. The two auto-refresh settings are already read from the config file (edit it by hand until there's a settings page); `breadcrumb_dropdowns` waits for P4.6b.

| Setting | Default | Feature | Where it hooks in |
|---|---|---|---|
| `breadcrumb_dropdowns` | off | Explorer's arrow between crumbs opens a menu of that folder's subfolders (task P4.6b) | The `separator()` call in `address.rs`: when on, the arrow becomes a button that opens the menu |
| `auto_refresh_folder` | on | Re-list the open folder when its modified time changes, checked every 2 s on a worker while the window is focused (D9) | Early return at the top of `App::check_for_changes`; F5 still works when off |
| `auto_refresh_drives` | on | Rescan places (drives, bookmarks, network) every 5 s while focused, and re-measure drive usage | Early return at the top of `App::check_places`; places are still scanned once at startup and when This PC opens |

### Theme (section 3 D13)

- egui's global style is set from the theme once at startup, so plain widgets already follow it: `ui.button` is ponkotsu's plain button, text edits are its inputs.
- Containers are `theme.surface(n)` frames: fill tone only, no borders or shadows. Bump `n` for each nested box.
- Buttons: `theme.filled_button(ui, Accent::Primary, ..)` for the one main action, `theme.outlined_button` for a colored secondary action, `ui.button` for everything else. Cancel is always plain.
- Selected rows use `secondary-container`. Categorical colors (`pink`, `red`, `yellow`, `green`, `blue`) are for painted file-type icons and drive bars, never for errors.
- Loading a different theme while running, and light/dark switching (P9.6), are not done yet.

### Fonts

egui's built-in fonts lack CJK and most emoji, so filenames in those scripts would show as boxes.

- **Linux:** at startup, on a worker thread, run `fc-match -f '%{file}|%{index}' ':lang=ja'` (and similar for zh-cn, zh-tw, ko, th, he, ar, hi) and add the resulting files as fallback fonts. Emoji are covered by egui's built-in monochrome emoji font; system color emoji fonts (CBDT/sbix bitmaps) can't be drawn by egui anyway. egui doesn't shape complex scripts, so Arabic and Indic names show unjoined, but not as boxes.
- **Windows:** use known paths in `C:\Windows\Fonts`.
- **macOS:** use known paths in `/System/Library/Fonts`.

## 7. Thumbnails

**Pipeline** (inside a thumbnail pool worker):
1. Look in the cache. If there's a hit and `Thumb::MTime` matches the file, decode the cached PNG and finish.
2. Skip the file if it's in the failure list (in memory, or in the cache's `fail/` directory).
3. Sniff the format from the first 32 bytes using magic numbers: PNG, JPEG, GIF, WebP, BMP. Skip if unknown.
4. Check the file size: skip anything over 64 MB.
5. Read the header dimensions; skip anything over 16,384 px on either side or 100 megapixels in total.
6. For JPEG, try the EXIF embedded thumbnail first, and fall back to a downscaled decode (1/2, 1/4 or 1/8).
7. Decode inside `catch_unwind`.
8. Apply the EXIF orientation.
9. Box-filter down to the target size (128 or 256 px, by icon view).
10. Write the cache PNG atomically (temp file + rename, permissions 0600), then send the RGBA buffer to the UI.

**Cache format** (freedesktop thumbnail spec): `$XDG_CACHE_HOME/thumbnails/{normal,large,x-large,xx-large}/` (128, 256, 512 and 1024 px). File name = lowercase hex MD5 of the canonical `file://` URI, plus `.png`. Required tEXt chunks are `Thumb::URI` and `Thumb::MTime`. Failures are recorded in `fail/itamiiru-<version>/`. Never thumbnail files inside the thumbnail directory itself. On Windows and macOS, use the same layout under the OS cache directory.

**UI side:** request thumbnails only for visible items. Upload with `egui::ColorImage::from_rgba_unmultiplied` and `ctx.load_texture`. Keep an LRU of about 512 `TextureHandle`s to bound GPU memory.

**Malformed-file corpus:** maintain `tests/corpus/` (truncated files, files whose headers claim enormous dimensions, random bytes behind valid magic numbers, zero-byte files). A test decodes every file and asserts there are no panics and no allocations above the limits.

## 8. Platform notes

### Linux
- **Drives:** parse `/proc/self/mountinfo`, which escapes spaces as `\040` and so on; decode them. Hide pseudo filesystems (proc, sysfs, cgroup*, devtmpfs, devpts, tmpfs, overlay, squashfs, autofs, bpf, tracefs, debugfs, securityfs, pstore, fusectl, configfs, mqueue, hugetlbfs, binfmt_misc, nsfs, ramfs, efivarfs) and bind-mount duplicates. Show `/`, `/home` if it's separate, `/boot*` only if hidden files are on, and everything under `/run/media/$USER`, `/media` and `/mnt`. Get labels by resolving the symlinks in `/dev/disk/by-label/` and matching mount sources. Get usage from `statvfs` (via libc).
- **Places:** `~/.config/user-dirs.dirs` (lines like `XDG_DOWNLOAD_DIR="$HOME/Downloads"`) and `~/.config/gtk-3.0/bookmarks` (`file:///path optional label`).
- **MIME filters:** turn `mime:` filters into globs with `/usr/share/mime/globs2` (`weight:type:glob`), also checking `$XDG_DATA_HOME` and `$XDG_DATA_DIRS`. Support wildcards like `image/*`.
- **Hidden files:** dotfiles. Optionally also names listed in the folder's `.hidden` file (a GNOME convention).
- **Trash:** freedesktop trash spec, using `~/.local/share/Trash/{files,info}` and `.trashinfo` files; per-volume `.Trash-$UID` for other drives.
- **Open with default app:** `xdg-open`.

### Windows (Phase 8)
- **Drives:** declare `GetLogicalDrives`, `GetDriveTypeW`, `GetVolumeInformationW` and `GetDiskFreeSpaceExW` by hand with `extern "system"` in `platform/windows.rs`.
- **Places:** `SHGetKnownFolderPath`, which handles relocated folders like Downloads.
- **Hidden files:** `FILE_ATTRIBUTE_HIDDEN` via `std::os::windows::fs::MetadataExt`.
- **Trash:** `SHFileOperationW` with `FOF_ALLOWUNDO`.
- **Open:** `ShellExecuteW`.

### macOS (Phase 8)
- **Drives:** `/Volumes`, with `statfs` via libc.
- **Places:** the standard folders under `~`.
- **Hidden files:** dotfiles and `UF_HIDDEN` in `st_flags`.
- **Trash:** undecided. Options are `osascript` asking Finder (slow, no dependencies) or objc FFI.
- **Open:** `open`.

## 9. Portal backend design (itamiiru-portal)

**D-Bus client (hand-written, `src/dbus/`):**
- **Address:** `$DBUS_SESSION_BUS_ADDRESS`, handling `unix:path=` and `unix:abstract=` (std `UnixStream` with `SocketAddrExt` on Linux) and ignoring `,guid=`. Fall back to `$XDG_RUNTIME_DIR/bus`.
- **Auth:** send `\0`, then `AUTH EXTERNAL <hex of uid as ASCII decimal>\r\n`, expect `OK <guid>`, then send `BEGIN\r\n`.
- **Startup:** call `Hello`, then `RequestName("org.freedesktop.impl.portal.desktop.itamiiru", 4 /* DO_NOT_QUEUE */)`.
- **Messages:** read both endiannesses and always write little-endian. Implement header fields, serials and reply_serial. Use a signature-driven `Value` enum for marshalling. Reject messages over 1 MiB.
- **Signatures needed:** `y b u s o g v ay aay as a{sv} a(us) (sa(us)) a(sa(us)) a(ss) a(ssa(ss)s)`.
- Capture real traffic with `dbus-monitor --session` to build marshalling test fixtures.

**Objects:**
- `/org/freedesktop/portal/desktop` exposes:
  - `org.freedesktop.impl.portal.FileChooser`: `OpenFile`, `SaveFile`, `SaveFiles`
  - `org.freedesktop.DBus.Introspectable`, returning a static XML string
  - `org.freedesktop.DBus.Properties`: the impl FileChooser interface defines no properties (checked against the XML, so no `version`): `GetAll` returns an empty dict, `Get` an UnknownProperty error.
  - `org.freedesktop.DBus.Peer`: `Ping`
- The `handle` object path passed with each call exposes `org.freedesktop.impl.portal.Request.Close`, from the moment the call arrives until the reply is sent.

**Event loop** (no async):
```
reader thread:   socket -> parse -> Event::Incoming(msg) ----------------+
                                                                         +--> mpsc --> main thread
per-pick thread: child stdout -> Event::PickDone(handle, output, status) +    (owns the socket writer
                                                                              and a map handle -> Child)
```
- **OpenFile / SaveFile / SaveFiles:** translate the options into a protocol request, spawn the picker, take its stdin and stdout, write the request, then hand stdout to a new per-pick thread. Store the `Child` in a map keyed by handle. Don't reply yet: D-Bus allows a delayed reply.
- **PickDone:** `wait()` on the child, parse the output, and reply `(u response, a{sv} results)`:
  - `uris` (as)
  - `current_filter` ((sa(us)), echoing the chosen filter back in its original form)
  - `choices` (a(ss))
  - For SaveFiles, build the URIs by joining the chosen folder with each name from the `files` option (`aay`).
- **Close:** kill the child and reply with response 2. Confirm against the frontend's behaviour in its `-v` logs.
- **Options to handle:** `accept_label`, `modal`, `multiple`, `directory`, `filters`, `current_filter`, `choices` (`a(ssa(ss)s)`: id, label, options, then the default), `current_name`, `current_folder` (`ay`, NUL-terminated), `current_file` (`ay`), `files` (`aay`, SaveFiles). A `current_filter` that isn't in `filters` is added to them.
- **Picker binary lookup:** `$ITAMI_BIN`, then the directory of `current_exe()`, then `PATH`.

**Install files (`data/`):**
- `itamiiru.portal`, installed to `/usr/share/xdg-desktop-portal/portals/`
- the D-Bus service file, installed to `~/.local/share/dbus-1/services/` for development
- `portals.conf.example`

`scripts/install-dev.sh` does all of this.

## 10. Getting apps to use the portal

Record what actually happens in `docs/app-compat.md` (Phase 6).

| App / toolkit | Setting | Notes |
|---|---|---|
| Flatpak apps | none | Always use the portal |
| GTK3 | `GTK_USE_PORTAL=1` | |
| GTK4 | `GDK_DEBUG=portals` | `GtkFileDialog` may use the portal by default on newer GTK. Verify. |
| Qt 5/6 | `QT_QPA_PLATFORMTHEME=xdgdesktopportal` | Overrides qt5ct/qt6ct/KDE theme selection. Check side effects on theming. |
| Firefox | `about:config`: `widget.use-xdg-desktop-portal.file-picker = 1` | |
| Chromium/Chrome/Electron | Usually automatic when the portal FileChooser is available | Verify per version; KDE sessions may use kdialog instead |
| LibreOffice | uses the GTK VCL plugin, which respects `GTK_USE_PORTAL=1` | Verify |
| JetBrains, Java/Swing, Tk, Wine | not possible | Own dialogs |

Set the environment variables session-wide in `~/.config/environment.d/60-itamiiru.conf` (see `data/itamiiru-env.conf.example`). Log out and back in afterwards.

## 11. Phases

Each phase ends with its "Done when" check passing, plus `cargo test`, `cargo clippy -D warnings` and the Windows `cargo check`.

### Phase 0: Setup
- [x] P0.1 Cargo workspace with `crates/itamiiru-core` (lib), `crates/itamiiru` (bin named `itami`), `crates/itamiiru-portal` (bin named `xdg-desktop-portal-itamiiru`, Linux-only via `cfg`). Edition 2024. Workspace lints: `unsafe_code = "deny"`, clippy defaults.
- [x] P0.2 Add eframe with minimal features, after verifying the feature names against eframe's current Cargo.toml. Show an empty window titled "itamiiru" with app_id/class `itamiiru`.
- [x] P0.3 Add the `debug!` macro and the `ITAMI_DEBUG` env handling to core.
- [x] P0.4 Record the baseline dependency count in section 14.
- [x] P0.5 Install the Windows target and confirm `cargo check` passes for itamiiru-core and itamiiru.

**Done when:** `cargo run -p itamiiru` opens a window, and all checks pass.

### Phase 1: Core listing and basic browser
- [x] P1.1 Entry model: name (`OsString`), path, kind (file, dir, symlink to dir, symlink to file, broken symlink, other), size, mtime, hidden flag.
- [x] P1.2 Listing worker as in section 4: thread per request, generation ids, batches of 500, per-entry errors tolerated, a whole-directory error reported.
- [x] P1.3 `ITAMI_DEBUG_SLOW_FS` test hook.
- [x] P1.4 `natsort.rs` with tests. Sort by name, size, mtime or type; folders first; ascending or descending.
- [x] P1.5 Details view with `TableBuilder`: virtualized rows, sortable headers, resizable columns, human-readable sizes, local-time dates.
- [x] P1.6 Navigation: back/forward/up history, plain-text address bar (breadcrumbs come in Phase 4), double-click or Enter opens a folder.
- [x] P1.7 Selection: click, Ctrl+click, Shift+click, Ctrl+A, arrow keys, type-ahead.
- [x] P1.8 Hidden-file toggle (Ctrl+H) and F5 refresh; mtime polling every 2 s while focused. A refresh keeps the old list on screen until the new one is complete, so it doesn't flash. Folder mtime changes only on add/remove/rename, so edits to a file's contents don't trigger it (F5 does).
- [x] P1.9 Painted file-type icons.
- [x] P1.10 System fallback fonts (section 6 Fonts). Loaded on a worker thread after the window opens, added at lowest priority.
- [x] P1.11 Inline error display (permission denied, missing folder), never a modal.

**Done when:**
- You can browse the whole filesystem.
- A 50,000-entry test directory scrolls smoothly.
- `ITAMI_DEBUG_SLOW_FS=3000` never freezes the UI, and navigating away while a listing is slow works.
- Non-UTF-8 and CJK filenames display.

`scripts/make-test-dirs.sh` builds the folders for these checks. Measured sort cost (ignored test `stream_sort_50k`): streaming 50,000 entries in 100 batches, the worst batch re-sort is 2.8 ms in release (14 ms debug); a full re-sort is 4.5 ms (25 ms debug).

### Phase 2: Picker mode and protocol

UI in `crates/itamiiru/src/picker.rs`; typed names are checked on a worker (`itamiiru-core/src/probe.rs`) before acting. The footer puts Cancel before the Open/Save button (ponkotsu: main action last).

- [x] P2.1 `percent.rs` (encode/decode, `file://` URI <-> path from raw bytes) with tests, including non-UTF-8 paths.
- [x] P2.2 `protocol.rs`: request/response structs, parse and serialize, round-trip tests, tests for the `-` empty-field rule.
- [x] P2.3 `itami --pick`: read the request from stdin, configure the dialog, write the response to stdout, set the exit code. Closing the window means cancel.
- [x] P2.4 Open mode: single and multiple selection, and the filename-field behaviours in section 6.
- [x] P2.5 Folder mode.
- [x] P2.6 Save mode: prefill and select the stem, append the extension, overwrite confirmation, writability check, `file` handling.
- [x] P2.7 `glob.rs` with tests (case-sensitive; `[pP][nN][gG]`-style patterns work). Filter dropdown; filters applied live.
- [x] P2.8 `mime:` filters via globs2 on Linux; a small built-in table (common image, text, audio, video, archive and document types) elsewhere.
- [x] P2.9 Choices UI (combo boxes and checkboxes).
- [x] P2.10 Config file and last folder per app. (Config file: `itamiiru-core/src/config.rs`. Last folder: `last_folder.<percent-encoded app_id>` key, written when a pick succeeds; used when the request has no folder or file.)
- [x] P2.11 Picker keyboard shortcuts (section 6).

**Done when:** every file in `tests/requests/` behaves correctly, and `cargo run -p itamiiru -- --pick < tests/requests/save-text.req` prints a valid response.

### Phase 3: Portal backend (Linux)

`crates/itamiiru-portal/src/{dbus/{wire,msg,conn}.rs, portal.rs, main.rs}`. `scripts/test-portal.sh` runs it on the session bus with a fake picker and checks OpenFile, SaveFiles, cancel, Close, Ping and errors (no window needed).

- [x] P3.1 `dbus/conn.rs`: address parsing, connect, SASL EXTERNAL, Hello, RequestName.
- [x] P3.2 `dbus/wire.rs`: `Value` enum, signature parser, marshal and unmarshal with alignment rules, both endiannesses in, size limits. Tests against captured fixtures.
- [x] P3.3 `dbus/msg.rs`: header build and parse, method call, method return, error, signal.
- [x] P3.4 Main loop (reader thread + PickDone events, as in section 9). Handle Introspectable, Properties and Peer.
- [x] P3.5 FileChooser methods: options -> protocol request, spawn the picker, response -> results.
- [x] P3.6 Request objects and Close.
- [x] P3.7 `scripts/install-dev.sh` works end to end.
- [x] P3.8 Errors to stderr (the journal); clean shutdown when the bus connection drops.

Checked 2026-10-11 on XFCE: open file, open folder and save all work through the real frontend after `scripts/install-dev.sh`; `scripts/test-portal.sh` covers direct calls and Close. Firefox not tried yet (Phase 6, P6.2).

**Done when:**
- A direct `gdbus call` to our bus name opens the picker and returns URIs (section 12).
- `GTK_USE_PORTAL=1 GDK_DEBUG=portals zenity --file-selection` shows itamiiru.
- Firefox open and save dialogs work.
- Cancelling in the app (Close) closes itamiiru.

### Phase 4: Windows-style layout and places

P4.1-P4.4 (the navigation pane) were done early, after P7.4, at the developer's request. Places are rescanned every 5 s while focused; drive usage is measured per drive on its own thread, and a drive whose check hangs is not asked again.

- [x] P4.1 Linux mountinfo parser with escape decoding and tests (use a fixture file).
- [x] P4.2 Drive model and worker: labels, usage via statvfs (thread per mount), categories (system, removable, network).
- [x] P4.3 User dirs and GTK bookmarks parsers, with tests.
- [x] P4.4 Navigation pane: Quick access, Bookmarks, This PC, Network. Collapsible, resizable.
- [x] P4.5 "This PC" page with drive tiles and usage bars. Also shows the Quick access folders and network locations as tiles. Reached from the pane's "This PC" title, by typing "This PC" in the address bar, or with Up from `/`; it's a Back/Forward history entry.
- [x] P4.6 Breadcrumb address bar (section 6). Starts with a "This PC" crumb; crumbs that don't fit fold into a "..." menu on the left. Explorer's per-crumb subfolder dropdowns are not done.
- [ ] P4.6b Crumb subfolder dropdowns behind the `breadcrumb_dropdowns` setting (section 6 Optional features). Own module (e.g. `crumb_menu.rs`); lists the folder on a worker when the arrow is clicked (generation id, like listings), shows folders only, hidden ones only with hidden files on. Needs the config file (P2.10) for the setting.
- [x] P4.7 Toolbar: New folder (works in picker mode too), view selector, hidden toggle. Also Cut, Copy, Paste, Rename, Delete and a Sort menu, like the Windows 11 command bar (`toolbar.rs`). The view selector lists all five views; only Details is enabled until P4.8. The hidden toggle moved here from the status bar.
- [x] P4.8 List and icon views (thumbnails come in Phase 5; use large painted icons for now). `iconview.rs`: List flows in 260 pt columns scrolling sideways; icon views are a grid of 48/96/192 pt painted icons with two-line names. Both draw only what's on screen. Arrow keys follow the layout (a row in the grid, a column in List).
- [x] P4.9 Search: recursive name match from the current folder on a worker, streamed results, cancelled via `AtomicBool`, entry and depth caps, shown as a results list with a Folder column. Breadth-first, case-insensitive substring; 10,000 results / 64 levels; symlinked folders not followed; /proc, /sys, /dev, /run skipped. Only folders and matches are stat'ed (`/usr`, 132k entries: about 90 ms warm). Search box right of the address bar, Ctrl+F / F3, starts 0.3 s after typing, Escape clears. Results support all file commands; F5 re-runs the search.
- [x] P4.10 Persist layout settings. `settings.rs` maps view, sort, hidden toggle, zoom, sidebar width, Details column widths, window size and maximized state to config keys. Loaded at startup; saved on a worker 1 s after the last change, and once more on exit.

**Done when:** placed side by side with a screenshot of the Windows dialog, the structure and behaviour clearly match.

### Phase 5: Thumbnails
- [ ] P5.1 **Spike:** compare `zune-jpeg` and `jpeg-decoder` on downscaled decoding and speed with real photos. Record the result in D12.
- [ ] P5.2 Magic-byte sniffing and the decoder dispatch with limits (section 7 steps 3-5, 7).
- [ ] P5.3 `exif.rs`: orientation and embedded thumbnail, with tests on sample files.
- [ ] P5.4 `resize.rs` (box filter) with tests.
- [ ] P5.5 `md5.rs` with RFC 1321 test vectors.
- [ ] P5.6 Cache read/write per the spec, including atomic writes, 0600 permissions and the fail directory.
- [ ] P5.7 Thumbnail pool: LIFO stack, dedup, cleared on navigation, `catch_unwind`.
- [ ] P5.8 UI integration: icon views request visible items only; texture LRU.
- [ ] P5.9 Preview pane.
- [ ] P5.10 Malformed-file corpus and test (section 7).

**Done when:**
- A folder of 2,000 photos scrolls smoothly, with on-screen thumbnails filling in first.
- A second visit is instant, served from the cache.
- Thumbnails written by Nautilus or Dolphin are reused.
- The corpus test passes.

### Phase 6: Rollout on your own machine
- [ ] P6.1 Session environment file installed; activation environment verified (section 13).
- [ ] P6.2 Test each app in section 10 and record results in `docs/app-compat.md`.
- [ ] P6.3 Tiling-WM float rule documented if needed.

**Done when:** every app you use daily shows itamiiru, or is listed as impossible with the reason.

### Phase 7: Explorer mode

P7.1-P7.4 were done early, after Phase 1, at the developer's request.

- [x] P7.1 Open files with the default app (`xdg-open`, `open`; Windows uses `explorer <file>` until `ShellExecuteW` in Phase 8).
- [x] P7.2 File-operation worker: copy and move with progress, a conflict prompt (skip, replace, keep both, apply to all), cancellation.
- [x] P7.3 Rename (F2, inline), New folder, and Delete to Trash (freedesktop trash spec, with tests in a temp directory); Shift+Delete with confirmation. Trash is Linux-only for now; elsewhere Delete asks and deletes permanently.
- [x] P7.4 In-app cut/copy/paste, plus right-click menus for items and for empty space (section 3 D17).
- [ ] P7.5 `org.freedesktop.FileManager1` (ShowFolders, ShowItems, ShowItemProperties) in itamiiru-portal, launching `itamiiru` with a selection argument.
- [ ] P7.6 `itamiiru.desktop` file and `inode/directory` default (`xdg-mime default itamiiru.desktop inode/directory`).

**Done when:** basic file management works well enough to use daily, and "Show in folder" in Firefox opens itamiiru.

### Phase 8: Windows and macOS
- [ ] P8.1 Windows platform module (section 8) with hand-declared FFI. Includes the local time offset (`GetTimeZoneInformation`); until then Windows shows dates in UTC.
- [ ] P8.2 Windows fonts and the cache location.
- [ ] P8.3 macOS platform module (section 8); decide on the trash approach.
- [ ] P8.4 Test on real machines (`cargo check` alone isn't enough).

**Done when:** explorer mode works on both.

### Phase 9: Polish
- [ ] P9.1 Attach the picker to the parent window: X11 transient-for; Wayland xdg-foreign. Research what winit exposes and how COSMIC solved it.
- [ ] P9.2 Accessibility decision (D10).
- [ ] P9.3 Recent files (`~/.local/share/recently-used.xbel`, with a minimal parser for the `href` attribute only).
- [ ] P9.4 inotify live updates on Linux via libc (replacing polling, D9).
- [ ] P9.5 Startup time: under 150 ms to first frame on your machine. Measure and fix.
- [ ] P9.6 Follow the system light/dark theme.
- [ ] P9.7 Packaging (install script -> AUR/deb/rpm later).

## 12. Testing recipes

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

# Watch which backend the frontend picks (stop the running one first; the path varies by distro:
# /usr/libexec/ or /usr/lib/)
systemctl --user stop xdg-desktop-portal
/usr/libexec/xdg-desktop-portal -r -v

# Raw D-Bus traffic
dbus-monitor --session "interface='org.freedesktop.impl.portal.FileChooser'"

# Portal backend logs
journalctl --user -f | grep -i itamiiru
```

## 13. Gotchas

- **D-Bus activation environment.** A D-Bus-activated portal inherits the environment of the user's systemd/dbus manager, not your shell. If `WAYLAND_DISPLAY`/`DISPLAY` aren't there, the picker can't open a window. Fix it with `dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP` in the session startup (sway/Hyprland users often need this).
- **portals.conf** refers to the backend by its `.portal` filename stem (`itamiiru`). Restart the frontend after any change: `systemctl --user restart xdg-desktop-portal`.
- **`current_folder` and `current_file` are `ay`**, NUL-terminated raw bytes, not strings.
- **`pkill -x` matches only the first 15 characters** of a process name, so use `pkill -f xdg-desktop-portal-itamiiru`.
- **Tiling WMs** will tile the picker. Users need a float rule for app_id `itamiiru`, e.g. sway: `for_window [app_id="itamiiru"] floating enable`. The Hyprland rule syntax changes between versions, so check its docs.
- **`statvfs` on a dead network mount blocks**, so do it on a worker thread per mount.
- **`QT_QPA_PLATFORMTHEME`** is also how qt5ct/qt6ct work, so setting it to `xdgdesktopportal` replaces those.
- **Non-UTF-8 filenames** exist in the wild. Test with `touch $'bad\xffname'`.
- **egui's default fonts** have no CJK or emoji (section 6 Fonts).

## 14. Dependency budget

| Date | After | `cargo tree` unique crates | Notes |
|---|---|---|---|
| 2026-10-11 | P1.6 | 174 | + egui_extras (no default features), libc |
| 2026-10-11 | P0.4 baseline | 168 | eframe 0.36.2 (glow, wayland, x11, default_fonts). Release `itami` binary: 13 MB unstripped |

## 15. Open questions

- macOS trash: shell out to `osascript`, or use objc FFI?
- Accessibility (D10): when, and at what dependency cost?
- Should itamiiru write to `recently-used.xbel` when a file is picked? GTK apps do this themselves; others don't.
- Parent-window attachment on Wayland through winit (P9.1).
