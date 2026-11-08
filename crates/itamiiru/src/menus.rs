//! Right-click menus, drawn from the configured entries
//! (`itamiiru_core::menu`). Built-in commands get their labels, shortcut
//! hints and enabled state here; the menu returns what was picked, and the
//! caller (the file list, the side pane, This PC) runs it on what was
//! right-clicked.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui;
use itamiiru_core::actions::{self, Action};
use itamiiru_core::menu::{BuiltIn, Entry, MenuConfig, MenuKind, Pick};
use itamiiru_core::places::{Places, Volume};
use itamiiru_core::sort::{SortColumn, SortOrder};

/// What the file list runs: from a menu, a key or the toolbar.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Command {
    Open,
    Cut,
    Copy,
    Paste,
    CopyPath,
    Rename,
    /// Move to the trash (Delete key).
    Trash,
    /// Delete without the trash, after asking (Shift+Delete).
    DeletePermanently,
    NewFolder,
    SelectAll,
    Refresh,
    ToggleHidden,
    TogglePreview,
    /// Open the selected folder in a new tab (also middle-click).
    OpenInNewTab,
    ToggleSplit,
    /// Split view: copy or move the selection to the other pane's folder.
    CopyToOtherPane,
    MoveToOtherPane,
    /// The settings page (Ctrl+Comma).
    OpenSettings,
    /// A custom action on the selected items, by its index in
    /// `MenuConfig::actions`.
    ActionOnSelection(usize),
    /// A custom action on the folder on screen (right-click on empty space).
    ActionOnFolder(usize),
    /// Sort by a column and direction (the "Sort by" menu).
    SetSort(SortOrder),
    /// Toggle bookmark for the selected folder or current folder.
    ToggleBookmark,
    /// Copy the path of the folder on screen.
    CopyFolderPath,
}

/// What was picked in a menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuPick {
    BuiltIn(BuiltIn),
    /// An index into `MenuConfig::actions`.
    Action(usize),
    SetSort(SortOrder),
}

/// What a side pane or This PC menu picked, with what it's about.
#[derive(Clone, Debug)]
pub enum PlaceCommand {
    Open(PathBuf),
    OpenInNewTab(PathBuf),
    TogglePin(PathBuf),
    CopyPath(PathBuf),
    Mount(Volume),
    Unmount(Volume),
    Eject(Volume),
    /// A custom action (an index into `MenuConfig::actions`) on a folder.
    RunAction(usize, PathBuf),
}

/// What the side pane's and This PC's menus need.
pub struct PlaceMenus<'a> {
    pub config: &'a MenuConfig,
    pub picker: bool,
    pub places: &'a Places,
    /// Volumes being mounted or ejected right now.
    pub busy: &'a HashSet<String>,
}

/// What a menu needs to know to decide which entries show, which are
/// greyed out, and what some of them say. Callers set what applies to
/// their menu and leave the rest at `new`'s values.
pub struct MenuState<'a> {
    pub config: &'a MenuConfig,
    /// The file dialog: its own menus (`[picker_menu]`).
    pub picker: bool,
    /// The item menu's subject: the selection, as (path, is a folder).
    pub selection: Vec<(PathBuf, bool)>,
    /// The other menus' subject: the folder on screen, the bookmark, the
    /// place or the drive. Empty for a drive that isn't mounted: actions
    /// are then greyed out.
    pub folder: Vec<(PathBuf, bool)>,
    /// Tabs exist (not in the picker), for "Open in new tab".
    pub tabs: bool,
    /// Item menu: how many items are selected, and whether they're one
    /// folder.
    pub selected: usize,
    pub selected_folder: bool,
    pub can_paste: bool,
    pub show_hidden: bool,
    pub preview_pane: bool,
    pub split: bool,
    pub trash_supported: bool,
    /// Split view: the other pane's folder name, for "Copy to" and "Move
    /// to". None hides them (no split, or the other pane shows This PC).
    pub other_pane: Option<String>,
    /// Both panes show the same folder: the items are greyed out.
    pub other_pane_same: bool,
    /// How the folder is sorted now, for the "Sort by" menu.
    pub sort: SortOrder,
    /// The folder `pin` is about is a bookmark.
    pub is_bookmarked: bool,
    /// Drive menu, removable drives: (mounted, busy mounting or ejecting).
    pub volume: Option<(bool, bool)>,
}

