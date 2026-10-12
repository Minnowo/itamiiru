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
}

/// What the menus need to know to enable or label their items.
pub struct MenuState {
    pub selected: usize,
    pub can_paste: bool,
    pub show_hidden: bool,
    pub trash_supported: bool,
}

/// The menu for right-clicking a file or folder (which is then selected).
pub fn item_menu(ui: &mut egui::Ui, state: &MenuState) -> Option<Command> {
    let mut command = None;
    item(ui, &mut command, "Open", "Enter", true, Command::Open);
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
