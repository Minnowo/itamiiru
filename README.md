# itamiiru

A file picker and file explorer laid out like the Windows 10/11 file
dialog, written in Rust with egui.

- **One file dialog for every Linux app.** itamiiru is a backend for
  xdg-desktop-portal's FileChooser, so GTK, Firefox, Chromium, Electron and
  Flatpak apps all open the same dialog.
- **A file explorer** with the same UI: Details, List and icon views with
  thumbnails, a preview pane, search, cut/copy/paste, trash, rename.

The command is `itami` (`itami [FOLDER]` opens the explorer). Linux only
for now; Windows and macOS are planned.

## Requirements

- Linux on x86_64 or aarch64, with X11 or Wayland and OpenGL (any normal
  desktop has these).
- `xdg-desktop-portal` (the frontend) and your desktop's usual backend,
  e.g. `xdg-desktop-portal-gtk`:
  - Debian/Ubuntu: `sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk`
  - Fedora: `sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk`
  - Arch: `sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk`
- Prebuilt binaries need glibc as new as the machine they were built on
  (`package.sh` prints the version; check yours with `ldd --version`).
  Built on Debian 13 that's glibc 2.39: Debian 13, Ubuntu 24.04, Fedora 40
  or newer. On older systems, build from source.
- Building from source: Rust from https://rustup.rs and a C toolchain
  (`build-essential` on Debian/Ubuntu).

## Install

The script installs for your user only (`~/.local/bin`) and asks for sudo
once, for the one file that must live in `/usr/share`. Run it as your
normal user, not with sudo.

**From a release tarball** (no Rust needed). On a machine with Rust, make
the tarball:

```
scripts/package.sh        # -> target/itamiiru-<version>-<arch>-linux.tar.gz
```

Copy it over, then:

```
tar xf itamiiru-*-linux.tar.gz
cd itamiiru-*-linux
./install.sh
```

**From source:**

```
git clone <this repo> itamiiru && cd itamiiru
scripts/install.sh
```

Then:

1. **Log out and back in**, so apps pick up `GTK_USE_PORTAL=1`.
2. **Firefox:** `about:config`, set
   `widget.use-xdg-desktop-portal.file-picker` to `1` (in each profile you
   use).
3. **LibreOffice:** Tools > Options > General, untick "Use LibreOffice
   dialogs".
4. Try it: `GTK_USE_PORTAL=1 zenity --file-selection`

`docs/app-compat.md` lists which apps were tested and what each needs.

Options: `--no-env` leaves the session environment alone (set the
variables yourself, see below); `--default-file-manager` makes itamiiru the
app for folders and for "Show in folder" (next section); `--uninstall`
removes everything.

## Default file manager

`scripts/install.sh --default-file-manager` also:

- makes itamiiru open folders (`xdg-mime default itamiiru.desktop
  inode/directory`), so double-clicking a folder elsewhere or `xdg-open
  ~/Downloads` opens itamiiru;
- answers `org.freedesktop.FileManager1`, which Firefox, Chromium and
  others call for "Show in folder": itamiiru opens the folder with the file
  selected.

On XFCE it does two more things:

- `xdg-open` there runs `exo-open`, which ignores the folder default above
  and uses Settings > Preferred Applications > File Manager. The script
  adds itamiiru to that list and selects it (`~/.config/xfce4/helpers.rc`).
