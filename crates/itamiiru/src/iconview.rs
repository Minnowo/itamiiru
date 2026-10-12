//! The List and icon views. Both only lay out and draw the items on
//! screen, so big folders cost no more than small ones. Clicks, menus and
//! rename work the same as in the Details view.

use std::ops::Range;

use eframe::egui::text::LayoutJob;
use eframe::egui::{self, Align, Align2, Rect, Sense, TextStyle};
use itamiiru_core::entry::Entry;
use itamiiru_core::thumbcache::ThumbSize;

use crate::details::{Rename, TableAction, TableInput, item_action, rename_box};
use crate::icons::paint_entry_icon;
use crate::menus;
use crate::theme::{self, Theme};
use crate::thumbs::Thumbs;

/// Width of a column in the List view.
const LIST_COLUMN_WIDTH: f32 = 260.0;

/// How the arrow keys move in the current layout: Up/Down by `vertical`
/// items, Left/Right by `horizontal` (0 means Left/Right do nothing).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct KeySteps {
    pub vertical: isize,
    pub horizontal: isize,
}

/// Where the icon views find thumbnails, and which size to show.
#[derive(Clone, Copy)]
pub struct ThumbSource<'a> {
    pub thumbs: &'a Thumbs,
    pub size: ThumbSize,
}

/// Icons in a grid with names underneath. `icon` is the icon size in points.
/// Also returns the range of `input.visible` that was on screen, so the
/// app can ask for those thumbnails.
pub fn show_icons(
    ui: &mut egui::Ui,
    input: &TableInput,
    rename: &mut Option<Rename>,
    scroll_to: Option<usize>,
    icon: f32,
    thumbs: Option<ThumbSource>,
) -> (Option<TableAction>, KeySteps, Range<usize>) {
    let mut action = None;
    let mut on_screen = 0..0;
    let modifiers = ui.input(|i| i.modifiers);
    let line = ui.text_style_height(&TextStyle::Body);
    let cell = egui::vec2(
        (icon * 1.5).max(96.0),
        theme::GAP + icon + theme::GAP_SMALL + 2.0 * line + theme::GAP,
    );
    let spacing = theme::GAP;
    let count = input.visible.len();
    let mut per_row = 1;

    let scroll = egui::ScrollArea::vertical()
        .id_salt("icon-view")
        .auto_shrink([false, false]);
    scroll.show_viewport(ui, |ui, viewport| {
        let width = ui.available_width();
        per_row = grid_columns(width, cell.x, spacing);
        let rows = count.div_ceil(per_row);
        let content_height = rows as f32 * (cell.y + spacing);
        let origin = ui.max_rect().min;
        ui.set_min_size(egui::vec2(width, content_height.max(viewport.height())));

        let content = Rect::from_min_size(
            origin,
            egui::vec2(width, content_height.max(viewport.height())),
        );
        background(ui, content, input, &mut action);

        let cell_rect = |index: usize| {
            let (row, col) = (index / per_row, index % per_row);
            let min = origin
                + egui::vec2(
                    col as f32 * (cell.x + spacing),
                    row as f32 * (cell.y + spacing),
                );
            return Rect::from_min_size(min, cell);
        };
        let (first_row, end_row) =
            visible_range(viewport.min.y, viewport.max.y, cell.y + spacing, rows);
        on_screen = first_row * per_row..(end_row * per_row).min(count);
        for index in on_screen.clone() {
            let entry = &input.entries[input.visible[index]];
            let thumbnail = match thumbs {
                Some(source) => source.thumbs.get(entry, source.size),
                None => None,
            };
            let response = icon_cell(
                ui,
                cell_rect(index),
                index,
                entry,
                input,
                rename,
                icon,
                thumbnail,
                &mut action,
            );
            item_action(&response, index, modifiers, input.menu, &mut action);
        }
        if let Some(index) = scroll_to
            && index < count
        {
            ui.scroll_to_rect(cell_rect(index), None);
        }
    });

    let steps = KeySteps {
        vertical: per_row as isize,
        horizontal: 1,
    };
    return (action, steps, on_screen);
}

