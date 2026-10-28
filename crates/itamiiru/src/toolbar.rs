//! The command row under the address bar: New folder, the clipboard
//! and delete commands, View and the hidden-files toggle, like the
//! Windows 11 Explorer command bar.

use eframe::egui::{self, Sense, StrokeKind, TextStyle};

use crate::icons::{Arrow, paint_arrow};
use crate::menus::Command;
use crate::theme;
use itamiiru_core::sort::{SortColumn, SortOrder};

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
    pub sort: SortOrder,
    pub show_hidden: bool,
    pub preview_pane: bool,
    /// Split view on or off; None hides the switch (the picker has no tabs).
    pub split: Option<bool>,
}

/// Widths of the toolbar's items as last drawn, so the next frame can
/// tell whether everything fits before laying it out. An item folded into
/// a menu keeps the width it had when it was last drawn.
#[derive(Clone, Default, PartialEq)]
struct Widths {
    commands: [f32; COMMAND_COUNT],
    separator: f32,
    /// Hidden files, Preview pane, Split view.
    switches: [f32; 3],
    /// The "..." button holding the commands that don't fit.
    more: f32,
}

const COMMAND_COUNT: usize = 6;

/// How the toolbar fits its width. With `compact` the switches move into
/// the View menu; the commands from `shown` on go into the "..." menu.
#[derive(Debug, PartialEq)]
struct Fit {
    compact: bool,
    shown: usize,
}

pub fn show(ui: &mut egui::Ui, state: &ToolbarState) -> Option<ToolbarAction> {
    let mut action = None;
    let in_folder = !state.this_pc;
    let has_selection = in_folder && state.selected > 0;
    let commands = [
        ("New folder", "Ctrl+Shift+N", in_folder, Command::NewFolder),
        ("Cut", "Ctrl+X", has_selection, Command::Cut),
        ("Copy", "Ctrl+C", has_selection, Command::Copy),
        (
            "Paste",
            "Ctrl+V",
            in_folder && state.can_paste,
            Command::Paste,
        ),
        (
            "Rename",
            "F2",
            in_folder && state.selected == 1,
            Command::Rename,
        ),
        ("Delete", "Delete", has_selection, Command::Trash),
    ];

    let widths_id = ui.id().with("toolbar-widths");
    let old_widths: Widths = ui.data(|d| d.get_temp(widths_id)).unwrap_or_default();
    let mut widths = old_widths.clone();

    ui.horizontal(|ui| {
        let spacing = ui.spacing().item_spacing.x;
        let view_label = format!("View: {}", state.view.label());
        let sort_label = sort_label(state.sort);
        let switch_count = if state.split.is_some() { 3 } else { 2 };
        let right_full =
            dropdown_width(ui, &view_label) + spacing + dropdown_width(ui, &sort_label);
        let right_short = dropdown_width(ui, "View") + spacing + dropdown_width(ui, "Sort");
        let fit = fit(
            &old_widths,
            switch_count,
            right_full,
            right_short,
            spacing,
            ui.available_width(),
        );

        for (i, (label, shortcut, enabled, value)) in commands.iter().enumerate().take(fit.shown) {
            if i == 1 {
                widths.separator = ui.separator().rect.width();
            }
            let response = ui.add_enabled(*enabled, egui::Button::new(*label));
            widths.commands[i] = response.rect.width();
            if response.on_hover_text(*shortcut).clicked() {
                action = Some(ToolbarAction::Command(*value));
            }
        }
        if fit.shown < COMMAND_COUNT {
            let more = ui.menu_button("...", |ui| {
                let mut command = None;
                for (label, shortcut, enabled, value) in &commands[fit.shown..] {
                    crate::menus::item(ui, &mut command, label, shortcut, *enabled, *value);
                }
                if let Some(command) = command {
                    action = Some(ToolbarAction::Command(command));
                }
            });
            widths.more = more.response.rect.width();
        }

        // Laid out from the right edge, so added in reverse order.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if fit.compact {
                dropdown(ui, "View", |ui| {
                    view_choices(ui, state.view, &mut action);
                    ui.separator();
                    switch(ui, state, 0, &mut action);
                    switch(ui, state, 1, &mut action);
                    switch(ui, state, 2, &mut action);
                });
                dropdown(ui, "Sort", |ui| {
                    sort_choices(ui, state.sort, &mut action);
                });
                return;
            }
            widths.switches[2] = switch(ui, state, 2, &mut action);
            widths.switches[1] = switch(ui, state, 1, &mut action);
            widths.switches[0] = switch(ui, state, 0, &mut action);
            dropdown(ui, &view_label, |ui| {
                view_choices(ui, state.view, &mut action);
            });
            dropdown(ui, &sort_label, |ui| {
                sort_choices(ui, state.sort, &mut action);
            });
        });
    });

    if widths != old_widths {
        ui.data_mut(|d| d.insert_temp(widths_id, widths));
        // The layout was chosen with the old widths; redo it.
        ui.ctx().request_repaint();
    }
    return action;
}

/// Decides what to fold, given the items' widths. The switches fold
/// first, then the commands from the right.
fn fit(
    widths: &Widths,
    switch_count: usize,
    view_full: f32,
    view_short: f32,
    spacing: f32,
    available: f32,
) -> Fit {
    let mut full_right = view_full;
    for width in &widths.switches[..switch_count] {
        full_right += spacing + width;
    }
    if left_width(widths, COMMAND_COUNT, spacing) + theme::GAP + full_right <= available {
        return Fit {
            compact: false,
            shown: COMMAND_COUNT,
        };
    }
    let mut shown = COMMAND_COUNT;
    while shown > 0 && left_width(widths, shown, spacing) + theme::GAP + view_short > available {
        shown -= 1;
    }
    return Fit {
        compact: true,
        shown,
    };
}