/// One menu entry as it will be drawn.
#[derive(Clone, Debug, PartialEq)]
enum Shown {
    Button {
        label: String,
        shortcut: &'static str,
        hover: &'static str,
        enabled: bool,
        pick: MenuPick,
    },
    Separator,
    Submenu {
        label: String,
        items: Vec<Shown>,
    },
    SortBy,
}

impl<'a> MenuState<'a> {
    pub fn new(config: &'a MenuConfig, picker: bool) -> MenuState<'a> {
        return MenuState {
            config,
            picker,
            selection: Vec::new(),
            folder: Vec::new(),
            tabs: !picker,
            selected: 0,
            selected_folder: false,
            can_paste: false,
            show_hidden: false,
            preview_pane: false,
            split: false,
            trash_supported: false,
            other_pane: None,
            other_pane_same: false,
            sort: SortOrder {
                column: SortColumn::Name,
                ascending: true,
            },
            is_bookmarked: false,
            volume: None,
        };
    }
}

/// The menu for right-clicking a file or folder (which is then selected).
pub fn item_menu(ui: &mut egui::Ui, state: &MenuState) -> Option<Command> {
    let pick = show(ui, MenuKind::Item, state)?;
    return file_list_command(pick, MenuKind::Item);
}

/// The menu for right-clicking empty space in the folder.
pub fn background_menu(ui: &mut egui::Ui, state: &MenuState) -> Option<Command> {
    let pick = show(ui, MenuKind::Background, state)?;
    return file_list_command(pick, MenuKind::Background);
}

/// Draws menu `kind` into `ui` (inside a context menu). Returns what was
/// picked; the menu closes then.
pub fn show(ui: &mut egui::Ui, kind: MenuKind, state: &MenuState) -> Option<MenuPick> {
    let shown = visible(kind, state.config.menu(kind, state.picker), state);
    if shown.is_empty() {
        ui.close();
        return None;
    }
    return draw(ui, &shown, state.sort);
}

/// Gives `response` (a side pane row or a This PC tile) the right-click
/// menu `kind`. `path` is the folder, bookmark, place or mount point;
/// None for a drive that isn't mounted. `volume` is set for removable
/// drives.
pub fn place_menu(
    response: &egui::Response,
    kind: MenuKind,
    path: Option<&Path>,
    volume: Option<&Volume>,
    menus: &PlaceMenus,
) -> Option<PlaceCommand> {
    let mut command = None;
    response.context_menu(|ui| {
        let mut state = MenuState::new(menus.config, menus.picker);
        if let Some(path) = path {
            state.folder = folder_subject(path);
            state.is_bookmarked = menus.places.is_bookmarked(path);
        }
        if let Some(volume) = volume {
            state.volume = Some((path.is_some(), menus.busy.contains(&volume.id)));
        }
        let Some(pick) = show(ui, kind, &state) else {
            return;
        };
        command = place_command(pick, path, volume);
    });
    return command;
}

fn place_command(
    pick: MenuPick,
    path: Option<&Path>,
    volume: Option<&Volume>,
) -> Option<PlaceCommand> {
    let builtin = match pick {
        MenuPick::BuiltIn(builtin) => builtin,
        MenuPick::Action(index) => {
            return Some(PlaceCommand::RunAction(index, path?.to_path_buf()));
        }
        MenuPick::SetSort(_) => return None,
    };
    let command = match builtin {
        BuiltIn::Mount => PlaceCommand::Mount(volume?.clone()),
        BuiltIn::Unmount => PlaceCommand::Unmount(volume?.clone()),
        BuiltIn::Eject => PlaceCommand::Eject(volume?.clone()),
        BuiltIn::Open => PlaceCommand::Open(path?.to_path_buf()),
        BuiltIn::OpenInNewTab => PlaceCommand::OpenInNewTab(path?.to_path_buf()),
        BuiltIn::Pin => PlaceCommand::TogglePin(path?.to_path_buf()),
        BuiltIn::CopyPath => PlaceCommand::CopyPath(path?.to_path_buf()),
        _ => return None,
    };
    return Some(command);
}

/// What a pick in the file list's menus runs there.
fn file_list_command(pick: MenuPick, kind: MenuKind) -> Option<Command> {
    let builtin = match pick {
        MenuPick::BuiltIn(builtin) => builtin,
        MenuPick::Action(index) if kind == MenuKind::Item => {
            return Some(Command::ActionOnSelection(index));
        }
        MenuPick::Action(index) => return Some(Command::ActionOnFolder(index)),
        MenuPick::SetSort(sort) => return Some(Command::SetSort(sort)),
    };
    let command = match builtin {
        BuiltIn::Open => Command::Open,
        BuiltIn::OpenInNewTab => Command::OpenInNewTab,
        BuiltIn::Pin => Command::ToggleBookmark,
        BuiltIn::CopyPath if kind == MenuKind::Background => Command::CopyFolderPath,
        BuiltIn::CopyPath => Command::CopyPath,
        BuiltIn::Cut => Command::Cut,
        BuiltIn::Copy => Command::Copy,
        BuiltIn::Paste => Command::Paste,
        BuiltIn::CopyToOtherPane => Command::CopyToOtherPane,
        BuiltIn::MoveToOtherPane => Command::MoveToOtherPane,
        BuiltIn::Rename => Command::Rename,
        BuiltIn::Trash => Command::Trash,
        BuiltIn::Delete => Command::DeletePermanently,
        BuiltIn::NewFolder => Command::NewFolder,
        BuiltIn::SelectAll => Command::SelectAll,
        BuiltIn::Refresh => Command::Refresh,
        BuiltIn::ToggleHidden => Command::ToggleHidden,
        BuiltIn::TogglePreview => Command::TogglePreview,
        BuiltIn::ToggleSplit => Command::ToggleSplit,
        BuiltIn::Settings => Command::OpenSettings,
        // Drawn as the submenu itself; not in the file list's other menus.
        BuiltIn::SortBy | BuiltIn::Mount | BuiltIn::Unmount | BuiltIn::Eject => return None,
    };
    return Some(command);
}

/// The entries that show, with their labels and state. Submenus with
/// nothing to show are dropped, and separators only go between items.
fn visible(kind: MenuKind, entries: &[Entry], state: &MenuState) -> Vec<Shown> {
    let mut shown = Vec::new();
    let mut subject = Vec::new();
    let items = if kind == MenuKind::Item {
        &state.selection
    } else {
        &state.folder
    };
    for (path, is_dir) in items {
        subject.push(actions::Item {
            path,
            is_dir: *is_dir,
        });
    }
    collect(kind, entries, state, &subject, &mut shown);
    return tidy(shown);
}

fn collect(
    kind: MenuKind,
    entries: &[Entry],
    state: &MenuState,
    subject: &[actions::Item],
    shown: &mut Vec<Shown>,
) {
    for entry in entries {
        match entry {
            Entry::Separator => shown.push(Shown::Separator),
            Entry::Submenu {
                name,
                targets,
                items,
            } => {
                if let Some(targets) = targets
                    && !subject.is_empty()
                    && !targets.match_all(subject)
                {
                    continue;
                }
                let mut inner = Vec::new();
                collect(kind, items, state, subject, &mut inner);
                let inner = tidy(inner);
                if !inner.is_empty() {
                    shown.push(Shown::Submenu {
                        label: name.clone(),
                        items: inner,
                    });
                }
            }
            Entry::Item {
                pick,
                name,
                targets,
            } => {
                // An entry's own `for` decides, else the action's.
                let action = match pick {
                    Pick::Action(index) => state.config.actions.get(*index),
                    Pick::BuiltIn(_) => None,
                };
                let shows = match (targets, action) {
                    _ if subject.is_empty() => true,
                    (Some(targets), _) => targets.match_all(subject),
                    (None, Some(action)) => action.applies_to(subject),
                    (None, None) => true,
                };
                if !shows {
                    continue;
                }
                let item = match pick {
                    Pick::BuiltIn(builtin) => {
                        builtin_shown(*builtin, kind, !subject.is_empty(), state)
                    }
                    Pick::Action(index) => action_shown(*index, action, subject),
                };
                let Some(mut item) = item else {
                    continue;
                };
                if let (Some(name), Shown::Button { label, .. }) = (name, &mut item) {
                    *label = name.clone();
                }
                shown.push(item);
            }
        }
    }
}

/// A custom action's button; greyed out when there's nothing to run it on
/// (a drive that isn't mounted).
fn action_shown(index: usize, action: Option<&Action>, subject: &[actions::Item]) -> Option<Shown> {
    let action = action?;
    return Some(Shown::Button {
        label: action.name.clone(),
        shortcut: "",
        hover: "",
        enabled: !subject.is_empty(),
        pick: MenuPick::Action(index),
    });
}

/// A built-in's button, or None when it doesn't apply here (e.g. Trash
/// without a trash, Mount on a mounted drive).
fn builtin_shown(
    builtin: BuiltIn,
    kind: MenuKind,
    has_path: bool,
    state: &MenuState,
) -> Option<Shown> {
    let in_list = kind == MenuKind::Item || kind == MenuKind::Background;
    let mut hover = "";
    let (label, shortcut, enabled): (String, &'static str, bool) = match builtin {
        BuiltIn::Open => {
            if !has_path {
                return None;
            }
            let shortcut = if kind == MenuKind::Item { "Enter" } else { "" };
            ("Open".to_string(), shortcut, true)
        }
        BuiltIn::OpenInNewTab => {
            let folder = if kind == MenuKind::Item {
                state.selected_folder
            } else {
                has_path
            };
            if !state.tabs || !folder {
                return None;
            }
            let shortcut = if kind == MenuKind::Item {
                "Middle-click"
            } else {
                ""
            };
            ("Open in new tab".to_string(), shortcut, true)
        }
        BuiltIn::Pin => {
            if (kind == MenuKind::Item && !state.selected_folder) || !has_path {
                return None;
            }
            let label = match (kind, state.is_bookmarked) {
                (MenuKind::Background, true) => "Unpin this folder from Bookmarks",
                (MenuKind::Background, false) => "Bookmark this folder",
                (_, true) => "Unpin from Bookmarks",
                (_, false) => "Pin to Bookmarks",
            };
            let shortcut = if in_list { "Ctrl+D" } else { "" };
            (label.to_string(), shortcut, true)
        }
        BuiltIn::CopyPath => {
            if !has_path {
                return None;
            }
            ("Copy path".to_string(), "", true)
        }
        BuiltIn::Cut => ("Cut".to_string(), "Ctrl+X", true),
        BuiltIn::Copy => ("Copy".to_string(), "Ctrl+C", true),
        BuiltIn::Paste => ("Paste".to_string(), "Ctrl+V", state.can_paste),
        BuiltIn::CopyToOtherPane | BuiltIn::MoveToOtherPane => {
            let name = state.other_pane.as_ref()?;
            let verb = if builtin == BuiltIn::CopyToOtherPane {
                "Copy"
            } else {
                "Move"
            };
            (format!("{verb} to \"{name}\""), "", !state.other_pane_same)
        }
        BuiltIn::Rename => ("Rename".to_string(), "F2", state.selected == 1),
        BuiltIn::Trash => {
            if !state.trash_supported {
                return None;
            }
            ("Move to trash".to_string(), "Delete", true)
        }
        BuiltIn::Delete => ("Delete permanently".to_string(), "Shift+Delete", true),
        BuiltIn::NewFolder => ("New folder".to_string(), "Ctrl+Shift+N", true),
        BuiltIn::SortBy => return Some(Shown::SortBy),
        BuiltIn::SelectAll => ("Select all".to_string(), "Ctrl+A", true),
        BuiltIn::Refresh => ("Refresh".to_string(), "F5", true),
        BuiltIn::ToggleHidden => {
            let label = if state.show_hidden {
                "Hide hidden files"
            } else {
                "Show hidden files"
            };
            (label.to_string(), "Ctrl+H", true)
        }
        BuiltIn::TogglePreview => {
            let label = if state.preview_pane {
                "Hide preview pane"
            } else {
                "Show preview pane"
            };
            (label.to_string(), "Alt+P", true)
        }
        BuiltIn::ToggleSplit => {
            if !state.tabs {
                return None;
            }
            let label = if state.split {
                "Close split view"
            } else {
                "Split view"
            };
            (label.to_string(), "", true)
        }
        BuiltIn::Settings => ("Settings".to_string(), "Ctrl+Comma", true),
        BuiltIn::Mount => {
            let (mounted, busy) = state.volume?;
            if mounted {
                return None;
            }
            ("Mount and open".to_string(), "", !busy)
        }
        BuiltIn::Unmount | BuiltIn::Eject => {
            let (mounted, busy) = state.volume?;
            if !mounted {
                return None;
            }
            if builtin == BuiltIn::Unmount {
                hover = "Stays plugged in; mount it again with a click";
                ("Unmount".to_string(), "", !busy)
            } else {
                hover = "Unmounts it and powers it off, ready to unplug";
                ("Eject".to_string(), "", !busy)
            }
        }
    };
    return Some(Shown::Button {
        label,
        shortcut,
        hover,
        enabled,
        pick: MenuPick::BuiltIn(builtin),
    });
}

/// Drops separators at either end and repeats.
fn tidy(shown: Vec<Shown>) -> Vec<Shown> {
    let mut tidy: Vec<Shown> = Vec::new();
    for item in shown {
        if item == Shown::Separator && matches!(tidy.last(), None | Some(Shown::Separator)) {
            continue;
        }
        tidy.push(item);
    }
    if tidy.last() == Some(&Shown::Separator) {
        tidy.pop();
    }
    return tidy;
}

fn draw(ui: &mut egui::Ui, shown: &[Shown], sort: SortOrder) -> Option<MenuPick> {
    let mut picked = None;
    for item in shown {
        match item {
            Shown::Separator => {
                ui.separator();
            }
            Shown::Button {
                label,
                shortcut,
                hover,
                enabled,
                pick,
            } => {
                let button = egui::Button::new(label.as_str()).shortcut_text(*shortcut);
                let mut response = ui.add_enabled(*enabled, button);
                if !hover.is_empty() {
                    response = response.on_hover_text(*hover);
                }
                if response.clicked() {
                    picked = Some(*pick);
                    ui.close();
                }
            }
            Shown::Submenu { label, items } => {
                ui.menu_button(label.as_str(), |ui| {
                    if let Some(pick) = draw(ui, items, sort) {
                        picked = Some(pick);
                    }
                });
            }
            Shown::SortBy => {
                // Every view can sort from here, including List and the
                // icon views, which have no column headers to click.
                ui.menu_button("Sort by", |ui| {
                    if let Some(chosen) = sort_menu(ui, sort) {
                        picked = Some(MenuPick::SetSort(chosen));
                    }
                });
            }
        }
    }
    return picked;
}

/// The column, then the direction, each with the current one marked.
fn sort_menu(ui: &mut egui::Ui, current: SortOrder) -> Option<SortOrder> {
    let columns = [
        (SortColumn::Name, "Name"),
        (SortColumn::Modified, "Date modified"),
        (SortColumn::Created, "Date created"),
        (SortColumn::Type, "Type"),
        (SortColumn::Size, "Size"),
    ];
    let mut chosen = current;
    for (column, label) in columns {
        if ui.radio_value(&mut chosen.column, column, label).clicked() {
            ui.close();
        }
    }
    ui.separator();
    if ui
        .radio_value(&mut chosen.ascending, true, "Ascending")
        .clicked()
    {
        ui.close();
    }
    if ui
        .radio_value(&mut chosen.ascending, false, "Descending")
        .clicked()
    {
        ui.close();
    }
    if chosen != current {
        return Some(chosen);
    }
    return None;
}

/// One menu entry. Clicking it sets `command` and closes the menu.
pub fn item(
    ui: &mut egui::Ui,
    command: &mut Option<Command>,
    label: &str,
    shortcut: &str,
    enabled: bool,
    value: Command,
) {
    let button = egui::Button::new(label).shortcut_text(shortcut);
    if ui.add_enabled(enabled, button).clicked() {
        *command = Some(value);
        ui.close();
    }
}

/// `path` as a menu subject: a folder.
pub fn folder_subject(path: &Path) -> Vec<(PathBuf, bool)> {
    return vec![(path.to_path_buf(), true)];
}

#[cfg(test)]
mod tests {
    use super::*;
    use itamiiru_core::config::Config;
    use itamiiru_core::menu;

