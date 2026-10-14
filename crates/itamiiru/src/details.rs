//! The Details view: a table with Name, Date modified, Type and Size.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align, Color32, Key, Layout, Rect, RichText, Sense, TextStyle};
use egui_extras::{Column, TableBuilder};
use itamiiru_core::entry::Entry;
use itamiiru_core::format::{format_local_time, format_size};
use itamiiru_core::sort::{SortColumn, SortOrder, type_label};

use crate::band;
use crate::icons::{Arrow, paint_arrow, paint_entry_icon};
use crate::menus::{self, Command, MenuState};
use crate::selection::Selection;
use crate::settings::DetailsColumns;
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
    /// Middle-click: a folder opens in a new tab.
    MiddleClick(usize),
    /// A drag started on an item (it's selected first if it isn't).
    DragStart(usize),
    /// A right-click menu item was chosen.
    Command(Command),
    /// The inline rename box closed; `commit` is false for Escape.
    RenameDone {
        commit: bool,
    },
    /// A click on empty space.
    ClearSelection,
    /// A drag-to-select box was started; `add` (Ctrl held) keeps the
    /// selection it started with.
    BandStart {
        add: bool,
    },
    /// The rows the drag-to-select box touches now.
    Band(Vec<usize>),
    /// Items are being dragged and the pointer is over this row.
    DropHover(usize),
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
    /// Search results: add a Folder column showing where each item is.
    pub show_folder: bool,
    /// Which of Date modified, Type and Size to show.
    pub columns: DetailsColumns,
    /// The longest Type text among the rows, to size that column.
    pub widest_type: &'a str,
    /// Dragging from empty space draws a box to select with. In the
    /// Details view, so do drags on columns other than Name.
    pub drag_select: bool,
    /// Items are being dragged inside the window.
    pub dragging: bool,
    /// Where they'd be dropped, as of the last frame: that folder's item
    /// is drawn selected.
    pub drop_target: Option<&'a Path>,
}

/// The longest texts the Date modified and Size columns can hold, for
/// sizing them: every date has the same digits-and-dashes shape, and
/// sizes below 1 KB are the longest ("1023 bytes", vs "1023 MB").
const WIDEST_DATE: &str = "0000-00-00 00:00";
const WIDEST_SIZE: &str = "1023 bytes";
/// The sort arrow drawn after a header's title.
const SORT_ARROW_SIZE: f32 = 8.0;
/// Name (and Folder, in search results) never get narrower than this.
const MIN_NAME_WIDTH: f32 = 120.0;

/// Draws the table. `scroll_to` scrolls that row into view, e.g. after an
/// arrow key. `rename` is the name being edited, if any. Name takes the
/// space the other columns leave; they are exactly as wide as their
/// longest content or header.
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
    let columns = input.columns;

    let modified_width = column_width(ui, "Date modified", WIDEST_DATE);
    let type_width = column_width(ui, "Type", input.widest_type);
    let size_width = column_width(ui, "Size", WIDEST_SIZE);

    let table_id = if input.show_folder {
        "search-table"
    } else {
        "folder-table"
    };
    let band_id = ui.id().with(table_id).with("band");
    let step = row_height + ui.spacing().item_spacing.y;
    let mut table = TableBuilder::new(ui)
        .id_salt(table_id)
        .striped(true)
        // Drag too, so items can be dragged out to other apps.
        .sense(Sense::click_and_drag())
        // Shrink to the rows, so the empty space below can take clicks.
        .auto_shrink([false, true])
        .cell_layout(Layout::left_to_right(Align::Center))
        .column(Column::remainder().at_least(MIN_NAME_WIDTH).clip(true));
    if input.show_folder {
        // Name and Folder share what's left.
        table = table.column(Column::remainder().at_least(MIN_NAME_WIDTH).clip(true));
    }
    if columns.modified {
        table = table.column(Column::exact(modified_width).clip(true));
    }
    if columns.kind {
        table = table.column(Column::exact(type_width).clip(true));
    }
    if columns.size {
        table = table.column(Column::exact(size_width).clip(true));
    }
    if let Some(row) = scroll_to {
        table = table.scroll_to_row(row, None);
    }
    // Where row 0 is (or would be) on screen, and the rows' visible area;
    // found from the rows drawn.
    let mut origin = None;
    let mut body_clip = Rect::NOTHING;
    let mut band_started = false;
    table
        .header(row_height + theme::GAP_SMALL, |mut header| {
            // None means the column can't be sorted by.
            let mut columns = vec![("Name", Some(SortColumn::Name))];
            if input.show_folder {
                columns.push(("Folder", None));
            }
            if input.columns.modified {
                columns.push(("Date modified", Some(SortColumn::Modified)));
            }
            if input.columns.kind {
                columns.push(("Type", Some(SortColumn::Type)));
            }
            if input.columns.size {
                columns.push(("Size", Some(SortColumn::Size)));
            }
            for (title, column) in columns {
                header.col(|ui| {
                    let Some(column) = column else {
                        ui.label(RichText::new(title).strong());
                        return;
                    };
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
                        let size = egui::vec2(SORT_ARROW_SIZE, SORT_ARROW_SIZE);
                        let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
                        let color = ui.visuals().weak_text_color();
                        paint_arrow(ui.painter(), rect, arrow, color);
                    }
                });
            }
        })
        .body(|mut body| {
            body_clip = body.ui_mut().clip_rect();
            band::auto_scroll(body.ui_mut(), band_id);
            // `rows` only calls the closure for rows on screen, so 50,000
            // entries cost no more per frame than 50.
            body.rows(row_height, input.visible.len(), |mut row| {
                let row_number = row.index();
                let entry = &input.entries[input.visible[row_number]];
                let drop_here = input.drop_target == Some(entry.path.as_path());
                row.set_selected(input.selection.is_selected(&entry.path) || drop_here);

                let (_, name_cell) = row.col(|ui| {
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
                if input.show_folder {
                    row.col(|ui| {
                        if let Some(parent) = entry.path.parent() {
                            cell_text(ui, RichText::new(parent.to_string_lossy()));
                        }
                    });
                }
                if columns.modified {
                    row.col(|ui| {
                        if let Some(mtime) = entry.mtime {
                            cell_text(ui, RichText::new(format_local_time(mtime)));
                        }
                    });
                }
                if columns.kind {
                    row.col(|ui| cell_text(ui, RichText::new(type_label(entry))));
                }
                if columns.size {
                    row.col(|ui| {
                        if !entry.is_dir() {
                            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                                cell_text(ui, RichText::new(format_size(entry.size)));
                            });
                        }
                    });
                }

                let response = row.response();
                let row_origin = egui::pos2(
                    response.rect.left(),
                    response.rect.top() - row_number as f32 * step,
                );
                origin = Some(row_origin);
                item_action(&response, row_number, modifiers, input.menu, &mut action);
                // Only the Name cell drags files; the rest of the row
                // starts a box.
                let started = response.drag_started_by(egui::PointerButton::Primary);
                if input.drag_select && started && !name_cell.drag_started() {
                    band::start(&response.ctx, band_id, row_origin);
                    action = Some(TableAction::BandStart {
                        add: modifiers.command,
                    });
                    band_started = true;
                }
                if input.dragging && response.contains_pointer() {
                    action = Some(TableAction::DropHover(row_number));
                }
            });
        });

    // The empty space below the rows: click to deselect, right-click for
    // the folder menu, drag for a box.
    let rest = ui.available_rect_before_wrap();
    let sense = if input.drag_select {
        Sense::click_and_drag()
    } else {
        Sense::click()
    };
    let background = ui.allocate_rect(rest, sense);
    background.context_menu(|ui| {
        if let Some(command) = menus::background_menu(ui, input.menu) {
            action = Some(TableAction::Command(command));
        }
    });
    if background.clicked() || background.secondary_clicked() {
        action = Some(TableAction::ClearSelection);
    }
    let Some(origin) = origin else {
        return action;
    };
    if background.drag_started_by(egui::PointerButton::Primary) {
        band::start(ui.ctx(), band_id, origin);
        action = Some(TableAction::BandStart {
            add: modifiers.command,
        });
        band_started = true;
    }
    if let Some(band) = band::current(ui.ctx(), band_id, origin) {
        // Over the rows and the space below them, not the header.
        let clip = body_clip.union(rest);
        band::paint(
            &ui.painter().with_clip_rect(clip),
            band,
            origin,
            input.theme,
        );
        if !band_started {
            let rows = band::rows_in(band, step, input.visible.len());
            action = Some(TableAction::Band(rows));
        }
    }
    return action;
}

