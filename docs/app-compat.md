# App compatibility

Which apps show itamiiru as their file dialog, and what they need
(PLAN.md section 10, task P6.2). Tested on Debian 13, XFCE on X11, with
`~/.xsessionrc` setting `GTK_USE_PORTAL=1` and `portals.conf` choosing
itamiiru for FileChooser.

Check each one with: Open, Save (does the suggested name appear, does the
file land where you picked?), and a folder picker where the app has one.
Running `journalctl --user -f` alongside shows the portal's requests when
`ITAMI_DEBUG=1` is set in its environment.

| App | Version | Toolkit | Needs | Open | Save | Folder | Notes |
|---|---|---|---|---|---|---|---|
| zenity | | GTK | `GTK_USE_PORTAL=1` | yes | yes | yes | Tested in P3.7 |
| Firefox | 153.4.0esr | GTK + own portal code | `widget.use-xdg-desktop-portal.file-picker = 1` in about:config | | | | `firefox` and `firefox-esr` are the same program here; it has two profiles, so set it in the one you use (or both) |
| VSCodium | 1.135 | Electron (Chromium) | maybe nothing; else `GTK_USE_PORTAL=1` | | | | |
| LibreOffice | 25.2.3.2 | GTK VCL plugin | `GTK_USE_PORTAL=1` | | | | LibreOffice's own dialogs must be off: Tools > Options > General > "Use LibreOffice dialogs" unchecked |
| Mousepad | 0.6.3 | GTK 3 | `GTK_USE_PORTAL=1` | | | | |
| Ristretto | | GTK 3 | `GTK_USE_PORTAL=1` | | | | |
| Atril | 1.26.2 | GTK 3 | `GTK_USE_PORTAL=1` | | | | |

## Not possible

| App | Why |
|---|---|
| JetBrains IDEs, Java/Swing apps | Draw their own dialogs |
| Tk apps | Draw their own dialogs |
| Wine apps | Use the Windows dialog inside Wine |

## Qt apps

None installed that open files. Qt needs `QT_QPA_PLATFORMTHEME=xdgdesktopportal`
and the matching platform theme plugin (Debian: `qt6-xdgdesktopportal-platformtheme`
for Qt 6; Qt 5's comes with `libqt5gui5`); it replaces qt5ct/qt6ct.
