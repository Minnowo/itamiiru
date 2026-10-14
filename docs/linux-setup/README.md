# Linux setup

How to install itamiiru and make it the file dialog for every app through
xdg-desktop-portal.

## Install

You install three things: the two programs, a menu entry, and two small
files that register itamiiru as a file dialog backend. Everything goes in
your home folder except one file that needs `sudo`.

Get the programs one of two ways:

- **A release:** download a tarball from the releases page and unpack it.
  There is an `opengl` and a `vulkan` build; take `opengl` if unsure. The
  programs are in `bin/`. They need glibc 2.31 or newer (Debian 11, Ubuntu
  20.04, Fedora 32 and later).
- **From source** (Rust from https://rustup.rs), in the repository root:
  `cargo build --release`. Add `--no-default-features --features wgpu` for
  the Vulkan build. The programs are in `target/release/`.

Run the commands below from the unpacked tarball or the repository root.
They use `target/release/`; from a tarball, use `bin/` instead.

**1. The programs.** Put both in the same folder: the backend starts `itami`
from next to itself.

```
mkdir -p ~/.local/bin
install -m755 target/release/itami target/release/xdg-desktop-portal-itamiiru ~/.local/bin/
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

| Distro         | Command                                                        |
|----------------|----------------------------------------------------------------|
| Arch           | `sudo pacman -S xdg-desktop-portal xdg-desktop-portal-gtk`     |
| Debian, Ubuntu | `sudo apt install xdg-desktop-portal xdg-desktop-portal-gtk`   |
| Fedora         | `sudo dnf install xdg-desktop-portal xdg-desktop-portal-gtk`   |

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

Change `default=` to the backend from step 1: `gtk`, `gnome` or `kde`.

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

**Test it:** `zenity --file-selection` should open itamiiru.
`docs/app-compat.md` lists the apps that were tested.

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

## Update

Get the new programs, repeat step 1 of Install, then restart the backend so the next
file dialog uses the new programs:

```
pkill -f xdg-desktop-portal-itamiiru
```

From source, `scripts/install-dev.sh` does all three in one go (see
`scripts/README.md`).

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
Your settings in `~/.config/itamiiru/` are kept.
