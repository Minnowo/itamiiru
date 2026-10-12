//! The "This PC" page: tiles for folders, drives and network places,
//! with usage bars, like Windows Explorer.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use eframe::egui::{self, Align2, Rect, RichText, Sense, TextStyle};
use itamiiru_core::format::format_size;
use itamiiru_core::places::{DiskUsage, Drive, DriveKind, Place, PlaceKind, Places, Volume};

use crate::icons::{paint_drive_icon, paint_place_icon};
use crate::theme::{self, Theme};

const TILE_WIDTH: f32 = 260.0;
/// Tile icons are twice the size of list icons.
const TILE_ICON: f32 = 2.0 * theme::ICON_SIZE;
const USAGE_BAR_HEIGHT: f32 = 6.0;
const NEARLY_FULL: f32 = 0.9;

pub enum TileAction {
    Select(PathBuf),
    Open(PathBuf),
    CopyPath(PathBuf),
    /// Mount a plugged-in drive, then open it.
    Mount(Volume),
    /// Unmount a removable drive's filesystem, leaving it plugged in.
    Unmount(Volume),
    /// Unmount everything on a removable drive and power it off.
    Eject(Volume),
}

pub fn show(
    ui: &mut egui::Ui,
    places: &Places,
    usage: &HashMap<PathBuf, DiskUsage>,
    busy: &HashSet<String>,
    show_hidden: bool,
    selected: Option<&Path>,
    theme: &Theme,
) -> Option<TileAction> {
    let mut action = None;

    let mut folders: Vec<&Place> = Vec::new();
    for place in &places.quick {
        if place.kind != PlaceKind::Home {
            folders.push(place);
        }
    }
    let mut local_drives: Vec<&Drive> = Vec::new();
    let mut network_drives: Vec<&Drive> = Vec::new();
    for drive in &places.drives {
        if drive.hidden && !show_hidden {
            continue;
        }
        if drive.kind == DriveKind::Network {
            network_drives.push(drive);
        } else {
            local_drives.push(drive);
        }
    }

    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if !folders.is_empty() {
                heading(ui, "Folders");
                ui.horizontal_wrapped(|ui| {
                    for place in &folders {
                        let response = folder_tile(ui, place, selected, theme);
                        handle(&response, &place.path, None, busy, &mut action);
                    }
                });
                ui.add_space(theme::GAP_LARGE);
            }

            heading(ui, "Devices and drives");
            ui.horizontal_wrapped(|ui| {
                for drive in &local_drives {
                    let response =
                        drive_tile(ui, drive, usage.get(&drive.mount_point), selected, theme);
                    // Removable drives can be ejected from their menu.
                    let eject = places.volume_at(&drive.mount_point);
                    handle(&response, &drive.mount_point, eject, busy, &mut action);
                }
                for volume in places.unmounted() {
                    let working = busy.contains(&volume.id);
                    let response = volume_tile(ui, volume, working, theme);
                    if !working && response.double_clicked() {
                        action = Some(TileAction::Mount(volume.clone()));
                    }
                    response.context_menu(|ui| {
                        if ui
                            .add_enabled(!working, egui::Button::new("Mount and open"))
                            .clicked()
                        {
                            action = Some(TileAction::Mount(volume.clone()));
                            ui.close();
                        }
                    });
                }
            });

            if !network_drives.is_empty() || !places.network.is_empty() {
                ui.add_space(theme::GAP_LARGE);
                heading(ui, "Network locations");
                ui.horizontal_wrapped(|ui| {
                    for drive in &network_drives {
                        let response =
                            drive_tile(ui, drive, usage.get(&drive.mount_point), selected, theme);
                        handle(&response, &drive.mount_point, None, busy, &mut action);
                    }
                    for place in &places.network {
                        let response = folder_tile(ui, place, selected, theme);
                        handle(&response, &place.path, None, busy, &mut action);
                    }
                });
            }
        });
    return action;
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(RichText::new(text).strong());
    ui.add_space(theme::GAP_SMALL);
}

