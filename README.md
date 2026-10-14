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

You install three things: the two programs, a menu entry, and two small
files that register itamiiru as a file dialog backend. Everything goes in
your home folder except one file that needs `sudo`.

- **Release tarball:** unpack it; the programs are in `bin/`. There's an
  `opengl` and a `vulkan` tarball.
- **From source** (Rust from https://rustup.rs):
  `cargo build --release` (add `--no-default-features --features wgpu` for
  the Vulkan build); the programs are in `target/release/`.

Run the commands below from the unpacked tarball or the source folder. They
use `bin/`; from source, use `target/release/` instead.

**1. The programs.** Put both in the same folder: the backend starts `itami`
from next to itself.

```
mkdir -p ~/.local/bin
install -m755 bin/itami bin/xdg-desktop-portal-itamiiru ~/.local/bin/
```

To run `itami` from a terminal, `~/.local/bin` must be on your `PATH` (it
is on most distros).

**2. The menu entry.** The desktop file puts itamiiru in your app menu and
lets other apps offer it for opening folders.

```
mkdir -p ~/.local/share/applications
sed "s|@BINDIR@|$HOME/.local/bin|" data/itamiiru.desktop \
  > ~/.local/share/applications/itamiiru.desktop
```

**3. The file dialog backend.** Two files tell the system that itamiiru can
show file dialogs:

```
# The D-Bus service: starts the backend when an app needs a dialog.
mkdir -p ~/.local/share/dbus-1/services
sed "s|@BINDIR@|$HOME/.local/bin|" data/org.freedesktop.impl.portal.desktop.itamiiru.service \
  > ~/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.itamiiru.service

# The .portal file: describes the backend to xdg-desktop-portal, which only
# looks for these in a system folder.
sudo install -Dm644 data/itamiiru.portal /usr/share/xdg-desktop-portal/portals/itamiiru.portal
```

## Set up file dialogs

Apps don't draw their file dialogs themselves: they ask
**xdg-desktop-portal**, which hands the request to a **backend**. These
steps tell the portal to use itamiiru for file dialogs (everything else
stays on your desktop's usual backend), and tell GTK apps to go through
the portal.

**1. Install the portal and your desktop's usual backend:**

| Distro | Command |
|---|---|
| Arch | `sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk` |
| Debian, Ubuntu | `sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk` |
| Fedora | `sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk` |

On GNOME use `xdg-desktop-portal-gnome`, on KDE `xdg-desktop-portal-kde`.

**2. Choose itamiiru for file dialogs:**

```
mkdir -p ~/.config/xdg-desktop-portal
cp data/portals.conf.example ~/.config/xdg-desktop-portal/portals.conf
```

The file reads:

```
[preferred]
default=gtk
org.freedesktop.impl.portal.FileChooser=itamiiru
```

Change `default=` to the backend from step 1: `gtk`, `gnome` or `kde`. (With
the Arch package, write the file by hand; the example isn't installed.)

**3. Make GTK apps use the portal.** GTK 3 reads `GTK_USE_PORTAL`, GTK 4
reads `GDK_DEBUG`.

- **X11 sessions** (Xfce, MATE, Cinnamon, ...): add these lines to
  `~/.xsessionrc` on Debian and Ubuntu, or to `~/.xprofile` on Arch and
  most others:

  ```
  export GTK_USE_PORTAL=1
  export GDK_DEBUG=portals
  # Apps started through D-Bus get these too.
  dbus-update-activation-environment --systemd GTK_USE_PORTAL GDK_DEBUG || true
  ```

- **Wayland sessions** (GNOME, KDE, sway, ...): create
  `~/.config/environment.d/60-itamiiru.conf` with:

  ```
  GTK_USE_PORTAL=1
  GDK_DEBUG=portals
  ```

**4. Log out and back in,** so apps see the new settings.

**5. Turn it on in apps that have their own switch:**

- **Firefox:** in `about:config`, set
  `widget.use-xdg-desktop-portal.file-picker` to `1`.
- **LibreOffice:** Tools > Options > General, untick "Use LibreOffice
  dialogs".

**Test it:** `zenity --file-selection` should open itamiiru. If the old
dialog shows up, see Troubleshooting. `docs/app-compat.md` lists the apps
that were tested.

## Use it as the default file manager (optional)

**Open folders with itamiiru** (double-clicking a folder in another app,
`xdg-open ~/Downloads`):

```
xdg-mime default itamiiru.desktop inode/directory
```

**"Show in folder"** in Firefox, Chromium and others:

```
sed "s|@BINDIR@|$HOME/.local/bin|" data/org.freedesktop.FileManager1.service \
  > ~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service
```

**On Xfce,** two more things:

- `xdg-open` follows Xfce's own setting there. Add itamiiru to it, then
  pick it in Settings > Default Applications > Utilities > File Manager:

  ```
  mkdir -p ~/.local/share/xfce4/helpers
  sed "s|@BINDIR@|$HOME/.local/bin|" data/itamiiru-xfce-helper.desktop \
    > ~/.local/share/xfce4/helpers/itamiiru.desktop
  ```

- Thunar runs in the background and answers "Show in folder" first. In
  Settings > Session and Startup > Session, set Thunar's restart style to
  **Never** and click **Save Session**; for now, run `thunar -q`.

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

## Update

Repeat step 1 of Install with the new programs, then restart the backend so
the next file dialog uses them:

```
pkill -f xdg-desktop-portal-itamiiru
```

## Uninstall

Undo the steps above:

```
rm ~/.local/bin/itami ~/.local/bin/xdg-desktop-portal-itamiiru
rm ~/.local/share/applications/itamiiru.desktop
rm ~/.local/share/dbus-1/services/org.freedesktop.impl.portal.desktop.itamiiru.service
rm -f ~/.local/share/dbus-1/services/org.freedesktop.FileManager1.service
rm -f ~/.local/share/xfce4/helpers/itamiiru.desktop
sudo rm /usr/share/xdg-desktop-portal/portals/itamiiru.portal
```

Then remove the `FileChooser=itamiiru` line from (or delete)
`~/.config/xdg-desktop-portal/portals.conf`, remove the lines you added in
step 3 of "Set up file dialogs", and pick another folder app if you made
itamiiru the default (`xdg-mime default thunar.desktop inode/directory`).
Do this before removing the Arch package too. Your settings in
`~/.config/itamiiru/` are kept.

## Troubleshooting

- **The old dialog still appears:** log out and back in; check the app sees
  `GTK_USE_PORTAL=1` with
  `cat /proc/$(pidof -s APP)/environ | tr '\0' '\n' | grep GTK_USE_PORTAL`;
  check the app's own switch (step 5).
- **Is the portal using itamiiru?** Watch `journalctl --user -f` while
  opening a dialog, or run the backend by hand to see its log:
  `pkill -f xdg-desktop-portal-itamiiru; ITAMI_DEBUG=1 xdg-desktop-portal-itamiiru`.
- **No window appears on Wayland or a tiling WM:** add
  `dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP`
  to your session startup.
- **Tiling WMs** tile the dialog: add a float rule for app id `itamiiru`,
  e.g. sway: `for_window [app_id="itamiiru"] floating enable`.
- **Characters show as boxes:** install a font for that language (Arch:
  `noto-fonts-cjk`; Debian: `fonts-noto-cjk`). `itami --font-info` shows
  which fonts itamiiru found.

## Development

`CLAUDE.md` has the rules and commands, `PLAN.md` the design and task list.
`scripts/install-dev.sh` builds the checkout and puts the programs in
`~/.local/bin` (after the one-time setup above). `scripts/package.sh` makes
the release tarballs.