    fn config(user: &str) -> MenuConfig {
        let menus = menu::load(
            &Config::parse(itamiiru_core::config::DEFAULTS, ""),
            &Config::parse(user, ""),
        );
        assert!(menus.problems.is_empty(), "{:?}", menus.problems);
        return menus;
    }

    /// The shown labels; submenus as `Name(...)`, separators as `---`.
    fn labels(shown: &[Shown]) -> Vec<String> {
        let mut labels = Vec::new();
        for item in shown {
            match item {
                Shown::Button { label, enabled, .. } => {
                    let grey = if *enabled { "" } else { " (off)" };
                    labels.push(format!("{label}{grey}"));
                }
                Shown::Separator => labels.push("---".to_string()),
                Shown::Submenu { label, items } => {
                    labels.push(format!("{label}({})", labels_of(items)));
                }
                Shown::SortBy => labels.push("Sort by".to_string()),
            }
        }
        return labels;
    }

    fn labels_of(shown: &[Shown]) -> String {
        return labels(shown).join(", ");
    }

    fn file(path: &str) -> (PathBuf, bool) {
        return (PathBuf::from(path), false);
    }

    #[test]
    fn the_default_item_menu_is_the_old_one() {
        let menus = config("");
        let mut state = MenuState::new(&menus, false);
        state.selection = folder_subject(Path::new("/home/me/src"));
        state.selected = 1;
        state.selected_folder = true;
        state.trash_supported = true;
        let shown = visible(MenuKind::Item, menus.menu(MenuKind::Item, false), &state);
        assert_eq!(
            labels(&shown),
            [
                "Open",
                "Open in new tab",
                "Pin to Bookmarks",
                "---",
                "Cut",
                "Copy",
                "Paste (off)",
                "Copy path",
                "---",
                "Rename",
                "Move to trash",
                "Delete permanently",
            ]
        );
        // Two files, no trash, split view, the picker.
        state.selection = vec![file("/a.txt"), file("/b.txt")];
        state.selected = 2;
        state.selected_folder = false;
        state.trash_supported = false;
        state.other_pane = Some("Docs".to_string());
        state.can_paste = true;
        let shown = visible(MenuKind::Item, menus.menu(MenuKind::Item, false), &state);
        assert_eq!(
            labels(&shown),
            [
                "Open",
                "---",
                "Cut",
                "Copy",
                "Paste",
                "Copy path",
                "Copy to \"Docs\"",
                "Move to \"Docs\"",
                "---",
                "Rename (off)",
                "Delete permanently",
            ]
        );
    }

