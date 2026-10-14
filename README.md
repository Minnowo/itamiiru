# itamiiru

A file picker and file explorer with a similar feel to the Windows 10/11 file
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
  They're saved in `~/.config/itamiiru/config`.
- **Custom right-click actions:** add menu items like "Open in terminal" in
  `~/.config/itamiiru/actions`:
  ```
  [Open in terminal]
  command = xfce4-terminal --working-directory=%d
  for = folders, background
  ```
  `data/actions.example` explains the placeholders (`%f`, `%F`, `%d`, `%n`)
  and the `for` targets. The file is read at startup.
- **USB drives** show up under This PC even before they're mounted: click
  to mount, right-click to unmount or eject (needs `udisks2`).
- **Drag files out** onto other apps (a browser upload box, a terminal, an
  editor), and **drop files in** from other apps: they're copied into the
  folder under the pointer. X11 sessions only for now; Escape cancels.
- **Drag and drop inside the window**: onto a folder, a folder name in the
  address bar, a place or drive in the side pane, or the other pane; hold
  it over a tab to open that tab. Within a drive files move, onto another
  drive they're copied; hold Ctrl to copy or Shift to move.
