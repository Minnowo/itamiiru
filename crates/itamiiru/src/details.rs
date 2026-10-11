//! The Details view: a table with Name, Date modified, Type and Size.

use std::collections::HashSet;
use std::path::PathBuf;

use eframe::egui::{self, Align, Key, Layout, RichText, Sense, TextStyle};
use egui_extras::{Column, TableBuilder};
use itamiiru_core::entry::Entry;
use itamiiru_core::format::{format_local_time, format_size};
use itamiiru_core::sort::{SortColumn, SortOrder, type_label};

use crate::icons::{Arrow, paint_arrow, paint_entry_icon};
use crate::menus::{self, Command, MenuState};
use crate::selection::Selection;
use crate::theme::{self, Theme};

/// Something the user did in the table. The app applies it after drawing.
/// Row numbers are positions in `visible`.
pub enum TableAction {
    Sort(SortColumn),
    Click {
        row: usize,
        ctrl: bool,
        shift: bool,
    },
    /// Double-click.
    Open(usize),
    /// Right-click on a row: select it first if it isn't already.
    RightClick(usize),
    /// A right-click menu item was chosen.
    Command(Command),
    /// The inline rename box closed; `commit` is false for Escape.
    RenameDone {
        commit: bool,
    },
}

/// A name being edited in place (F2).
pub struct Rename {
    pub path: PathBuf,
    pub text: String,
    /// The box still needs focus and the name's stem selected.
    pub needs_focus: bool,
}

/// Everything the table reads.
pub struct TableInput<'a> {
    pub entries: &'a [Entry],
    /// Indices into `entries` of the rows to draw, top to bottom.
    pub visible: &'a [usize],
    pub selection: &'a Selection,
    pub sort: SortOrder,
    pub theme: &'a Theme,
    /// Cut files, drawn faded until they're pasted.
    pub cut: &'a HashSet<PathBuf>,
    pub menu: &'a MenuState,
}

/// Draws the table. `scroll_to` scrolls that row into view, e.g. after an
/// arrow key. `rename` is the name being edited, if any.
pub fn show(
    ui: &mut egui::Ui,
    input: &TableInput,
    rename: &mut Option<Rename>,
    scroll_to: Option<usize>,
) -> Option<TableAction> {
    let mut action = None;
    let line_height = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let row_height = line_height + theme::GAP_SMALL;
    // `command` is Ctrl on Linux and Windows, Cmd on macOS.
    let modifiers = ui.input(|i| i.modifiers);
    let sort = input.sort;

    let mut table = TableBuilder::new(ui);
    if let Some(row) = scroll_to {
        table = table.scroll_to_row(row, None);
    }
    table
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        // Shrink to the rows, so the empty space below can take clicks.
        .auto_shrink([false, true])
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
                    let button = egui::Button::new(RichText::new(title).strong()).frame(false);
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
            body.rows(row_height, input.visible.len(), |mut row| {
                let row_number = row.index();
                let entry = &input.entries[input.visible[row_number]];
                row.set_selected(input.selection.is_selected(&entry.path));

                row.col(|ui| {
                    let icon_size = egui::vec2(theme::ICON_SIZE, theme::ICON_SIZE);
                    let (icon_rect, _) = ui.allocate_exact_size(icon_size, Sense::hover());
                    let mut painter = ui.painter().clone();
                    let faded = entry.hidden || input.cut.contains(&entry.path);
                    if faded {
                        // Hidden and cut files are drawn faded, as in Explorer.
                        painter.multiply_opacity(0.5);
                    }
                    paint_entry_icon(&painter, icon_rect, entry, input.theme);

                    if let Some(editing) = rename
                        && editing.path == entry.path
                    {
                        if let Some(commit) = rename_box(ui, editing) {
                            action = Some(TableAction::RenameDone { commit });
                        }
                        return;
                    }
                    let mut name = RichText::new(entry.name.to_string_lossy());
                    if faded {
                        name = name.color(ui.visuals().weak_text_color());
                    }
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
                } else if response.secondary_clicked() {
                    action = Some(TableAction::RightClick(row_number));
                }
                response.context_menu(|ui| {
                    if let Some(command) = menus::item_menu(ui, input.menu) {
                        action = Some(TableAction::Command(command));
                    }
                });
            });
        });

    return action;
}

/// The inline rename text box. Returns Some(commit) when it closes:
/// Enter or clicking elsewhere commits, Escape cancels.
fn rename_box(ui: &mut egui::Ui, editing: &mut Rename) -> Option<bool> {
    let id = ui.id().with("rename");
    let edit = egui::TextEdit::singleline(&mut editing.text)
        .id(id)
        .desired_width(f32::INFINITY);
    let response = ui.add(edit);
    if editing.needs_focus {
        editing.needs_focus = false;
        response.request_focus();
        select_stem(ui.ctx(), id, &editing.text);
    }
    if response.lost_focus() {
        let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
        return Some(!cancelled);
    }
    return None;
}

/// Selects the name without its extension, like Explorer: "report" in "report.txt".
fn select_stem(ctx: &egui::Context, id: egui::Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let stem_chars = match text.rfind('.') {
        // A leading dot (".bashrc") isn't an extension.
        Some(dot) if dot > 0 => text[..dot].chars().count(),
        _ => text.chars().count(),
    };
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let range = CCursorRange::two(CCursor::new(0), CCursor::new(stem_chars));
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// One line of text that gets cut off with "..." and doesn't eat clicks
/// meant for the row.
fn cell_text(ui: &mut egui::Ui, text: RichText) {
    ui.add(egui::Label::new(text).truncate().selectable(false));
}
