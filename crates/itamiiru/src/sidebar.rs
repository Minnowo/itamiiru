//! The navigation pane on the left: Quick access, Bookmarks, This PC and
//! Network.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Rect, Sense, TextStyle};
use itamiiru_core::format::format_size;
use itamiiru_core::places::{DiskUsage, Drive, DriveKind, Place, Places, Volume};

use crate::icons::{ButtonIcon, icon_button, paint_drive_icon, paint_eject, paint_place_icon};
use crate::theme::{self, Theme};

/// Height of a drive's usage bar, in points.
const USAGE_BAR_HEIGHT: f32 = 4.0;
/// A drive more full than this gets an error-colored bar, like Explorer's red.
const NEARLY_FULL: f32 = 0.9;

/// Items dragged over the side pane: they can be dropped on a place or
/// a drive.
#[derive(Default)]
pub struct SidebarDrop {
    /// Items are being dragged (set by the app before drawing).
    pub dragging: bool,
    /// Where the app would drop them, as of the last frame: that row is
    /// drawn selected.
    pub target: Option<PathBuf>,
    /// The row under the pointer (set while drawing).
    pub hover: Option<PathBuf>,
}

impl SidebarDrop {
    /// The row for `path` is drawn selected: it's the folder on screen,
    /// or where a drag would drop.
    fn marks(&self, current: Option<&Path>, path: &Path) -> bool {
        return current == Some(path) || self.target.as_deref() == Some(path);
    }

    fn notice(&mut self, row: &egui::Response, path: &Path) {
        if self.dragging && row.contains_pointer() {
            self.hover = Some(path.to_path_buf());
        }
    }
}

pub enum SidebarAction {
    GoTo(PathBuf),
    ThisPc,
    /// Mount a plugged-in drive, then open it.
    Mount(Volume),
    /// Unmount a removable drive's filesystem, leaving it plugged in.
    Unmount(Volume),
    /// Unmount everything on a removable drive and power it off.
    Eject(Volume),
    /// From the right-click menu.
    OpenInNewTab(PathBuf),
    CopyPath(PathBuf),
    RemoveBookmark(PathBuf),
    /// The buttons at the bottom: look for drives and bookmarks again,
    /// and open the settings page.
    Refresh,
    Settings,
}

