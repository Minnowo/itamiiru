//! The command row under the address bar (P4.7): New folder, the clipboard
//! and delete commands, View and the hidden-files toggle, like the
//! Windows 11 Explorer command bar.

use eframe::egui::{self, Sense, StrokeKind, TextStyle};

use crate::icons::{Arrow, paint_arrow};
use crate::menus::Command;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ViewMode {
    Details,
    List,
    MediumIcons,
    LargeIcons,
    ExtraLargeIcons,
}

impl ViewMode {
    pub const ALL: [ViewMode; 5] = [
        ViewMode::Details,
        ViewMode::List,
        ViewMode::MediumIcons,
        ViewMode::LargeIcons,
        ViewMode::ExtraLargeIcons,
    ];

    pub fn label(self) -> &'static str {
        return match self {
            ViewMode::Details => "Details",
            ViewMode::List => "List",
            ViewMode::MediumIcons => "Medium icons",
            ViewMode::LargeIcons => "Large icons",
            ViewMode::ExtraLargeIcons => "Extra large icons",
        };
    }

    /// The name used in the config file.
    pub fn key(self) -> &'static str {
        return match self {
            ViewMode::Details => "details",
            ViewMode::List => "list",
            ViewMode::MediumIcons => "medium_icons",
            ViewMode::LargeIcons => "large_icons",
            ViewMode::ExtraLargeIcons => "extra_large_icons",
        };
    }

    pub fn from_key(key: &str) -> Option<ViewMode> {
        for view in ViewMode::ALL {
            if view.key() == key {
                return Some(view);
            }
        }
        return None;
    }

    /// File icon size in points for the icon views (Details and List use
    /// the small list icon).
    pub fn icon_size(self) -> f32 {
        return match self {
            ViewMode::Details | ViewMode::List => crate::theme::ICON_SIZE,
            ViewMode::MediumIcons => 48.0,
            ViewMode::LargeIcons => 96.0,
            ViewMode::ExtraLargeIcons => 192.0,
        };
    }
}

pub enum ToolbarAction {
    Command(Command),
    SetView(ViewMode),
}

/// What the toolbar needs to know to enable its buttons.
pub struct ToolbarState {
    pub selected: usize,
    pub can_paste: bool,
    /// On the This PC page there are no files to act on.
    pub this_pc: bool,
    pub view: ViewMode,
    pub show_hidden: bool,
    pub preview_pane: bool,
    /// Split view on or off; None hides the switch (the picker has no tabs).
    pub split: Option<bool>,
}

pub fn show(ui: &mut egui::Ui, state: &ToolbarState) -> Option<ToolbarAction> {
    let mut action = None;
    let in_folder = !state.this_pc;
    let has_selection = in_folder && state.selected > 0;

    ui.horizontal(|ui| {
        command(
            ui,
            &mut action,
            "New folder",
            "Ctrl+Shift+N",
            in_folder,
            Command::NewFolder,
        );
        ui.separator();
        command(
            ui,
            &mut action,
            "Cut",
            "Ctrl+X",
            has_selection,
            Command::Cut,
        );
        command(
            ui,
            &mut action,
            "Copy",
            "Ctrl+C",
            has_selection,
            Command::Copy,
        );
        let can_paste = in_folder && state.can_paste;
        command(
            ui,
            &mut action,
            "Paste",
            "Ctrl+V",
            can_paste,
            Command::Paste,
        );
        let one_selected = in_folder && state.selected == 1;
        command(
            ui,
            &mut action,
            "Rename",
            "F2",
            one_selected,
            Command::Rename,
        );
        command(
            ui,
            &mut action,
            "Delete",
            "Delete",
            has_selection,
            Command::Trash,
        );

        // Laid out from the right edge, so added in reverse order.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if let Some(split) = state.split {
                let mut split = split;
                let checkbox = ui.checkbox(&mut split, "Split view");
                if checkbox
                    .on_hover_text("Two folders side by side (F6 switches)")
                    .changed()
                {
                    action = Some(ToolbarAction::Command(Command::ToggleSplit));
                }
            }
            let mut preview_pane = state.preview_pane;
            let checkbox = ui.checkbox(&mut preview_pane, "Preview pane");
            if checkbox.on_hover_text("Alt+P").changed() {
                action = Some(ToolbarAction::Command(Command::TogglePreview));
            }
            let mut show_hidden = state.show_hidden;
            let checkbox = ui.checkbox(&mut show_hidden, "Hidden files");
            if checkbox.on_hover_text("Ctrl+H").changed() {
                action = Some(ToolbarAction::Command(Command::ToggleHidden));
            }
            if let Some(view) = view_selector(ui, state.view) {
                action = Some(ToolbarAction::SetView(view));
            }
        });
    });
    return action;
}

/// One command button with its shortcut in the tooltip.
fn command(
    ui: &mut egui::Ui,
    action: &mut Option<ToolbarAction>,
    label: &str,
    shortcut: &str,
    enabled: bool,
    value: Command,
) {
    let response = ui.add_enabled(enabled, egui::Button::new(label));
    if response.on_hover_text(shortcut).clicked() {
        *action = Some(ToolbarAction::Command(value));
    }
}

fn view_selector(ui: &mut egui::Ui, current: ViewMode) -> Option<ViewMode> {
    let mut chosen = current;
    let label = format!("View: {}", current.label());
    dropdown(ui, &label, |ui| {
        for mode in ViewMode::ALL {
            if ui
                .selectable_value(&mut chosen, mode, mode.label())
                .clicked()
            {
                ui.close();
            }
        }
    });
    if chosen != current {
        return Some(chosen);
    }
    return None;
}

/// A button that looks like a drop-down field (label and a down arrow) and
/// opens a menu below it. Used instead of egui's ComboBox, whose list sits
/// in a scroll area that showed a needless scroll bar on hover.
pub fn dropdown(ui: &mut egui::Ui, text: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    let padding = ui.spacing().button_padding;
    let font = TextStyle::Button.resolve(ui.style());
    let text_color = ui.visuals().text_color();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, text_color);
    let arrow_size = 8.0;
    let height = (galley.size().y + 2.0 * padding.y).max(ui.spacing().interact_size.y);
    let width = padding.x + galley.size().x + padding.x + arrow_size + padding.x;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(width, height), Sense::click());

    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        painter.rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
        let text_pos = egui::pos2(
            rect.left() + padding.x,
            rect.center().y - galley.size().y / 2.0,
        );
        painter.galley(text_pos, galley, visuals.text_color());
        let arrow_center = egui::pos2(rect.right() - padding.x - arrow_size / 2.0, rect.center().y);
        let arrow_rect =
            egui::Rect::from_center_size(arrow_center, egui::vec2(arrow_size, arrow_size));
        paint_arrow(painter, arrow_rect, Arrow::Down, visuals.fg_stroke.color);
    }

    egui::Popup::menu(&response)
        .width(rect.width())
        .show(add_contents);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_sizes_grow() {
        let mut last = 0.0;
        for mode in ViewMode::ALL {
            assert!(mode.icon_size() >= last, "{mode:?}");
            last = mode.icon_size();
        }
        assert_eq!(ViewMode::ExtraLargeIcons.icon_size(), 192.0);
    }
}