/// Small icons and names in columns that flow top to bottom, then left to
/// right, scrolling sideways.
pub fn show_list(
    ui: &mut egui::Ui,
    input: &TableInput,
    rename: &mut Option<Rename>,
    scroll_to: Option<usize>,
) -> (Option<TableAction>, KeySteps) {
    let mut action = None;
    let modifiers = ui.input(|i| i.modifiers);
    let row_height =
        ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE) + theme::GAP_SMALL;
    let count = input.visible.len();
    let mut per_column = 1;

    let scroll = egui::ScrollArea::horizontal()
        .id_salt("list-view")
        .auto_shrink([false, false]);
    scroll.show_viewport(ui, |ui, viewport| {
        let height = viewport.height();
        per_column = ((height / row_height).floor() as usize).max(1);
        let columns = count.div_ceil(per_column);
        let content_width = columns as f32 * LIST_COLUMN_WIDTH;
        let origin = ui.max_rect().min;
        ui.set_min_size(egui::vec2(content_width.max(viewport.width()), height));

        let content = Rect::from_min_size(
            origin,
            egui::vec2(content_width.max(viewport.width()), height),
        );
        background(ui, content, input, &mut action);

        let cell_rect = |index: usize| {
            let (col, row) = (index / per_column, index % per_column);
            let min = origin + egui::vec2(col as f32 * LIST_COLUMN_WIDTH, row as f32 * row_height);
            return Rect::from_min_size(
                min,
                egui::vec2(LIST_COLUMN_WIDTH - theme::GAP, row_height),
            );
        };
        let (first_col, end_col) =
            visible_range(viewport.min.x, viewport.max.x, LIST_COLUMN_WIDTH, columns);
        for index in first_col * per_column..(end_col * per_column).min(count) {
            let entry = &input.entries[input.visible[index]];
            let response = list_cell(
                ui,
                cell_rect(index),
                index,
                entry,
                input,
                rename,
                &mut action,
            );
            item_action(&response, index, modifiers, input.menu, &mut action);
        }
        if let Some(index) = scroll_to
            && index < count
        {
            ui.scroll_to_rect(cell_rect(index), None);
        }
    });

    let steps = KeySteps {
        vertical: 1,
        horizontal: per_column as isize,
    };
    return (action, steps);
}

/// How many cells of `cell_width` fit across `width`, at least one.
pub fn grid_columns(width: f32, cell_width: f32, spacing: f32) -> usize {
    let fit = ((width + spacing) / (cell_width + spacing)).floor();
    if fit < 1.0 {
        return 1;
    }
    return fit as usize;
}

/// The rows (or columns) of size `step` that overlap `start..end`, as
/// `first..end_exclusive`, clamped to `total`.
pub fn visible_range(start: f32, end: f32, step: f32, total: usize) -> (usize, usize) {
    if step <= 0.0 || total == 0 {
        return (0, 0);
    }
    let first = (start.max(0.0) / step).floor() as usize;
    let last = (end.max(0.0) / step).ceil() as usize;
    return (first.min(total), last.min(total));
}

/// Empty space: click clears the selection, right-click shows the folder menu.
/// Added before the items, so the items (added later, on top) win clicks.
fn background(ui: &mut egui::Ui, rect: Rect, input: &TableInput, action: &mut Option<TableAction>) {
    let response = ui.interact(rect, ui.id().with("background"), Sense::click());
    if response.clicked() || response.secondary_clicked() {
        *action = Some(TableAction::ClearSelection);
    }
    response.context_menu(|ui| {
        if let Some(command) = menus::background_menu(ui, input.menu) {
            *action = Some(TableAction::Command(command));
        }
    });
}

/// Paints the hover or selected background of an item.
fn item_background(
    ui: &egui::Ui,
    rect: Rect,
    response: &egui::Response,
    selected: bool,
    theme: &Theme,
) {
    let fill = if selected {
        Some(theme.secondary_container)
    } else if response.hovered() {
        Some(theme.on_surface_hover_tint)
    } else {
        None
    };
    if let Some(fill) = fill {
        ui.painter().rect_filled(rect, theme::RADIUS_SMALL, fill);
    }
}

#[allow(clippy::too_many_arguments)]
fn icon_cell(
    ui: &mut egui::Ui,
    rect: Rect,
    index: usize,
    entry: &Entry,
    input: &TableInput,
    rename: &mut Option<Rename>,
    icon: f32,
    thumbnail: Option<&egui::TextureHandle>,
    action: &mut Option<TableAction>,
) -> egui::Response {
    let theme = input.theme;
    let response = ui.interact(rect, ui.id().with(("item", index)), Sense::click_and_drag());
    let selected = input.selection.is_selected(&entry.path);
    item_background(ui, rect, &response, selected, theme);

    let faded = entry.hidden || input.cut.contains(&entry.path);
    let mut painter = ui.painter().clone();
    if faded {
        painter.multiply_opacity(0.5);
    }
    let icon_center = egui::pos2(rect.center().x, rect.top() + theme::GAP + icon / 2.0);
    let icon_rect = Rect::from_center_size(icon_center, egui::vec2(icon, icon));
    match thumbnail {
        Some(texture) => paint_thumbnail(&painter, icon_rect, texture, ui.ctx().pixels_per_point()),
        None => paint_entry_icon(&painter, icon_rect, entry, theme),
    }

    let name_rect = Rect::from_min_max(
        egui::pos2(
            rect.left() + theme::GAP_SMALL,
            icon_rect.bottom() + theme::GAP_SMALL,
        ),
        egui::pos2(
            rect.right() - theme::GAP_SMALL,
            rect.bottom() - theme::GAP_SMALL,
        ),
    );
    if let Some(editing) = rename
        && editing.path == entry.path
    {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(name_rect)
                .id_salt(("rename", index)),
        );
        if let Some(commit) = rename_box(&mut child, editing) {
            *action = Some(TableAction::RenameDone { commit });
        }
        return response;
    }

    // The name, centered, wrapped to at most two lines.
    let color = text_color(ui, theme, selected, faded);
    let font = TextStyle::Body.resolve(ui.style());
    let mut job = LayoutJob::simple(
        entry.name.to_string_lossy().into_owned(),
        font,
        color,
        name_rect.width(),
    );
    job.wrap.max_rows = 2;
    job.wrap.break_anywhere = true;
    job.halign = Align::Center;
    let galley = painter.layout_job(job);
    painter.galley(
        egui::pos2(name_rect.center().x, name_rect.top()),
        galley,
        color,
    );
    return response.on_hover_text(entry.name.to_string_lossy());
}