- Only one program can own `org.freedesktop.FileManager1`, and XFCE starts
  `Thunar --daemon` at every login, which holds it. Unless you saved a
  session, XFCE starts its built-in "Failsafe" session, so changing Thunar's
  restart style in Session and Startup does nothing. The script takes
  Thunar's daemon out of your copy of that session, from the next login
  (or run `thunar -q` now; it closes Thunar's windows). Thunar opened by
  hand still takes the name while it runs.

The same applies to other file managers running as a background daemon;
the installer says when one holds the name.

`--uninstall` puts back the folder app you had chosen before (or the
system's default if you hadn't chosen one), XFCE's previous file manager,
and Thunar's daemon in the Failsafe session.

By hand: `itami [FOLDER] [--select ITEM]...` (paths or `file://` URIs);
copy `data/itamiiru.desktop` to `~/.local/share/applications/` and
`data/org.freedesktop.FileManager1.service` to
`~/.local/share/dbus-1/services/`, replacing `@BINDIR@` in both, then run
`xdg-mime default itamiiru.desktop inode/directory`.

## What the script does (to install by hand)

1. **Binaries.** Copy `itami` and `xdg-desktop-portal-itamiiru` (from
   `bin/` in the tarball, or `target/release/` after
   `cargo build --release`) into `~/.local/bin`. They must sit in the same
   folder: the backend starts the picker from next to itself (or set
   `ITAMI_BIN` to the picker's path).

2. **D-Bus activation file.** Copy
   `data/org.freedesktop.impl.portal.desktop.itamiiru.service` to
   `~/.local/share/dbus-1/services/`, replacing `@BINDIR@` with the
   folder from step 1, e.g. `/home/you/.local/bin`.

3. **Portal description.** As root, copy `data/itamiiru.portal` to
   `/usr/share/xdg-desktop-portal/portals/`. The frontend only looks there.

4. **Choose itamiiru for file dialogs.** Create
   `~/.config/xdg-desktop-portal/portals.conf`:

   ```
   [preferred]
   default=gtk
   org.freedesktop.impl.portal.FileChooser=itamiiru
   ```

   Set `default=` to the backend you use now (`gtk`, `gnome`, `kde`, `wlr`,
   ...; your desktop's `/usr/share/xdg-desktop-portal/*-portals.conf` says
   which). Only file dialogs change.

5. **Session environment**, so GTK apps use the portal at all:
   - X sessions started by `/etc/X11/Xsession` (XFCE, MATE, Cinnamon on
     Debian/Ubuntu): append `data/xsessionrc.example` to `~/.xsessionrc`.
     That file runs with `set -e`, so a failing line stops the login; the
     example guards every command.
   - Wayland and systemd-based sessions (GNOME, KDE, sway, ...): copy the
     GTK lines of `data/itamiiru-env.conf.example` to
     `~/.config/environment.d/60-itamiiru.conf`.
   - Qt apps also need `QT_QPA_PLATFORMTHEME=xdgdesktopportal` and that
     platform theme plugin; it replaces qt5ct/qt6ct.

6. **Restart the frontend:** `systemctl --user restart xdg-desktop-portal`,
   then log out and back in.

## Uninstall

```
scripts/install.sh --uninstall
```

It removes the binaries, the portal and FileManager1 files, the menu
entry, the folder default and XFCE changes (see above), its line in
`portals.conf` and its lines in `~/.xsessionrc` (or its environment.d
file). Settings in `~/.config/itamiiru/` are kept; delete that folder too
for a clean slate. Thumbnails stay in `~/.cache/thumbnails`, shared with
other file managers.

## Troubleshooting

- **The old dialog still appears:** the app didn't get `GTK_USE_PORTAL=1`
  (log out and in; check with `cat /proc/<pid>/environ | tr '\0' '\n'`),
  or the app has its own setting (Firefox, LibreOffice above).
- **Is the portal choosing itamiiru?** Watch `journalctl --user -f` while
  opening a dialog. For more detail, run the backend by hand:
  `pkill -f xdg-desktop-portal-itamiiru; ITAMI_DEBUG=1 ~/.local/bin/xdg-desktop-portal-itamiiru`
  and open a dialog from another terminal.
- **No window appears (Wayland, tiling WMs):** the D-Bus activation
  environment may lack the display variables. Add
  `dbus-update-activation-environment --systemd WAYLAND_DISPLAY DISPLAY XDG_CURRENT_DESKTOP`
  to your session startup.
- **Tiling window managers** tile the dialog; add a float rule for app id
  `itamiiru`, e.g. sway: `for_window [app_id="itamiiru"] floating enable`.

## USB sticks and other removable drives

Plugged-in drives show up under This PC (in the sidebar and on the This
PC page) even before they're mounted. Click one to mount and open it;
the Eject button next to a mounted one unmounts everything on it and
powers it off, and the status bar says when it's safe to unplug. This
goes through UDisks2 (`udisks2` package), which every Linux desktop
ships, so no password is needed for removable drives in your own session.

## Settings

The **Settings** button at the right of the toolbar (or Ctrl+Comma) opens
the settings page; changes apply at once. Behind it is
`~/.config/itamiiru/config`, `key=value` lines written as you use the app
(view, sort, zoom, window size, ...), which can also be edited by hand
while the app is closed. Optional features: `thumbnails`, `preview_pane`, `auto_refresh_folder`,
`auto_refresh_drives` (`true` or `false`). `status_fields` picks what the
status bar shows for one selected item, in order: any of `name`, `type`,
`size`, `modified`, `path`, comma-separated (default
`type,size,modified`). `details_columns` picks the Details view's columns
besides Name: any of `modified`, `type`, `size` (default all three). Colors can be changed with a
theme file at `~/.config/itamiiru/theme` (see PLAN.md section 6).

## Development

See `CLAUDE.md` for the rules and commands and `PLAN.md` for the design
and task list. `scripts/install-dev.sh` rebuilds and reinstalls from the
checkout, leaving the session environment alone.
