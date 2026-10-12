//! Right-click menus. They return a `Command`; the app runs it, the same
//! way it runs keyboard shortcuts.

use eframe::egui;

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
}

/// What the menus need to know to enable or label their items.
pub struct MenuState {
    pub selected: usize,
    pub can_paste: bool,
    pub show_hidden: bool,
    pub trash_supported: bool,
    /// One folder is selected and tabs are available (not in the picker).
    pub open_in_new_tab: bool,
    /// Split view: the other pane's folder name, for "Copy to" and "Move
    /// to". None hides them (no split, or the other pane shows This PC).
    pub other_pane: Option<String>,
    /// Both panes show the same folder: the items are greyed out.
    pub other_pane_same: bool,
}

/// The menu for right-clicking a file or folder (which is then selected).
pub fn item_menu(ui: &mut egui::Ui, state: &MenuState) -> Option<Command> {
    let mut command = None;
    item(ui, &mut command, "Open", "Enter", true, Command::Open);
    if state.open_in_new_tab {
        item(
            ui,
            &mut command,
            "Open in new tab",
            "Middle-click",
            true,
            Command::OpenInNewTab,
        );
    }
    ui.separator();
    item(ui, &mut command, "Cut", "Ctrl+X", true, Command::Cut);
    item(ui, &mut command, "Copy", "Ctrl+C", true, Command::Copy);
    item(
        ui,
        &mut command,
        "Paste",
        "Ctrl+V",
        state.can_paste,
        Command::Paste,
    );
    item(ui, &mut command, "Copy path", "", true, Command::CopyPath);
    if let Some(name) = &state.other_pane {
        let enabled = !state.other_pane_same;
        let copy = format!("Copy to \"{name}\"");
        item(
            ui,
            &mut command,
            &copy,
            "",
            enabled,
            Command::CopyToOtherPane,
        );
        let move_label = format!("Move to \"{name}\"");
        item(
            ui,
            &mut command,
            &move_label,
            "",
            enabled,
            Command::MoveToOtherPane,
        );
    }
    ui.separator();
    item(
        ui,
        &mut command,
        "Rename",
        "F2",
        state.selected == 1,
        Command::Rename,
    );
    if state.trash_supported {
        item(
            ui,
            &mut command,
            "Move to trash",
            "Delete",
            true,
            Command::Trash,
        );
    }
    item(
        ui,
        &mut command,
        "Delete permanently",
        "Shift+Delete",
        true,
        Command::DeletePermanently,
    );
    return command;
}

/// The menu for right-clicking empty space in the folder.
pub fn background_menu(ui: &mut egui::Ui, state: &MenuState) -> Option<Command> {
    let mut command = None;
    item(
        ui,
        &mut command,
        "Paste",
        "Ctrl+V",
        state.can_paste,
        Command::Paste,
    );
    item(
        ui,
        &mut command,
        "New folder",
        "Ctrl+Shift+N",
        true,
        Command::NewFolder,
    );
    ui.separator();
    item(
        ui,
        &mut command,
        "Select all",
        "Ctrl+A",
        true,
        Command::SelectAll,
    );
    item(ui, &mut command, "Refresh", "F5", true, Command::Refresh);
    let hidden_label = if state.show_hidden {
        "Hide hidden files"
    } else {
        "Show hidden files"
    };
    item(
        ui,
        &mut command,
        hidden_label,
        "Ctrl+H",
        true,
        Command::ToggleHidden,
    );
    return command;
}

/// One menu entry. Clicking it sets `command` and closes the menu.
fn item(
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