/// Draws a thumbnail centered in the icon's square, keeping its shape.
/// Small images aren't blown up past their own size.
fn paint_thumbnail(
    painter: &egui::Painter,
    icon_rect: Rect,
    texture: &egui::TextureHandle,
    pixels_per_point: f32,
) {
    let [w, h] = texture.size();
    let natural = egui::vec2(w as f32, h as f32) / pixels_per_point;
    let scale = (icon_rect.width() / natural.x)
        .min(icon_rect.height() / natural.y)
        .min(1.0);
    let rect = Rect::from_center_size(icon_rect.center(), natural * scale);
    let uv = Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0));
    painter.image(texture.id(), rect, uv, egui::Color32::WHITE);
}

fn list_cell(
    ui: &mut egui::Ui,
    rect: Rect,
    index: usize,
    entry: &Entry,
    input: &TableInput,
    rename: &mut Option<Rename>,
    action: &mut Option<TableAction>,
) -> egui::Response {
    let theme = input.theme;
    let response = ui.interact(rect, ui.id().with(("item", index)), Sense::click_and_drag());
    let selected = input.selection.is_selected(&entry.path);
    item_background(ui, rect, &response, selected, theme);

    let faded = entry.hidden || input.cut.contains(&entry.path);
    let mut painter = ui.painter().clone();
    if faded {
        painter.multiply_opacity(0.5);
    }
    let icon_rect = Rect::from_min_size(
        egui::pos2(
            rect.left() + theme::GAP_SMALL,
            rect.center().y - theme::ICON_SIZE / 2.0,
        ),
        egui::vec2(theme::ICON_SIZE, theme::ICON_SIZE),
    );
    paint_entry_icon(&painter, icon_rect, entry, theme);

    let name_rect = Rect::from_min_max(
        egui::pos2(icon_rect.right() + theme::GAP, rect.top()),
        egui::pos2(rect.right() - theme::GAP_SMALL, rect.bottom()),
    );
    if let Some(editing) = rename
        && editing.path == entry.path
    {
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(name_rect)
                .id_salt(("rename", index)),
        );
        if let Some(commit) = rename_box(&mut child, editing) {
            *action = Some(TableAction::RenameDone { commit });
        }
        return response;
    }

    let color = text_color(ui, theme, selected, faded);
    let font = TextStyle::Body.resolve(ui.style());
    let clipped = painter.with_clip_rect(name_rect);
    let name_pos = egui::pos2(name_rect.left(), name_rect.center().y);
    clipped.text(
        name_pos,
        Align2::LEFT_CENTER,
        entry.name.to_string_lossy(),
        font,
        color,
    );
    return response;
}

fn text_color(ui: &egui::Ui, theme: &Theme, selected: bool, faded: bool) -> egui::Color32 {
    if selected {
        return theme.on_secondary_container;
    }
    if faded {
        return ui.visuals().weak_text_color();
    }
    return theme.on_surface;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_that_fit() {
        // Three cells and two gaps take 304 px; a fourth would need 408.
        assert_eq!(grid_columns(400.0, 96.0, 8.0), 3);
        assert_eq!(grid_columns(408.0, 96.0, 8.0), 4);
        assert_eq!(grid_columns(10.0, 96.0, 8.0), 1);
        assert_eq!(grid_columns(1000.0, 96.0, 8.0), 9);
    }

    #[test]
    fn visible_ranges() {
        assert_eq!(visible_range(0.0, 100.0, 50.0, 10), (0, 2));
        assert_eq!(visible_range(75.0, 160.0, 50.0, 10), (1, 4));
        assert_eq!(visible_range(0.0, 10_000.0, 50.0, 10), (0, 10));
        assert_eq!(visible_range(-20.0, 30.0, 50.0, 10), (0, 1));
        assert_eq!(visible_range(0.0, 100.0, 50.0, 0), (0, 0));
    }
}