/// Width of the first `shown` commands, plus the "..." button if any
/// are left out.
fn left_width(widths: &Widths, shown: usize, spacing: f32) -> f32 {
    let mut total = 0.0;
    for width in &widths.commands[..shown] {
        total += width + spacing;
    }
    // The separator after New folder.
    if shown >= 2 {
        total += widths.separator + spacing;
    }
    if shown < COMMAND_COUNT {
        total += widths.more + spacing;
    }
    return total;
}

/// One of the on/off switches: 0 is Hidden files, 1 Preview pane, 2 Split
/// view (left out in the picker). Returns its width.
fn switch(
    ui: &mut egui::Ui,
    state: &ToolbarState,
    which: usize,
    action: &mut Option<ToolbarAction>,
) -> f32 {
    let (mut value, label, tip, command) = match which {
        0 => (
            state.show_hidden,
            "Hidden files",
            "Ctrl+H",
            Command::ToggleHidden,
        ),
        1 => (
            state.preview_pane,
            "Preview pane",
            "Alt+P",
            Command::TogglePreview,
        ),
        _ => match state.split {
            Some(split) => (
                split,
                "Split view",
                "Two folders side by side (F6 switches)",
                Command::ToggleSplit,
            ),
            None => return 0.0,
        },
    };
    let response = ui.checkbox(&mut value, label);
    let width = response.rect.width();
    if response.on_hover_text(tip).changed() {
        *action = Some(ToolbarAction::Command(command));
    }
    return width;
}

fn view_choices(ui: &mut egui::Ui, current: ViewMode, action: &mut Option<ToolbarAction>) {
    for mode in ViewMode::ALL {
        if ui.selectable_label(mode == current, mode.label()).clicked() {
            if mode != current {
                *action = Some(ToolbarAction::SetView(mode));
            }
            ui.close();
        }
    }
}

fn sort_label(sort: SortOrder) -> String {
    let col = match sort.column {
        SortColumn::Name => "Name",
        SortColumn::Modified => "Date",
        SortColumn::Created => "Created",
        SortColumn::Type => "Type",
        SortColumn::Size => "Size",
    };
    let arrow = if sort.ascending { "^" } else { "v" };
    return format!("Sort: {col} {arrow}");
}

fn sort_choices(ui: &mut egui::Ui, current: SortOrder, action: &mut Option<ToolbarAction>) {
    let columns = [
        (SortColumn::Name, "Name"),
        (SortColumn::Modified, "Date modified"),
        (SortColumn::Created, "Date created"),
        (SortColumn::Type, "Type"),
        (SortColumn::Size, "Size"),
    ];
    for (col, label) in columns {
        let selected = current.column == col;
        if ui.selectable_label(selected, label).clicked() {
            let ascending = if selected { !current.ascending } else { true };
            *action = Some(ToolbarAction::Command(Command::SetSort(SortOrder {
                column: col,
                ascending,
            })));
            ui.close();
        }
    }
    ui.separator();
    if ui
        .selectable_label(current.ascending, "Ascending")
        .clicked()
    {
        if !current.ascending {
            *action = Some(ToolbarAction::Command(Command::SetSort(SortOrder {
                column: current.column,
                ascending: true,
            })));
        }
        ui.close();
    }
    if ui
        .selectable_label(!current.ascending, "Descending")
        .clicked()
    {
        if current.ascending {
            *action = Some(ToolbarAction::Command(Command::SetSort(SortOrder {
                column: current.column,
                ascending: false,
            })));
        }
        ui.close();
    }
}

/// The width `dropdown` gives a button showing `text`.
fn dropdown_width(ui: &egui::Ui, text: &str) -> f32 {
    let padding = ui.spacing().button_padding;
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, ui.visuals().text_color());
    return padding.x + galley.size().x + padding.x + DROPDOWN_ARROW + padding.x;
}

const DROPDOWN_ARROW: f32 = 8.0;

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
    let arrow_size = DROPDOWN_ARROW;
    let height = (galley.size().y + 2.0 * padding.y).max(ui.spacing().interact_size.y);
    let width = dropdown_width(ui, text);
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

    // At least as wide as the button, wider when the items need it (the
    // short "View" button's menu holds the switches too).
    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(rect.width());
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        add_contents(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding() {
        let widths = Widths {
            commands: [80.0, 40.0, 40.0, 40.0, 50.0, 50.0],
            separator: 6.0,
            switches: [90.0, 90.0, 80.0],
            more: 24.0,
        };
        // Commands: 300 + 6 * 8 = 348, separator 14. Switches with split:
        // 260 + 3 * 8 = 284, plus the View button and the gap.
        let fit_in = |available| fit(&widths, 3, 100.0, 50.0, 8.0, available);
        let all = Fit {
            compact: false,
            shown: COMMAND_COUNT,
        };
        assert_eq!(fit_in(362.0 + 8.0 + 284.0 + 100.0), all);
        let compact = Fit {
            compact: true,
            shown: COMMAND_COUNT,
        };
        assert_eq!(fit_in(362.0 + 8.0 + 284.0 + 99.0), compact);
        assert_eq!(fit_in(362.0 + 8.0 + 50.0), compact);
        // Delete goes into "...": 362 - 58 + 32 = 336.
        let one_folded = Fit {
            compact: true,
            shown: 5,
        };
        assert_eq!(fit_in(336.0 + 8.0 + 50.0), one_folded);
        assert_eq!(fit_in(0.0).shown, 0);
        // Without Split view the full row needs 88 less.
        assert_eq!(
            fit(&widths, 2, 100.0, 50.0, 8.0, 362.0 + 8.0 + 196.0 + 100.0),
            all
        );
    }

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
