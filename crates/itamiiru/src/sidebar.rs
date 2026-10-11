//! The navigation pane on the left: Quick access, Bookmarks, This PC and
//! Network (PLAN.md section 6).

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Rect, Sense, TextStyle};
use itamiiru_core::format::format_size;
use itamiiru_core::places::{DiskUsage, Drive, Place, Places};

use crate::icons::{paint_drive_icon, paint_place_icon};
use crate::theme::{self, Theme};

/// Height of a drive's usage bar, in points.
const USAGE_BAR_HEIGHT: f32 = 4.0;
/// A drive more full than this gets an error-colored bar, like Explorer's red.
const NEARLY_FULL: f32 = 0.9;

pub enum SidebarAction {
    GoTo(PathBuf),
    ThisPc,
}

/// Draws the pane. `current` is the folder on screen, or None on This PC.
pub fn show(
    ui: &mut egui::Ui,
    places: &Places,
    usage: &HashMap<PathBuf, DiskUsage>,
    current: Option<&Path>,
    show_hidden: bool,
    theme: &Theme,
) -> Option<SidebarAction> {
    let mut action = None;
    let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        section(ui, "Quick access", |ui| {
            for place in &places.quick {
                if place_row(ui, place, current, theme).clicked() {
                    action = Some(SidebarAction::GoTo(place.path.clone()));
                }
            }
        });
        if !places.bookmarks.is_empty() {
            section(ui, "Bookmarks", |ui| {
                for place in &places.bookmarks {
                    if place_row(ui, place, current, theme).clicked() {
                        action = Some(SidebarAction::GoTo(place.path.clone()));
                    }
                }
            });
        }

        // "This PC" opens its own page when the title is clicked; only the
        // arrow collapses it.
        let id = ui.make_persistent_id("this-pc");
        let state =
            egui::collapsing_header::CollapsingState::load_with_default_open(ui.ctx(), id, true);
        state
            .show_header(ui, |ui| {
                let mut text = egui::RichText::new("This PC").strong();
                if current.is_none() {
                    text = text.color(theme.primary);
                }
                let label = egui::Label::new(text)
                    .sense(Sense::click())
                    .selectable(false);
                let title = ui.add(label);
                if title.on_hover_text("Show all drives").clicked() {
                    action = Some(SidebarAction::ThisPc);
                }
            })
            .body(|ui| {
                for drive in &places.drives {
                    if drive.hidden && !show_hidden {
                        continue;
                    }
                    let drive_usage = usage.get(&drive.mount_point);
                    if drive_row(ui, drive, drive_usage, current, theme).clicked() {
                        action = Some(SidebarAction::GoTo(drive.mount_point.clone()));
                    }
                }
            });

        if !places.network.is_empty() {
            section(ui, "Network", |ui| {
                for place in &places.network {
                    if place_row(ui, place, current, theme).clicked() {
                        action = Some(SidebarAction::GoTo(place.path.clone()));
                    }
                }
            });
        }
    });
    return action;
}

/// A collapsible group with a heading, open by default.
fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .default_open(true)
        .show(ui, add_contents);
}

fn place_row(
    ui: &mut egui::Ui,
    place: &Place,
    current: Option<&Path>,
    theme: &Theme,
) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let selected = current == Some(place.path.as_path());
    let (response, icon_rect, text_left) = row(ui, line + theme::GAP_SMALL, selected, theme);
    if ui.is_rect_visible(response.rect) {
        paint_place_icon(ui.painter(), icon_rect, place.kind, theme);
        let text_pos = egui::pos2(text_left, response.rect.center().y);
        row_text(ui, response.rect, text_pos, &place.name, selected, theme);
    }
    return response.on_hover_text(place.path.to_string_lossy());
}

fn drive_row(
    ui: &mut egui::Ui,
    drive: &Drive,
    usage: Option<&DiskUsage>,
    current: Option<&Path>,
    theme: &Theme,
) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let height = line + USAGE_BAR_HEIGHT + 2.0 * theme::GAP_SMALL;
    let selected = current == Some(drive.mount_point.as_path());
    let (response, icon_rect, text_left) = row(ui, height, selected, theme);
    let rect = response.rect;
    if ui.is_rect_visible(rect) {
        paint_drive_icon(ui.painter(), icon_rect, drive.kind, theme);
        let text_pos = egui::pos2(text_left, rect.top() + theme::GAP_SMALL + line / 2.0);
        row_text(ui, rect, text_pos, &drive.name, selected, theme);

        if let Some(usage) = usage {
            let bar_top = rect.top() + theme::GAP_SMALL + line;
            let bar = Rect::from_min_max(
                egui::pos2(text_left, bar_top),
                egui::pos2(rect.right() - theme::GAP, bar_top + USAGE_BAR_HEIGHT),
            );
            let used = usage.used_fraction();
            let fill_color = if used > NEARLY_FULL {
                theme.error
            } else {
                theme.primary
            };
            ui.painter()
                .rect_filled(bar, 1.0, theme.surface_container_3);
            let mut filled = bar;
            filled.set_right(bar.left() + bar.width() * used);
            ui.painter().rect_filled(filled, 1.0, fill_color);
        }
    }

    let mut tip = drive.mount_point.to_string_lossy().into_owned();
    if !drive.fs_type.is_empty() {
        tip.push_str(&format!(" ({})", drive.fs_type));
    }
    if let Some(usage) = usage {
        tip.push_str(&format!(
            "\n{} free of {}",
            format_size(usage.free),
            format_size(usage.total)
        ));
    }
    return response.on_hover_text(tip);
}

/// Allocates one full-width clickable row and paints its hover or selected
/// background. Returns the response, where the icon goes, and where text starts.
fn row(
    ui: &mut egui::Ui,
    height: f32,
    selected: bool,
    theme: &Theme,
) -> (egui::Response, Rect, f32) {
    let size = egui::vec2(ui.available_width(), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
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
    let icon_rect = Rect::from_min_size(
        egui::pos2(
            rect.left() + theme::GAP_SMALL,
            rect.top() + theme::GAP_SMALL,
        ),
        egui::vec2(theme::ICON_SIZE, theme::ICON_SIZE),
    );
    let text_left = icon_rect.right() + theme::GAP;
    return (response, icon_rect, text_left);
}

/// Row text, clipped to the row so long names don't spill over.
fn row_text(ui: &egui::Ui, row: Rect, pos: egui::Pos2, text: &str, selected: bool, theme: &Theme) {
    let color = if selected {
        theme.on_secondary_container
    } else {
        theme.on_surface
    };
    let font = TextStyle::Body.resolve(ui.style());
    let painter = ui
        .painter()
        .with_clip_rect(row.shrink2(egui::vec2(theme::GAP_SMALL, 0.0)));
    painter.text(pos, Align2::LEFT_CENTER, text, font, color);
}
