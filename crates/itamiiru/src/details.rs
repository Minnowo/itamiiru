//! The Details view: a table with Name, Date modified, Type and Size.

use eframe::egui::{self, Align, Layout, RichText, Sense, TextStyle};
use egui_extras::{Column, TableBuilder};
use itamiiru_core::entry::Entry;
use itamiiru_core::format::{format_local_time, format_size};
use itamiiru_core::sort::{SortColumn, SortOrder, type_label};

use crate::icons::{Arrow, paint_arrow, paint_entry_icon};
use crate::selection::Selection;
use crate::theme::{self, Theme};

/// Something the user did in the table. The app applies it after drawing.
/// Row numbers are positions in `visible`.
pub enum TableAction {
    Sort(SortColumn),
    Click { row: usize, ctrl: bool, shift: bool },
    Open(usize),
}

/// Draws the rows `visible` (indices into `entries`, top to bottom).
/// `scroll_to` scrolls that row into view, e.g. after an arrow key.
pub fn show(
    ui: &mut egui::Ui,
    entries: &[Entry],
    visible: &[usize],
    selection: &Selection,
    sort: SortOrder,
    theme: &Theme,
    scroll_to: Option<usize>,
) -> Option<TableAction> {
    let mut action = None;
    let line_height = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let row_height = line_height + theme::GAP_SMALL;
    // `command` is Ctrl on Linux and Windows, Cmd on macOS.
    let modifiers = ui.input(|i| i.modifiers);

    let mut table = TableBuilder::new(ui);
    if let Some(row) = scroll_to {
        table = table.scroll_to_row(row, None);
    }
    table
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::initial(320.0).at_least(120.0).clip(true))
        .column(Column::initial(140.0).at_least(60.0).clip(true))
        .column(Column::initial(110.0).at_least(50.0).clip(true))
        .column(Column::remainder().at_least(60.0).clip(true))
        .header(row_height + theme::GAP_SMALL, |mut header| {
            let columns = [
                ("Name", SortColumn::Name),
                ("Date modified", SortColumn::Modified),
                ("Type", SortColumn::Type),
                ("Size", SortColumn::Size),
            ];
            for (title, column) in columns {
                header.col(|ui| {
                    let button =
                        egui::Button::new(egui::RichText::new(title).strong()).frame(false);
                    if ui.add(button).clicked() {
                        action = Some(TableAction::Sort(column));
                    }
                    if column == sort.column {
                        let arrow = if sort.ascending {
                            Arrow::Up
                        } else {
                            Arrow::Down
                        };
                        let size = egui::vec2(8.0, 8.0);
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        let color = ui.visuals().weak_text_color();
                        paint_arrow(ui.painter(), rect, arrow, color);
                    }
                });
            }
        })
        .body(|body| {
            // `rows` only calls the closure for rows on screen, so 50,000
            // entries cost no more per frame than 50.
            body.rows(row_height, visible.len(), |mut row| {
                let row_number = row.index();
                let entry = &entries[visible[row_number]];
                row.set_selected(selection.is_selected(&entry.path));

                row.col(|ui| {
                    let icon_size = egui::vec2(theme::ICON_SIZE, theme::ICON_SIZE);
                    let (icon_rect, _) = ui.allocate_exact_size(icon_size, Sense::hover());
                    let mut painter = ui.painter().clone();
                    let mut name = RichText::new(entry.name.to_string_lossy());
                    if entry.hidden {
                        // Hidden files are drawn faded, as in Explorer.
                        painter.multiply_opacity(0.5);
                        name = name.color(ui.visuals().weak_text_color());
                    }
                    paint_entry_icon(&painter, icon_rect, entry, theme);
                    cell_text(ui, name);
                });
                row.col(|ui| {
                    if let Some(mtime) = entry.mtime {
                        cell_text(ui, RichText::new(format_local_time(mtime)));
                    }
                });
                row.col(|ui| cell_text(ui, RichText::new(type_label(entry))));
                row.col(|ui| {
                    if !entry.is_dir() {
                        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                            cell_text(ui, RichText::new(format_size(entry.size)));
                        });
                    }
                });

                let response = row.response();
                if response.double_clicked() {
                    action = Some(TableAction::Open(row_number));
                } else if response.clicked() {
                    action = Some(TableAction::Click {
                        row: row_number,
                        ctrl: modifiers.command,
                        shift: modifiers.shift,
                    });
                }
            });
        });

    return action;
}

/// One line of text that gets cut off with "..." and doesn't eat clicks
/// meant for the row.
fn cell_text(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).truncate().selectable(false));
}
