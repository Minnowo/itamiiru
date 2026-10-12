//! The settings page: a dialog for the options in the config file (PLAN.md
//! section 6), opened from the toolbar or with Ctrl+Comma. Changes apply
//! right away and are saved like every other setting. It edits a copy of
//! the options; the app compares and applies what changed.

use std::path::Path;
use std::time::SystemTime;

use eframe::egui;
use itamiiru_core::entry::{Entry, EntryKind};

use crate::icons::{Arrow, arrow_button};
use crate::settings::{DetailsColumns, Settings, StatusField};
use crate::theme::{self, Accent, Theme};

const DIALOG_WIDTH: f32 = 460.0;

/// Everything the page can change.
#[derive(Clone, Debug, PartialEq)]
pub struct Options {
    pub show_hidden: bool,
    pub auto_refresh_folder: bool,
    pub auto_refresh_drives: bool,
    pub thumbnails: bool,
    pub preview_pane: bool,
    pub status_fields: Vec<StatusField>,
    pub columns: DetailsColumns,
}

impl Options {
    /// The options as they are on a fresh install.
    pub fn defaults() -> Options {
        let d = Settings::default();
        return Options {
            show_hidden: d.show_hidden,
            auto_refresh_folder: d.auto_refresh_folder,
            auto_refresh_drives: d.auto_refresh_drives,
            thumbnails: d.thumbnails,
            preview_pane: d.preview_pane,
            status_fields: d.status_fields,
            columns: d.columns,
        };
    }
}

/// Draws the dialog. `config_path` is shown so people can find the file.
/// Returns false once it's closed (Done, Escape, or a click outside it).
pub fn show(
    ctx: &egui::Context,
    theme: &Theme,
    options: &mut Options,
    config_path: Option<&Path>,
) -> bool {
    let mut open = true;
    let modal = egui::Modal::new(egui::Id::new("settings")).show(ctx, |ui| {
        ui.set_width(DIALOG_WIDTH);
        ui.heading("Settings");
        ui.add_space(theme::GAP);
        // Taller than the window: scroll instead of running off it.
        let max_height = ctx.content_rect().height() * 0.7;
        egui::ScrollArea::vertical()
            .max_height(max_height)
            .show(ui, |ui| {
                section(ui, theme, "Folders", |ui| {
                    ui.checkbox(&mut options.show_hidden, "Show hidden files")
                        .on_hover_text("Ctrl+H");
                    ui.checkbox(
                        &mut options.auto_refresh_folder,
                        "Refresh the open folder when its contents change",
                    )
                    .on_hover_text(
                        "Checked every 2 seconds while the window is focused. \
                         Off: use the refresh button or F5.",
                    );
                });
                section(ui, theme, "Drives", |ui| {
                    ui.checkbox(
                        &mut options.auto_refresh_drives,
                        "Notice new drives and bookmarks while the window is open",
                    )
                    .on_hover_text(
                        "Checked every 5 seconds while the window is focused. \
                         Off: use the refresh button at the bottom of the side pane.",
                    );
                });
                section(ui, theme, "Pictures", |ui| {
                    ui.checkbox(&mut options.thumbnails, "Show thumbnails in the icon views");
                    ui.checkbox(&mut options.preview_pane, "Show the preview pane")
                        .on_hover_text("Alt+P");
                });
                section(ui, theme, "Details view", |ui| {
                    ui.label("Columns besides Name, which takes the rest of the width:");
                    ui.checkbox(&mut options.columns.modified, "Date modified");
                    ui.checkbox(&mut options.columns.kind, "Type")
                        .on_hover_text("The extension, e.g. \"PNG File\", or \"Folder\"");
                    ui.checkbox(&mut options.columns.size, "Size");
                });
                section(ui, theme, "Status bar", |ui| {
                    ui.label("Details shown for one selected item, in this order:");
                    status_fields(ui, &mut options.status_fields);
                    ui.add_space(theme::GAP_SMALL);
                    ui.weak(format!("Example: {}", example(&options.status_fields)));
                });
                if let Some(path) = config_path {
                    section(ui, theme, "Files", |ui| {
                        ui.weak(format!("Settings are saved in {}", path.display()));
                        if let Some(dir) = path.parent() {
                            let theme_file = dir.join("theme");
                            ui.weak(format!(
                                "Colors can be changed in {} (see the README)",
                                theme_file.display()
                            ));
                            ui.weak(format!(
                                "Right-click actions are in {} (read at startup; see the README)",
                                dir.join("actions").display()
                            ));
                        }
                    });
                }
            });
        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            if ui.button("Restore defaults").clicked() {
                *options = Options::defaults();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme.filled_button(ui, Accent::Primary, "Done").clicked() {
                    open = false;
                }
            });
        });
    });
    if modal.should_close() {
        open = false;
    }
    return open;
}