/// Click selects, double-click opens, right-click shows a small menu, with
/// Eject for a removable drive (`eject`).
fn handle(
    response: &egui::Response,
    path: &Path,
    eject: Option<&Volume>,
    busy: &HashSet<String>,
    action: &mut Option<TileAction>,
) {
    if response.double_clicked() {
        *action = Some(TileAction::Open(path.to_path_buf()));
    } else if response.clicked() || response.secondary_clicked() {
        *action = Some(TileAction::Select(path.to_path_buf()));
    }
    response.context_menu(|ui| {
        if ui.button("Open").clicked() {
            *action = Some(TileAction::Open(path.to_path_buf()));
            ui.close();
        }
        if ui.button("Copy path").clicked() {
            *action = Some(TileAction::CopyPath(path.to_path_buf()));
            ui.close();
        }
        if let Some(volume) = eject {
            ui.separator();
            let enabled = !busy.contains(&volume.id);
            let unmount = ui.add_enabled(enabled, egui::Button::new("Unmount"));
            if unmount
                .on_hover_text("Stays plugged in; mount it again with a click")
                .clicked()
            {
                *action = Some(TileAction::Unmount(volume.clone()));
                ui.close();
            }
            let eject = ui.add_enabled(enabled, egui::Button::new("Eject"));
            if eject
                .on_hover_text("Unmounts it and powers it off, ready to unplug")
                .clicked()
            {
                *action = Some(TileAction::Eject(volume.clone()));
                ui.close();
            }
        }
    });
}

/// A plugged-in drive that isn't mounted yet: dimmed, "Not mounted"
/// where mounted drives show their usage.
fn volume_tile(ui: &mut egui::Ui, volume: &Volume, working: bool, theme: &Theme) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body);
    let small = ui.text_style_height(&TextStyle::Small);
    let content = line + theme::GAP_SMALL + small;
    let height = content.max(TILE_ICON) + 2.0 * theme::GAP;
    let (response, icon_rect, text_left) = tile(ui, height, false, theme);
    let rect = response.rect;
    if ui.is_rect_visible(rect) {
        let mut painter = ui.painter().clone();
        painter.multiply_opacity(0.6);
        paint_drive_icon(&painter, icon_rect, DriveKind::Removable, theme);
        let top = rect.center().y - content / 2.0;
        let name_pos = egui::pos2(text_left, top + line / 2.0);
        tile_text(
            ui,
            rect,
            name_pos,
            &volume.name,
            TextStyle::Body,
            theme.on_surface,
        );
        let status = if working {
            "Mounting...".to_string()
        } else {
            format!("Not mounted, {}", format_size(volume.size))
        };
        let small_pos = egui::pos2(text_left, top + line + theme::GAP_SMALL + small / 2.0);
        tile_text(
            ui,
            rect,
            small_pos,
            &status,
            TextStyle::Small,
            theme.on_surface_variant,
        );
    }
    let tip = format!(
        "{} ({})\nDouble-click to mount and open it",
        volume.device.to_string_lossy(),
        volume.fs_type
    );
    return response.on_hover_text(tip);
}

fn folder_tile(
    ui: &mut egui::Ui,
    place: &Place,
    selected: Option<&Path>,
    theme: &Theme,
) -> egui::Response {
    let height = TILE_ICON + 2.0 * theme::GAP;
    let is_selected = selected == Some(place.path.as_path());
    let (response, icon_rect, text_left) = tile(ui, height, is_selected, theme);
    if ui.is_rect_visible(response.rect) {
        paint_place_icon(ui.painter(), icon_rect, place.kind, theme);
        let pos = egui::pos2(text_left, response.rect.center().y);
        tile_text(
            ui,
            response.rect,
            pos,
            &place.name,
            TextStyle::Body,
            theme.on_surface,
        );
    }
    return response.on_hover_text(place.path.to_string_lossy());
}

