//! Small icons painted with egui's painter, so there are no icon fonts or
//! image files to ship.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextStyle};
use itamiiru_core::entry::{Entry, EntryKind};
use itamiiru_core::filekind::{FileKind, file_kind};

use crate::theme::Theme;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Arrow {
    Left,
    Right,
    Up,
    Down,
}

/// Paints a filled triangle pointing in the `arrow` direction, centered in `rect`.
pub fn paint_arrow(painter: &egui::Painter, rect: Rect, arrow: Arrow, color: Color32) {
    let c = rect.center();
    // Half the side of the square the triangle fits in.
    let r = rect.width().min(rect.height()) / 2.0;
    let points = match arrow {
        Arrow::Up => vec![
            Pos2::new(c.x, c.y - r * 0.6),
            Pos2::new(c.x + r, c.y + r * 0.4),
            Pos2::new(c.x - r, c.y + r * 0.4),
        ],
        Arrow::Down => vec![
            Pos2::new(c.x - r, c.y - r * 0.4),
            Pos2::new(c.x + r, c.y - r * 0.4),
            Pos2::new(c.x, c.y + r * 0.6),
        ],
        Arrow::Left => vec![
            Pos2::new(c.x - r * 0.6, c.y),
            Pos2::new(c.x + r * 0.4, c.y - r),
            Pos2::new(c.x + r * 0.4, c.y + r),
        ],
        Arrow::Right => vec![
            Pos2::new(c.x + r * 0.6, c.y),
            Pos2::new(c.x - r * 0.4, c.y + r),
            Pos2::new(c.x - r * 0.4, c.y - r),
        ],
    };
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
}

/// A square button, the height of a normal button, showing an arrow.
/// Put it in `ui.add_enabled_ui(..)` to disable it; egui then fades it.
pub fn arrow_button(ui: &mut egui::Ui, arrow: Arrow) -> egui::Response {
    let side = ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::click());
    if ui.is_rect_visible(rect) {
        // The same colors a plain button would use in its current state.
        let visuals = ui.style().interact(&response);
        let painter = ui.painter();
        painter.rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
        paint_arrow(
            painter,
            rect.shrink(side * 0.32),
            arrow,
            visuals.fg_stroke.color,
        );
    }
    return response;
}

/// Paints the icon for a file list entry in `rect` (square, any size).
/// Colors come from the theme's categorical palette.
pub fn paint_entry_icon(painter: &egui::Painter, rect: Rect, entry: &Entry, theme: &Theme) {
    let kind = file_kind(entry);
    if kind == FileKind::Folder {
        paint_folder(painter, rect, theme);
    } else {
        paint_page(painter, rect, kind, theme);
    }
    if entry.executable {
        paint_executable_badge(painter, rect, theme);
    }
    match entry.kind {
        EntryKind::SymlinkToDir | EntryKind::SymlinkToFile => {
            paint_link_badge(painter, rect, theme.surface_container_3, theme.on_surface);
        }
        EntryKind::BrokenSymlink => {
            paint_link_badge(painter, rect, theme.error, theme.on_error);
        }
        _ => {}
    }
}

/// A point at fractions (0 to 1) of `rect`'s width and height.
fn at(rect: Rect, fx: f32, fy: f32) -> Pos2 {
    return Pos2::new(
        rect.min.x + fx * rect.width(),
        rect.min.y + fy * rect.height(),
    );
}

fn paint_folder(painter: &egui::Painter, rect: Rect, theme: &Theme) {
    let tab = Rect::from_min_max(at(rect, 0.05, 0.12), at(rect, 0.45, 0.35));
    painter.rect_filled(tab, 1.5, theme.d_yellow);
    let body = Rect::from_min_max(at(rect, 0.05, 0.25), at(rect, 0.95, 0.88));
    painter.rect_filled(body, 2.0, theme.yellow);
}