/// How wide a column must be for its longest text, or for its header
/// (a frameless button, plus the sort arrow), whichever is wider.
fn column_width(ui: &egui::Ui, header: &str, widest: &str) -> f32 {
    let font = TextStyle::Body.resolve(ui.style());
    let painter = ui.painter();
    let content = painter
        .layout_no_wrap(widest.to_string(), font.clone(), Color32::PLACEHOLDER)
        .size()
        .x;
    let spacing = ui.spacing();
    let title = painter
        .layout_no_wrap(header.to_string(), font, Color32::PLACEHOLDER)
        .size()
        .x;
    let header_width =
        title + 2.0 * spacing.button_padding.x + spacing.item_spacing.x + SORT_ARROW_SIZE;
    // A little room so text doesn't touch the next column.
    return content.max(header_width) + theme::GAP;
}

/// What a click, double-click or right-click on an item means. Shared by
/// all the views.
pub fn item_action(
    response: &egui::Response,
    row: usize,
    modifiers: egui::Modifiers,
    menu: &MenuState,
    action: &mut Option<TableAction>,
) {
    if response.double_clicked() {
        *action = Some(TableAction::Open(row));
    } else if response.clicked() {
        *action = Some(TableAction::Click {
            row,
            ctrl: modifiers.command,
            shift: modifiers.shift,
        });
    } else if response.secondary_clicked() {
        *action = Some(TableAction::RightClick(row));
    } else if response.middle_clicked() {
        *action = Some(TableAction::MiddleClick(row));
    } else if response.drag_started_by(egui::PointerButton::Primary) {
        *action = Some(TableAction::DragStart(row));
    }
    response.context_menu(|ui| {
        if let Some(command) = menus::item_menu(ui, menu) {
            *action = Some(TableAction::Command(command));
        }
    });
}

/// The inline rename text box. Returns Some(commit) when it closes:
/// Enter or clicking elsewhere commits, Escape cancels.
pub fn rename_box(ui: &mut egui::Ui, editing: &mut Rename) -> Option<bool> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn columns_fit_their_content_or_header() {
        let ctx = egui::Context::default();
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            let date = column_width(ui, "Date modified", WIDEST_DATE);
            let size = column_width(ui, "Size", WIDEST_SIZE);
            // A short header doesn't make a column narrower than its content.
            assert!(size >= column_width(ui, "", WIDEST_SIZE));
            // Nor short content narrower than its header.
            assert_eq!(column_width(ui, "Size", ""), column_width(ui, "Size", "1"));
            // Longer text needs more room.
            assert!(column_width(ui, "Type", "JPEG File") > column_width(ui, "Type", "File"));
            // Both stay far narrower than a name column would be.
            assert!(date < 200.0 && size < 120.0, "date {date} size {size}");
        });
        output.textures_delta.clear();
    }
}
