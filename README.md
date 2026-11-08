# itamiiru

A file picker and file explorer with a similar feel to the Windows 10 file
dialog.

- [x] xdg-desktop-portal backend (can be used as Save As, Open File, Choose Folder dialogs, etc)
- [x] tabs and split view
- [x] filter filenames in the current folder
- [x] search folder and sub folders
- [x] add custom right-click menu actions
- [x] image thumbnails (jpg, png, gif, bmp, webp)
- [x] sends items to trash (optional)
- [x] detect, mount and eject USB drives

## Install

Linux only for now. `docs/linux-setup/README.md` covers building, installing
and setting itamiiru up as the file dialog for every app through
xdg-desktop-portal.

## Using it

- **Command line:** `itami [FOLDER] [--select ITEM]`. Started from a
  terminal it moves to the background; `--foreground` keeps it attached.
  `itami --help` lists the options.
- **Settings:** the gear at the bottom of the side pane, or Ctrl+Comma.
  Two files in `~/.config/itamiiru/` hold them:
  - `config.toml` is yours: your defaults, colors and right-click menus.
    itami only reads it, at startup; problems in it are listed on the
    settings page.
  - `live.toml` is itami's: what you change in the app (zoom, window size,
    the settings page). It wins over `config.toml`; delete a line from it
    to go back to your `config.toml` value.

  `data/default.toml` (built into itami) lists every setting with its
  default.
- **Right-click menus** are yours to arrange: the menus for files and
  folders, empty space, bookmarks, places and drives are lists in
  `config.toml`'s `[menu]` table. Add, remove and reorder entries, nest
  submenus as deep as you like, and add your own commands:
  ```toml
  [action.extract_here]
  name = "Extract here"
  command = "7z x %f -o%d"
  for = ["*.zip", "*.7z"]

  [menu]
  item = [
    "open", "open_in_new_tab", "pin",
    { name = "7-Zip", for = ["*.zip", "*.7z"], items = ["extract_here"] },
    "---",
    "cut", "copy", "paste", "copy_path",
    "---",
    "rename", "trash", "delete",
  ]
  ```
  A menu you set replaces the default one; the others stay as they are.
  `data/default.toml` has the default menus, every built-in command, and
  the placeholders (`%f`, `%F`, `%d`, `%n`) and `for` targets.
- **Colors:** a `[colors]` table in `config.toml` changes theme colors by
  role name, e.g. `primary = "#dcc496"`.
- **USB drives** show up under This PC even before they're mounted: click
  to mount, right-click to unmount or eject (needs `udisks2`).
- **Drag files out** onto other apps (a browser upload box, a terminal, an
  editor), and **drop files in** from other apps: they're copied into the
  folder under the pointer. X11 sessions only for now; Escape cancels.
- **Drag and drop inside the window**: onto a folder, a folder name in the
  address bar, a place or drive in the side pane, or the other pane; hold
  it over a tab to open that tab. Within a drive files move, onto another
  drive they're copied; hold Ctrl to copy or Shift to move.