/// Draws the pane. `current` is the folder on screen, or None on This PC.
/// `busy` holds the volumes being mounted or ejected right now. `tabs` is
/// false in the picker, which has no tabs to open a place in.
#[allow(clippy::too_many_arguments)]
pub fn show(
    ui: &mut egui::Ui,
    places: &Places,
    usage: &HashMap<PathBuf, DiskUsage>,
    busy: &HashSet<String>,
    current: Option<&Path>,
    show_hidden: bool,
    tabs: bool,
    drop: &mut SidebarDrop,
    theme: &Theme,
) -> Option<SidebarAction> {
    let mut action = None;
    drop.hover = None;
    // The buttons sit at the bottom; the places scroll above them.
    egui::Panel::bottom("sidebar-buttons")
        .show_separator_line(false)
        .frame(egui::Frame::new().inner_margin(egui::Margin {
            left: 0,
            right: 0,
            top: theme::GAP as i8,
            bottom: 0,
        }))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                let refresh = icon_button(ui, ButtonIcon::Refresh, false);
                if refresh
                    .on_hover_text("Look for drives and bookmarks again")
                    .clicked()
                {
                    action = Some(SidebarAction::Refresh);
                }
                let settings = icon_button(ui, ButtonIcon::Settings, false);
                if settings.on_hover_text("Settings (Ctrl+Comma)").clicked() {
                    action = Some(SidebarAction::Settings);
                }
            });
        });
    let scroll = egui::ScrollArea::vertical().auto_shrink([false, false]);
    scroll.show(ui, |ui| {
        section(ui, "Quick access", |ui| {
            for place in &places.quick {
                let selected = drop.marks(current, &place.path);
                let row = place_row(ui, place, selected, theme);
                drop.notice(&row, &place.path);
                if row.clicked() {
                    action = Some(SidebarAction::GoTo(place.path.clone()));
                }
                place_menu(&row, &place.path, None, busy, tabs, false, &mut action);
            }
        });
        if !places.bookmarks.is_empty() {
            section(ui, "Bookmarks", |ui| {
                for place in &places.bookmarks {
                    let selected = drop.marks(current, &place.path);
                    let row = place_row(ui, place, selected, theme);
                    drop.notice(&row, &place.path);
                    if row.clicked() {
                        action = Some(SidebarAction::GoTo(place.path.clone()));
                    }
                    place_menu(&row, &place.path, None, busy, tabs, true, &mut action);
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
                    let volume = places.volume_at(&drive.mount_point);
                    let removable = volume.is_some();
                    let selected = drop.marks(current, &drive.mount_point);
                    let row = drive_row(ui, drive, drive_usage, removable, selected, theme);
                    drop.notice(&row, &drive.mount_point);
                    if row.clicked() {
                        action = Some(SidebarAction::GoTo(drive.mount_point.clone()));
                    }
                    place_menu(
                        &row,
                        &drive.mount_point,
                        volume,
                        busy,
                        tabs,
                        false,
                        &mut action,
                    );
                }
                // Plugged in, not mounted yet: a click mounts and opens it.
                for volume in places.unmounted() {
                    let working = busy.contains(&volume.id);
                    let row = volume_row(ui, volume, working, theme);
                    if row.clicked() && !working {
                        action = Some(SidebarAction::Mount(volume.clone()));
                    }
                    row.context_menu(|ui| {
                        let mount = egui::Button::new("Mount and open");
                        if ui.add_enabled(!working, mount).clicked() {
                            action = Some(SidebarAction::Mount(volume.clone()));
                            ui.close();
                        }
                    });
                }
            });

        if !places.network.is_empty() {
            section(ui, "Network", |ui| {
                for place in &places.network {
                    let selected = drop.marks(current, &place.path);
                    let row = place_row(ui, place, selected, theme);
                    drop.notice(&row, &place.path);
                    if row.clicked() {
                        action = Some(SidebarAction::GoTo(place.path.clone()));
                    }
                    place_menu(&row, &place.path, None, busy, tabs, false, &mut action);
                }
            });
        }
    });
    return action;
}

/// The right-click menu of a folder, bookmark or drive row; removable
/// drives (`eject`) also get Eject.
fn place_menu(
    row: &egui::Response,
    path: &Path,
    eject: Option<&Volume>,
    busy: &HashSet<String>,
    tabs: bool,
    is_bookmark: bool,
    action: &mut Option<SidebarAction>,
) {
    row.context_menu(|ui| {
        if ui.button("Open").clicked() {
            *action = Some(SidebarAction::GoTo(path.to_path_buf()));
            ui.close();
        }
        if tabs && ui.button("Open in new tab").clicked() {
            *action = Some(SidebarAction::OpenInNewTab(path.to_path_buf()));
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            *action = Some(SidebarAction::CopyPath(path.to_path_buf()));
            ui.close();
        }
        if is_bookmark {
            ui.separator();
            if ui.button("Remove from Bookmarks").clicked() {
                *action = Some(SidebarAction::RemoveBookmark(path.to_path_buf()));
                ui.close();
            }
        }
        if let Some(volume) = eject {
            ui.separator();
            let enabled = !busy.contains(&volume.id);
            let unmount = ui.add_enabled(enabled, egui::Button::new("Unmount"));
            if unmount
                .on_hover_text("Stays plugged in; mount it again with a click")
                .clicked()
            {
                *action = Some(SidebarAction::Unmount(volume.clone()));
                ui.close();
            }
            let eject = ui.add_enabled(enabled, egui::Button::new("Eject"));
            if eject
                .on_hover_text("Unmounts it and powers it off, ready to unplug")
                .clicked()
            {
                *action = Some(SidebarAction::Eject(volume.clone()));
                ui.close();
            }
        }
    });
}