/// A sheet of paper with a folded corner, with a mark showing its kind.
fn paint_page(painter: &egui::Painter, rect: Rect, kind: FileKind, theme: &Theme) {
    // Fill and the color of the marks drawn on it (the matching `on-` role).
    let (fill, mark) = match kind {
        FileKind::Image => (theme.blue, theme.on_blue),
        FileKind::Archive => (theme.red, theme.on_red),
        FileKind::Audio => (theme.pink, theme.on_pink),
        FileKind::Video => (theme.d_pink, theme.on_d_pink),
        FileKind::Document => (theme.green, theme.on_green),
        FileKind::Text => (theme.on_surface_variant, theme.surface),
        FileKind::Folder | FileKind::Other => (theme.outline, theme.surface),
    };
    let fold = 0.25;
    let page = vec![
        at(rect, 0.15, 0.05),
        at(rect, 0.85 - fold, 0.05),
        at(rect, 0.85, 0.05 + fold),
        at(rect, 0.85, 0.95),
        at(rect, 0.15, 0.95),
    ];
    painter.add(Shape::convex_polygon(page, fill, Stroke::NONE));
    let corner = vec![
        at(rect, 0.85 - fold, 0.05),
        at(rect, 0.85, 0.05 + fold),
        at(rect, 0.85 - fold, 0.05 + fold),
    ];
    painter.add(Shape::convex_polygon(
        corner,
        mark.gamma_multiply(0.35),
        Stroke::NONE,
    ));

    let line = Stroke::new((rect.height() / 16.0).max(1.0), mark);
    match kind {
        FileKind::Text | FileKind::Document => {
            for fy in [0.45, 0.6, 0.75] {
                painter.line_segment([at(rect, 0.28, fy), at(rect, 0.72, fy)], line);
            }
        }
        FileKind::Image => {
            let mountain = vec![
                at(rect, 0.22, 0.85),
                at(rect, 0.45, 0.5),
                at(rect, 0.7, 0.85),
            ];
            painter.add(Shape::convex_polygon(mountain, mark, Stroke::NONE));
            painter.circle_filled(at(rect, 0.65, 0.45), rect.width() * 0.08, mark);
        }
        FileKind::Archive => {
            // A zipper down the middle.
            for (i, fy) in [0.12, 0.24, 0.36, 0.48, 0.6].into_iter().enumerate() {
                let fx = if i % 2 == 0 { 0.42 } else { 0.5 };
                let tooth = Rect::from_min_size(
                    at(rect, fx, fy),
                    egui::vec2(rect.width() * 0.08, rect.height() * 0.08),
                );
                painter.rect_filled(tooth, 0.0, mark);
            }
        }
        FileKind::Audio => {
            painter.circle_filled(at(rect, 0.42, 0.75), rect.width() * 0.1, mark);
            painter.line_segment([at(rect, 0.5, 0.75), at(rect, 0.5, 0.38)], line);
            painter.line_segment([at(rect, 0.5, 0.38), at(rect, 0.66, 0.46)], line);
        }
        FileKind::Video => {
            let play = vec![
                at(rect, 0.38, 0.45),
                at(rect, 0.68, 0.65),
                at(rect, 0.38, 0.85),
            ];
            painter.add(Shape::convex_polygon(play, mark, Stroke::NONE));
        }
        FileKind::Folder | FileKind::Other => {}
    }
}

/// A small green dot in the bottom-right corner.
fn paint_executable_badge(painter: &egui::Painter, rect: Rect, theme: &Theme) {
    let center = at(rect, 0.8, 0.8);
    let radius = rect.width() * 0.2;
    painter.circle_filled(center, radius, theme.surface);
    painter.circle_filled(center, radius * 0.75, theme.green);
}

/// A small box with an arrow in the bottom-left corner, like Windows shortcuts.
fn paint_link_badge(painter: &egui::Painter, rect: Rect, fill: Color32, arrow: Color32) {
    let badge = Rect::from_min_max(at(rect, 0.0, 0.55), at(rect, 0.45, 1.0));
    painter.rect_filled(badge, 1.5, fill);
    let stroke = Stroke::new((rect.height() / 16.0).max(1.0), arrow);
    let tip = at(badge, 0.75, 0.25);
    painter.line_segment([at(badge, 0.25, 0.75), tip], stroke);
    painter.line_segment([at(badge, 0.4, 0.25), tip], stroke);
    painter.line_segment([at(badge, 0.75, 0.6), tip], stroke);
}
