//! The command row under the address bar: New folder, the clipboard
//! and delete commands, View and the hidden-files toggle, like the
//! Windows 10 Explorer command bar.

use eframe::egui::{self, Pos2, Rect, Sense, StrokeKind, TextStyle};

use crate::icons::{Arrow, ButtonIcon, paint_arrow, paint_button_icon};
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
    /// Cut, Copy, Paste, Rename and Delete as icons instead of names.
    pub icons: bool,
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
        let right_full = dropdown_width(ui, Some(ButtonIcon::View), &view_label)
            + spacing
            + dropdown_width(ui, Some(ButtonIcon::Sort), &sort_label);
        let right_short = dropdown_width(ui, Some(ButtonIcon::View), "View")
            + spacing
            + dropdown_width(ui, Some(ButtonIcon::Sort), "Sort");
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
            // New folder always shows its name; an icon alone would say less.
            let as_icon = state.icons && i > 0;
            let response = if as_icon {
                let icon = match i {
                    1 => ButtonIcon::Cut,
                    2 => ButtonIcon::Copy,
                    3 => ButtonIcon::Paste,
                    4 => ButtonIcon::Rename,
                    _ => ButtonIcon::Delete,
                };
                // Disabled like any egui widget: faded and not clickable.
                ui.add_enabled_ui(*enabled, |ui| toolbar_icon_button(ui, icon))
                    .inner
            } else {
                ui.add_enabled(*enabled, egui::Button::new(*label))
            };
            widths.commands[i] = response.rect.width();
            let tip = if as_icon {
                format!("{label} ({shortcut})")
            } else {
                shortcut.to_string()
            };
            if response.on_hover_text(tip).clicked() {
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
                dropdown_with_icon(ui, Some(ButtonIcon::View), "View", |ui| {
                    view_choices(ui, state.view, &mut action);
                    ui.separator();
                    switch(ui, state, 0, &mut action);
                    switch(ui, state, 1, &mut action);
                    switch(ui, state, 2, &mut action);
                });
                dropdown_with_icon(ui, Some(ButtonIcon::Sort), "Sort", |ui| {
                    sort_choices(ui, state.sort, &mut action);
                });
                return;
            }
            widths.switches[2] = switch(ui, state, 2, &mut action);
            widths.switches[1] = switch(ui, state, 1, &mut action);
            widths.switches[0] = switch(ui, state, 0, &mut action);
            dropdown_with_icon(ui, Some(ButtonIcon::View), &view_label, |ui| {
                view_choices(ui, state.view, &mut action);
            });
            dropdown_with_icon(ui, Some(ButtonIcon::Sort), &sort_label, |ui| {
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
    if which == 1 {
        let side = ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
        let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::click());
        if ui.is_rect_visible(rect) {
            let visuals = ui
                .style()
                .interact_selectable(&response, state.preview_pane);
            let painter = ui.painter();
            painter.rect(
                rect,
                visuals.corner_radius,
                visuals.weak_bg_fill,
                visuals.bg_stroke,
                StrokeKind::Inside,
            );
            let color = visuals.fg_stroke.color;
            let area = rect.shrink(side * 0.26);
            paint_button_icon(painter, area, ButtonIcon::PreviewPane, color);
        }
        let width = response.rect.width();
        if response.on_hover_text("Preview pane (Alt+P)").clicked() {
            *action = Some(ToolbarAction::Command(Command::TogglePreview));
        }
        return width;
    }
    let (mut value, label, tip, command) = match which {
        0 => (
            state.show_hidden,
            "Hidden files",
            "Ctrl+H",
            Command::ToggleHidden,
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

/// A square button showing `icon`. In a disabled Ui (`add_enabled_ui`)
/// egui fades it and ignores clicks, as it does for text buttons.
fn toolbar_icon_button(ui: &mut egui::Ui, icon: ButtonIcon) -> egui::Response {
    let side = ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::click());
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
        let area = rect.shrink(side * 0.26);
        paint_button_icon(painter, area, icon, visuals.fg_stroke.color);
    }
    return response;
}

/// The width `dropdown` gives a button showing `text`.
fn dropdown_width(ui: &egui::Ui, icon: Option<ButtonIcon>, text: &str) -> f32 {
    let padding = ui.spacing().button_padding;
    let font = TextStyle::Button.resolve(ui.style());
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, ui.visuals().text_color());
    let icon_w = if icon.is_some() {
        theme::ICON_SIZE + theme::GAP_SMALL
    } else {
        0.0
    };
    return padding.x + icon_w + galley.size().x + padding.x + DROPDOWN_ARROW + padding.x;
}

const DROPDOWN_ARROW: f32 = 8.0;

/// A button that looks like a drop-down field (label and a down arrow) and
/// opens a menu below it. Used instead of egui's ComboBox, whose list sits
/// in a scroll area that showed a needless scroll bar on hover.
pub fn dropdown(ui: &mut egui::Ui, text: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    dropdown_with_icon(ui, None, text, add_contents);
}