/// A collapsible group with a heading, open by default.
fn section(ui: &mut egui::Ui, title: &str, add_contents: impl FnOnce(&mut egui::Ui)) {
    egui::CollapsingHeader::new(egui::RichText::new(title).strong())
        .default_open(true)
        .show(ui, add_contents);
}

fn place_row(ui: &mut egui::Ui, place: &Place, selected: bool, theme: &Theme) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let (response, icon_rect, text_left) = row(ui, line + theme::GAP_SMALL, selected, theme);
    if ui.is_rect_visible(response.rect) {
        paint_place_icon(ui.painter(), icon_rect, place.kind, theme);
        let text_pos = egui::pos2(text_left, response.rect.center().y);
        row_text(ui, response.rect, text_pos, &place.name, selected, theme);
    }
    return response.on_hover_text(place.path.to_string_lossy());
}

/// A mounted drive with its usage bar. Removable ones get the eject
/// symbol at the right, as a marker; unmount and eject are in the
/// right-click menu.
fn drive_row(
    ui: &mut egui::Ui,
    drive: &Drive,
    usage: Option<&DiskUsage>,
    removable: bool,
    selected: bool,
    theme: &Theme,
) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let height = line + USAGE_BAR_HEIGHT + 2.0 * theme::GAP_SMALL;
    let (response, icon_rect, text_left) = row(ui, height, selected, theme);
    let mut rect = response.rect;

    if removable {
        let side = theme::ICON_SIZE + theme::GAP_SMALL;
        let mark = Rect::from_center_size(
            egui::pos2(
                rect.right() - theme::GAP_SMALL - side / 2.0,
                rect.center().y,
            ),
            egui::vec2(side, side),
        );
        let color = if selected {
            theme.on_secondary_container
        } else {
            theme.on_surface_variant
        };
        paint_eject(ui.painter(), mark.shrink(side * 0.25), color);
        // Name and bar stop before the mark.
        rect.set_right(mark.left() - theme::GAP_SMALL);
    }

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
    if removable {
        tip.push_str("\nRemovable: right-click to unmount or eject");
    }
    return response.on_hover_text(tip);
}

/// A plugged-in drive that isn't mounted: dimmed, with "Not mounted" (or
/// "Mounting...") where mounted drives show their usage bar.
fn volume_row(ui: &mut egui::Ui, volume: &Volume, working: bool, theme: &Theme) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body).max(theme::ICON_SIZE);
    let small = ui.text_style_height(&TextStyle::Small);
    let height = line + small + 2.0 * theme::GAP_SMALL;
    let (response, icon_rect, text_left) = row(ui, height, false, theme);
    let rect = response.rect;
    if ui.is_rect_visible(rect) {
        let mut painter = ui.painter().clone();
        painter.multiply_opacity(0.6);
        paint_drive_icon(&painter, icon_rect, DriveKind::Removable, theme);
        let name_pos = egui::pos2(text_left, rect.top() + theme::GAP_SMALL + line / 2.0);
        row_text(ui, rect, name_pos, &volume.name, false, theme);
        let status = if working {
            "Mounting..."
        } else {
            "Not mounted"
        };
        let font = TextStyle::Small.resolve(ui.style());
        let status_pos = egui::pos2(
            text_left,
            rect.top() + theme::GAP_SMALL + line + small / 2.0,
        );
        ui.painter().with_clip_rect(rect).text(
            status_pos,
            Align2::LEFT_CENTER,
            status,
            font,
            theme.on_surface_variant,
        );
    }
    let mut tip = format!("{} ({})", volume.device.to_string_lossy(), volume.fs_type);
    tip.push_str("\nClick to mount and open it");
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