fn drive_tile(
    ui: &mut egui::Ui,
    drive: &Drive,
    usage: Option<&DiskUsage>,
    selected: Option<&Path>,
    theme: &Theme,
) -> egui::Response {
    let line = ui.text_style_height(&TextStyle::Body);
    let small = ui.text_style_height(&TextStyle::Small);
    let content = line + theme::GAP_SMALL + USAGE_BAR_HEIGHT + theme::GAP_SMALL + small;
    let height = content.max(TILE_ICON) + 2.0 * theme::GAP;
    let is_selected = selected == Some(drive.mount_point.as_path());
    let (response, icon_rect, text_left) = tile(ui, height, is_selected, theme);
    let rect = response.rect;

    if ui.is_rect_visible(rect) {
        paint_drive_icon(ui.painter(), icon_rect, drive.kind, theme);
        let top = rect.center().y - content / 2.0;
        let name_pos = egui::pos2(text_left, top + line / 2.0);
        tile_text(
            ui,
            rect,
            name_pos,
            &drive.name,
            TextStyle::Body,
            theme.on_surface,
        );

        let bar_top = top + line + theme::GAP_SMALL;
        let bar = Rect::from_min_max(
            egui::pos2(text_left, bar_top),
            egui::pos2(
                rect.right() - theme::PADDING as f32,
                bar_top + USAGE_BAR_HEIGHT,
            ),
        );
        let small_pos = egui::pos2(text_left, bar.bottom() + theme::GAP_SMALL + small / 2.0);
        match usage {
            Some(usage) => {
                let used = usage.used_fraction();
                let fill = if used > NEARLY_FULL {
                    theme.error
                } else {
                    theme.primary
                };
                ui.painter()
                    .rect_filled(bar, 2.0, theme.surface_container_3);
                let mut filled = bar;
                filled.set_right(bar.left() + bar.width() * used);
                ui.painter().rect_filled(filled, 2.0, fill);
                let text = format!(
                    "{} free of {}",
                    format_size(usage.free),
                    format_size(usage.total)
                );
                tile_text(
                    ui,
                    rect,
                    small_pos,
                    &text,
                    TextStyle::Small,
                    theme.on_surface_variant,
                );
            }
            None => {
                tile_text(
                    ui,
                    rect,
                    small_pos,
                    "Size unknown",
                    TextStyle::Small,
                    theme.on_surface_variant,
                );
            }
        }
    }

    let mut tip = drive.mount_point.to_string_lossy().into_owned();
    if !drive.fs_type.is_empty() {
        tip.push_str(&format!(" ({})", drive.fs_type));
    }
    return response.on_hover_text(tip);
}

/// Allocates one tile and paints its background (a surface box that
/// lightens on hover, or the selection color). Returns the response, where
/// the icon goes, and where text starts.
fn tile(
    ui: &mut egui::Ui,
    height: f32,
    selected: bool,
    theme: &Theme,
) -> (egui::Response, Rect, f32) {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(TILE_WIDTH, height), Sense::click());
    let fill = if selected {
        theme.secondary_container
    } else if response.hovered() {
        theme.surface_container_1_hover
    } else {
        theme.surface_container_1
    };
    ui.painter().rect_filled(rect, theme::RADIUS_MEDIUM, fill);
    let padding = theme::PADDING as f32;
    let icon_rect = Rect::from_min_size(
        egui::pos2(rect.left() + padding, rect.center().y - TILE_ICON / 2.0),
        egui::vec2(TILE_ICON, TILE_ICON),
    );
    let text_left = icon_rect.right() + theme::GAP;
    return (response, icon_rect, text_left);
}

/// One line of text inside a tile. Clipped at the sides only, so long names
/// stop at the padding; clipping top and bottom too would cut into the
/// drive tiles' first and last lines, which sit closer to the edge.
fn tile_text(
    ui: &egui::Ui,
    tile: Rect,
    pos: egui::Pos2,
    text: &str,
    style: TextStyle,
    color: egui::Color32,
) {
    let font = style.resolve(ui.style());
    let clip = tile.shrink2(egui::vec2(theme::PADDING as f32, 0.0));
    let painter = ui.painter().with_clip_rect(clip);
    painter.text(pos, Align2::LEFT_CENTER, text, font, color);
}