pub fn dropdown_with_icon(
    ui: &mut egui::Ui,
    icon: Option<ButtonIcon>,
    text: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    let padding = ui.spacing().button_padding;
    let font = TextStyle::Button.resolve(ui.style());
    let text_color = ui.visuals().text_color();
    let galley = ui
        .painter()
        .layout_no_wrap(text.to_string(), font, text_color);
    let arrow_size = DROPDOWN_ARROW;
    let height = (galley.size().y + 2.0 * padding.y).max(ui.spacing().interact_size.y);
    let width = dropdown_width(ui, icon, text);
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
        let mut text_left = rect.left() + padding.x;
        if let Some(btn_icon) = icon {
            let icon_rect = Rect::from_center_size(
                Pos2::new(text_left + theme::ICON_SIZE / 2.0, rect.center().y),
                egui::vec2(theme::ICON_SIZE * 0.8, theme::ICON_SIZE * 0.8),
            );
            paint_button_icon(painter, icon_rect, btn_icon, visuals.fg_stroke.color);
            text_left += theme::ICON_SIZE + theme::GAP_SMALL;
        }
        let text_pos = egui::pos2(text_left, rect.center().y - galley.size().y / 2.0);
        painter.galley(text_pos, galley, visuals.text_color());
        let arrow_center = egui::pos2(rect.right() - padding.x - arrow_size / 2.0, rect.center().y);
        let arrow_rect =
            egui::Rect::from_center_size(arrow_center, egui::vec2(arrow_size, arrow_size));
        paint_arrow(painter, arrow_rect, Arrow::Down, visuals.fg_stroke.color);
    }

    egui::Popup::menu(&response).show(|ui| {
        ui.set_min_width(rect.width());
        ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Extend);
        add_contents(ui);
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{Theme, ThemeName};

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

    #[test]
    fn disabled_icon_buttons_are_faded_and_ignore_clicks() {
        for name in ThemeName::ALL {
            // Nothing selected: Cut is disabled.
            let ctx = themed_context(name);
            let (cut, _) = run(&ctx, 0, Vec::new());
            let cut = cut.expect("the Cut icon is drawn");
            assert!(cut.alpha < 200, "{name:?}: faded: {:?}", cut.color);
            let (_, action) = click(&ctx, 0, cut.center);
            assert!(action.is_none(), "{name:?}: a disabled button does nothing");

            // One file selected: Cut is enabled.
            let ctx = themed_context(name);
            let (cut, _) = run(&ctx, 1, Vec::new());
            let cut = cut.expect("the Cut icon is drawn");
            assert_eq!(cut.alpha, 255, "{name:?}: not faded: {:?}", cut.color);
            let (_, action) = click(&ctx, 1, cut.center);
            assert!(
                matches!(action, Some(ToolbarAction::Command(Command::Cut))),
                "{name:?}: an enabled button works"
            );
        }
    }

    fn themed_context(name: ThemeName) -> egui::Context {
        let ctx = egui::Context::default();
        Theme::new(name, None).install(&ctx);
        return ctx;
    }

    /// One of the Cut icon's rings (the only circles in the toolbar).
    struct Ring {
        center: egui::Pos2,
        color: egui::Color32,
        alpha: u8,
    }

    /// Draws the toolbar with icons for `selected` items. Returns a ring
    /// of the Cut icon and the toolbar's action, if any.
    fn run(
        ctx: &egui::Context,
        selected: usize,
        events: Vec<egui::Event>,
    ) -> (Option<Ring>, Option<ToolbarAction>) {
        let state = ToolbarState {
            selected,
            can_paste: false,
            this_pc: false,
            view: ViewMode::Details,
            sort: SortOrder {
                column: SortColumn::Name,
                ascending: true,
            },
            show_hidden: false,
            preview_pane: false,
            split: Some(false),
            icons: true,
        };
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1200.0, 200.0),
            )),
            events,
            ..Default::default()
        };
        let mut action = None;
        let mut output = ctx.run_ui(input, |ui| action = show(ui, &state));
        output.textures_delta.clear();
        let mut ring = None;
        for clipped in &output.shapes {
            find_ring(&clipped.shape, &mut ring);
        }
        return (ring, action);
    }

    fn find_ring(shape: &egui::Shape, out: &mut Option<Ring>) {
        match shape {
            egui::Shape::Circle(circle) => {
                let color = circle.stroke.color;
                *out = Some(Ring {
                    center: circle.center,
                    color,
                    alpha: color.a(),
                });
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    find_ring(shape, out);
                }
            }
            _ => {}
        }
    }

    /// Clicks at `pos` over three frames, as a person would.
    fn click(
        ctx: &egui::Context,
        selected: usize,
        pos: egui::Pos2,
    ) -> (Option<Ring>, Option<ToolbarAction>) {
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let mut last = run(ctx, selected, vec![egui::Event::PointerMoved(pos)]);
        for event in [press(true), press(false)] {
            let result = run(ctx, selected, vec![event]);
            if result.1.is_some() {
                return result;
            }
            last = result;
        }
        return last;
    }
}