    #[test]
    fn the_default_background_and_side_menus() {
        let menus = config("");
        let mut state = MenuState::new(&menus, false);
        state.folder = folder_subject(Path::new("/home/me"));
        state.is_bookmarked = true;
        let shown = visible(
            MenuKind::Background,
            menus.menu(MenuKind::Background, false),
            &state,
        );
        assert_eq!(
            labels(&shown),
            [
                "Paste (off)",
                "New folder",
                "Unpin this folder from Bookmarks",
                "---",
                "Sort by",
                "Select all",
                "Refresh",
                "Show hidden files",
            ]
        );
        let shown = visible(
            MenuKind::Bookmark,
            menus.menu(MenuKind::Bookmark, false),
            &state,
        );
        assert_eq!(
            labels(&shown),
            [
                "Open",
                "Open in new tab",
                "Copy path",
                "---",
                "Unpin from Bookmarks"
            ]
        );
        // A removable drive, mounted, then not mounted.
        state.volume = Some((true, false));
        let drive = menus.menu(MenuKind::Drive, false);
        let shown = visible(MenuKind::Drive, drive, &state);
        assert_eq!(
            labels(&shown),
            [
                "Open",
                "Open in new tab",
                "Copy path",
                "---",
                "Unmount",
                "Eject"
            ]
        );
        state.folder = Vec::new();
        state.volume = Some((false, true));
        assert_eq!(
            labels(&visible(MenuKind::Drive, drive, &state)),
            ["Mount and open (off)"]
        );
        // A fixed drive: no volume, nothing to unmount.
        state.folder = folder_subject(Path::new("/"));
        state.volume = None;
        assert_eq!(
            labels(&visible(MenuKind::Drive, drive, &state)),
            ["Open", "Open in new tab", "Copy path"]
        );
        // The picker has no tabs.
        let picker = MenuState {
            folder: folder_subject(Path::new("/")),
            ..MenuState::new(&menus, true)
        };
        assert_eq!(
            labels(&visible(
                MenuKind::Place,
                menus.menu(MenuKind::Place, true),
                &picker
            )),
            ["Open", "Copy path"]
        );
    }