/// A heading and its settings on a raised surface.
fn section(
    ui: &mut egui::Ui,
    theme: &Theme,
    title: &str,
    add_contents: impl FnOnce(&mut egui::Ui),
) {
    egui::Frame::new()
        .fill(theme.surface_container_2)
        .corner_radius(theme::RADIUS_MEDIUM)
        .inner_margin(theme::PADDING)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.strong(title);
            ui.add_space(theme::GAP_SMALL);
            add_contents(ui);
        });
    ui.add_space(theme::GAP);
}

/// One row per field: a checkbox, and for the shown ones, arrows that
/// move them earlier or later.
fn status_fields(ui: &mut egui::Ui, fields: &mut Vec<StatusField>) {
    for (field, shown) in field_rows(fields) {
        ui.horizontal(|ui| {
            let position = fields.iter().position(|f| *f == field);
            let can_move_up = position.is_some_and(|i| i > 0);
            let can_move_down = position.is_some_and(|i| i + 1 < fields.len());
            let up = ui.add_enabled_ui(can_move_up, |ui| arrow_button(ui, Arrow::Up));
            if up.inner.on_hover_text("Earlier").clicked() {
                move_field(fields, field, -1);
            }
            let down = ui.add_enabled_ui(can_move_down, |ui| arrow_button(ui, Arrow::Down));
            if down.inner.on_hover_text("Later").clicked() {
                move_field(fields, field, 1);
            }
            let mut checked = shown;
            if ui.checkbox(&mut checked, field_label(field)).changed() {
                set_field(fields, field, checked);
            }
        });
    }
}

fn field_label(field: StatusField) -> &'static str {
    return match field {
        StatusField::Name => "Name",
        StatusField::Type => "Type",
        StatusField::Size => "Size (files only)",
        StatusField::Modified => "Date modified",
        StatusField::Path => "Full path",
    };
}

/// The rows to show: the shown fields in their order, then the rest.
fn field_rows(fields: &[StatusField]) -> Vec<(StatusField, bool)> {
    let mut rows = Vec::new();
    for field in fields {
        rows.push((*field, true));
    }
    for field in StatusField::ALL {
        if !fields.contains(&field) {
            rows.push((field, false));
        }
    }
    return rows;
}

/// Shows a field (added at the end) or hides it.
fn set_field(fields: &mut Vec<StatusField>, field: StatusField, shown: bool) {
    fields.retain(|f| *f != field);
    if shown {
        fields.push(field);
    }
}

/// Moves a shown field one place earlier (-1) or later (+1).
fn move_field(fields: &mut [StatusField], field: StatusField, step: isize) {
    let Some(from) = fields.iter().position(|f| *f == field) else {
        return;
    };
    let to = from as isize + step;
    if to < 0 || to as usize >= fields.len() {
        return;
    }
    fields.swap(from, to as usize);
}

/// What the status bar would say for a sample photo.
fn example(fields: &[StatusField]) -> String {
    let sample = Entry {
        name: "photo.jpg".into(),
        path: "/home/you/Pictures/photo.jpg".into(),
        kind: EntryKind::File,
        size: 3 * 1024 * 1024,
        mtime: Some(SystemTime::now()),
        hidden: false,
        executable: false,
    };
    return crate::app::selection_summary(&[&sample], fields).unwrap_or_default();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rows_put_shown_fields_first_in_order() {
        let fields = vec![StatusField::Size, StatusField::Name];
        let rows = field_rows(&fields);
        assert_eq!(rows.len(), StatusField::ALL.len());
        assert_eq!(rows[0], (StatusField::Size, true));
        assert_eq!(rows[1], (StatusField::Name, true));
        assert_eq!(rows[2], (StatusField::Type, false));
    }

    #[test]
    fn showing_hiding_and_moving_fields() {
        let mut fields = vec![StatusField::Type, StatusField::Size];
        set_field(&mut fields, StatusField::Name, true);
        assert_eq!(
            fields,
            [StatusField::Type, StatusField::Size, StatusField::Name]
        );
        move_field(&mut fields, StatusField::Name, -1);
        move_field(&mut fields, StatusField::Name, -1);
        assert_eq!(
            fields,
            [StatusField::Name, StatusField::Type, StatusField::Size]
        );
        // Already first: stays.
        move_field(&mut fields, StatusField::Name, -1);
        assert_eq!(fields[0], StatusField::Name);
        // Not shown: nothing to move.
        move_field(&mut fields, StatusField::Path, 1);
        assert_eq!(fields.len(), 3);
        set_field(&mut fields, StatusField::Type, false);
        assert_eq!(fields, [StatusField::Name, StatusField::Size]);
    }

    #[test]
    fn example_follows_the_fields() {
        assert_eq!(example(&[StatusField::Name]), "photo.jpg");
        assert_eq!(
            example(&[StatusField::Size, StatusField::Type]),
            "3.0 MB, JPG File"
        );
    }
}