    #[test]
    fn user_menus_with_submenus_and_actions() {
        let menus = config(
            r#"
[action.extract]
name = "Extract here"
command = "7z x %f"
for = ["*.zip"]

[action.terminal]
name = "Terminal"
command = "xterm"
for = ["folders"]

[menu]
item = [
  "---",
  { name = "7-Zip", items = [
      "extract",
      { name = "Deeper", items = ["extract", "---"] },
  ]},
  { name = "Folders only", for = ["folders"], items = ["copy"] },
  { name = "Empty", items = ["terminal"] },
  "---", "---",
  { id = "cut", name = "Snip" },
  { id = "terminal", for = ["*.zip"] },
  "---",
]
drive = ["terminal", "mount"]
"#,
        );
        let mut state = MenuState::new(&menus, false);
        state.selection = vec![file("/x/a.zip")];
        state.selected = 1;
        let shown = visible(MenuKind::Item, menus.menu(MenuKind::Item, false), &state);
        assert_eq!(
            labels(&shown),
            [
                "7-Zip(Extract here, Deeper(Extract here))",
                "---",
                "Snip",
                "Terminal",
            ]
        );
        // Not a zip: the 7-Zip submenu has nothing left, so it goes.
        state.selection = vec![file("/x/a.txt")];
        let shown = visible(MenuKind::Item, menus.menu(MenuKind::Item, false), &state);
        assert_eq!(labels(&shown), ["Snip"]);
        // An unmounted drive: the action shows, greyed out.
        state.folder = Vec::new();
        state.volume = Some((false, false));
        let shown = visible(MenuKind::Drive, menus.menu(MenuKind::Drive, false), &state);
        assert_eq!(labels(&shown), ["Terminal (off)", "Mount and open"]);
        // A pick in the file list's menus becomes its command.
        assert_eq!(
            file_list_command(MenuPick::Action(1), MenuKind::Background),
            Some(Command::ActionOnFolder(1))
        );
        assert_eq!(
            file_list_command(MenuPick::BuiltIn(BuiltIn::CopyPath), MenuKind::Background),
            Some(Command::CopyFolderPath)
        );
    }

    #[test]
    fn side_menu_picks() {
        let path = Path::new("/media/usb");
        assert!(matches!(
            place_command(MenuPick::Action(2), Some(path), None),
            Some(PlaceCommand::RunAction(2, p)) if p == path
        ));
        assert!(place_command(MenuPick::Action(2), None, None).is_none());
        // Every built-in that works in a side menu does something there.
        let volume = Volume {
            id: "v".to_string(),
            drive_id: String::new(),
            name: String::new(),
            device: PathBuf::new(),
            size: 0,
            fs_type: String::new(),
            mount_point: None,
            can_power_off: true,
        };
        for builtin in BuiltIn::ALL {
            let side = [MenuKind::Bookmark, MenuKind::Place, MenuKind::Drive];
            if side.iter().any(|kind| builtin.works_in(*kind)) {
                let pick = MenuPick::BuiltIn(builtin);
                assert!(
                    place_command(pick, Some(path), Some(&volume)).is_some(),
                    "{builtin:?}"
                );
            }
        }
    }

    #[test]
    fn every_builtin_has_a_place_to_run() {
        // Each built-in that works in the file list's menus maps to a
        // command there (Sort by is drawn as its submenu instead).
        for builtin in BuiltIn::ALL {
            for kind in [MenuKind::Item, MenuKind::Background] {
                if builtin.works_in(kind) && builtin != BuiltIn::SortBy {
                    assert!(
                        file_list_command(MenuPick::BuiltIn(builtin), kind).is_some(),
                        "{builtin:?} in {kind:?}"
                    );
                }
            }
        }
    }
}
