//! Small icons painted with egui's painter, so there are no icon fonts or
//! image files to ship.

use eframe::egui::{self, Color32, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, TextStyle};
use itamiiru_core::entry::{Entry, EntryKind};
use itamiiru_core::filekind::{FileKind, file_kind};
use itamiiru_core::places::{DriveKind, PlaceKind};

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

/// The eject symbol: a triangle pointing up over a bar, in `rect`.
pub fn paint_eject(painter: &egui::Painter, rect: Rect, color: Color32) {
    let w = rect.width().min(rect.height());
    let c = rect.center();
    let left = c.x - w / 2.0;
    let right = c.x + w / 2.0;
    let top = c.y - w / 2.0;
    let triangle_bottom = top + w * 0.6;
    let points = vec![
        Pos2::new(c.x, top),
        Pos2::new(right, triangle_bottom),
        Pos2::new(left, triangle_bottom),
    ];
    painter.add(Shape::convex_polygon(points, color, Stroke::NONE));
    let bar = Rect::from_min_max(
        Pos2::new(left, top + w * 0.75),
        Pos2::new(right, top + w * 0.9),
    );
    painter.rect_filled(bar, 0.0, color);
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ButtonIcon {
    /// A magnifying glass.
    Search,
    /// A plus, for a new tab or new item.
    Plus,
    /// A funnel, for filtering names.
    Filter,
    /// A circular arrow, for refreshing.
    Refresh,
    /// A gear, for the settings page.
    Settings,
    /// Scissors, for Cut.
    Cut,
    /// Two overlapping sheets, for Copy.
    Copy,
    /// Clipboard with document, for Paste.
    Paste,
    /// Diagonal pencil with baseline, for Rename.
    Rename,
    /// Trash bin with lid, for Delete.
    Delete,
    /// Three horizontal lines with arrow, for Sort.
    Sort,
    /// 2x2 grid, for View.
    View,
    /// Split window, for Preview pane.
    PreviewPane,
    /// Three rows with bullets, for Details view.
    DetailsView,
    /// Two large tiles, for Large icons view.
    LargeIconsView,
}

/// Paints the given button icon into `area` using `color`.
pub fn paint_button_icon(painter: &egui::Painter, area: Rect, icon: ButtonIcon, color: Color32) {
    let stroke = Stroke::new(1.5, color);
    match icon {
        ButtonIcon::Search => {
            let radius = area.width() * 0.32;
            let center = area.left_top() + egui::vec2(radius, radius);
            painter.circle_stroke(center, radius, stroke);
            let start = center + egui::vec2(radius, radius) * 0.7;
            painter.line_segment([start, area.right_bottom()], stroke);
        }
        ButtonIcon::Plus => {
            let c = area.center();
            painter.line_segment(
                [Pos2::new(area.left(), c.y), Pos2::new(area.right(), c.y)],
                stroke,
            );
            painter.line_segment(
                [Pos2::new(c.x, area.top()), Pos2::new(c.x, area.bottom())],
                stroke,
            );
        }
        ButtonIcon::Refresh => {
            let center = area.center();
            let radius = area.width() * 0.45;
            let start = 0.35 * std::f32::consts::PI;
            let end = 1.85 * std::f32::consts::PI;
            let steps = 20;
            let mut points = Vec::new();
            for i in 0..=steps {
                let angle = start + (end - start) * i as f32 / steps as f32;
                points.push(center + radius * egui::vec2(angle.cos(), angle.sin()));
            }
            let tip = points[steps];
            painter.add(Shape::line(points, stroke));
            let size = area.width() * 0.3;
            let head = vec![
                tip + egui::vec2(size * 0.6, 0.0),
                tip + egui::vec2(-size * 0.4, -size * 0.7),
                tip + egui::vec2(-size * 0.4, size * 0.5),
            ];
            painter.add(Shape::convex_polygon(head, color, Stroke::NONE));
        }
        ButtonIcon::Settings => {
            let center = area.center();
            let outer = area.width() * 0.5;
            let inner = area.width() * 0.3;
            for i in 0..8 {
                let angle = i as f32 * std::f32::consts::PI / 4.0;
                let direction = egui::vec2(angle.cos(), angle.sin());
                painter.line_segment(
                    [center + direction * inner, center + direction * outer],
                    Stroke::new(stroke.width * 1.6, color),
                );
            }
            painter.circle_stroke(center, inner, stroke);
            painter.circle_stroke(center, inner * 0.35, stroke);
        }
        ButtonIcon::Filter => {
            let c = area.center().x;
            let stem = area.width() * 0.12;
            let neck = area.top() + area.height() * 0.5;
            let points = vec![
                area.left_top(),
                area.right_top(),
                Pos2::new(c + stem, neck),
                Pos2::new(c + stem, area.bottom()),
                Pos2::new(c - stem, area.bottom() - area.height() * 0.15),
                Pos2::new(c - stem, neck),
            ];
            painter.add(Shape::closed_line(points, stroke));
        }
        ButtonIcon::Cut => {
            let r = area.width() * 0.16;
            let p_left = Pos2::new(area.left() + r, area.bottom() - r);
            let p_right = Pos2::new(area.right() - r, area.bottom() - r);
            painter.circle_stroke(p_left, r, stroke);
            painter.circle_stroke(p_right, r, stroke);
            let pivot = Pos2::new(area.center().x, area.center().y + area.height() * 0.08);
            painter.line_segment([p_left + egui::vec2(r * 0.7, -r * 0.7), pivot], stroke);
            painter.line_segment([p_right + egui::vec2(-r * 0.7, -r * 0.7), pivot], stroke);
            painter.line_segment(
                [
                    pivot,
                    Pos2::new(area.right() - r * 0.4, area.top() + r * 0.4),
                ],
                stroke,
            );
            painter.line_segment(
                [
                    pivot,
                    Pos2::new(area.left() + r * 0.4, area.top() + r * 0.4),
                ],
                stroke,
            );
        }
        ButtonIcon::Copy => {
            let back = Rect::from_min_max(area.left_top(), at(area, 0.72, 0.72));
            painter.rect_stroke(back, 1.0, stroke, StrokeKind::Inside);
            let front = Rect::from_min_max(at(area, 0.28, 0.28), area.right_bottom());
            painter.rect_filled(front, 1.0, color.gamma_multiply(0.2));
            painter.rect_stroke(front, 1.0, stroke, StrokeKind::Inside);
        }
        ButtonIcon::Paste => {
            let board = Rect::from_min_max(at(area, 0.15, 0.18), at(area, 0.85, 0.95));
            painter.rect_stroke(board, 1.0, stroke, StrokeKind::Inside);
            let clip = Rect::from_min_max(at(area, 0.35, 0.08), at(area, 0.65, 0.26));
            painter.rect_filled(clip, 1.0, color);
            let doc = Rect::from_min_max(at(area, 0.28, 0.34), at(area, 0.72, 0.84));
            painter.line_segment([at(doc, 0.2, 0.35), at(doc, 0.8, 0.35)], stroke);
            painter.line_segment([at(doc, 0.2, 0.65), at(doc, 0.8, 0.65)], stroke);
        }
        ButtonIcon::Rename => {
            let p1 = at(area, 0.18, 0.82);
            let p2 = at(area, 0.82, 0.18);
            painter.line_segment([p1, p2], Stroke::new(stroke.width * 2.0, color));
            painter.line_segment([at(area, 0.1, 0.92), at(area, 0.9, 0.92)], stroke);
        }
        ButtonIcon::Delete => {
            painter.line_segment([at(area, 0.15, 0.25), at(area, 0.85, 0.25)], stroke);
            let handle = Rect::from_min_max(at(area, 0.40, 0.12), at(area, 0.60, 0.25));
            painter.rect_stroke(handle, 1.0, stroke, StrokeKind::Inside);
            let body = vec![
                at(area, 0.22, 0.25),
                at(area, 0.78, 0.25),
                at(area, 0.72, 0.92),
                at(area, 0.28, 0.92),
            ];
            painter.add(Shape::closed_line(body, stroke));
            painter.line_segment([at(area, 0.42, 0.38), at(area, 0.42, 0.80)], stroke);
            painter.line_segment([at(area, 0.58, 0.38), at(area, 0.58, 0.80)], stroke);
        }
        ButtonIcon::Sort => {
            painter.line_segment([at(area, 0.1, 0.25), at(area, 0.60, 0.25)], stroke);
            painter.line_segment([at(area, 0.1, 0.50), at(area, 0.45, 0.50)], stroke);
            painter.line_segment([at(area, 0.1, 0.75), at(area, 0.30, 0.75)], stroke);
            let ax = area.right() - area.width() * 0.18;
            painter.line_segment(
                [
                    Pos2::new(ax, area.top() + area.height() * 0.2),
                    Pos2::new(ax, area.bottom() - area.height() * 0.2),
                ],
                stroke,
            );
            let tip = Pos2::new(ax, area.bottom() - area.height() * 0.2);
            painter.line_segment(
                [
                    Pos2::new(ax - area.width() * 0.12, tip.y - area.height() * 0.18),
                    tip,
                ],
                stroke,
            );
            painter.line_segment(
                [
                    Pos2::new(ax + area.width() * 0.12, tip.y - area.height() * 0.18),
                    tip,
                ],
                stroke,
            );
        }
        ButtonIcon::View => {
            let sz = 0.34;
            painter.rect_stroke(
                Rect::from_min_size(
                    at(area, 0.10, 0.10),
                    egui::vec2(area.width() * sz, area.height() * sz),
                ),
                1.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_size(
                    at(area, 0.56, 0.10),
                    egui::vec2(area.width() * sz, area.height() * sz),
                ),
                1.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_size(
                    at(area, 0.10, 0.56),
                    egui::vec2(area.width() * sz, area.height() * sz),
                ),
                1.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_size(
                    at(area, 0.56, 0.56),
                    egui::vec2(area.width() * sz, area.height() * sz),
                ),
                1.0,
                stroke,
                StrokeKind::Inside,
            );
        }
        ButtonIcon::PreviewPane => {
            let win = Rect::from_min_max(at(area, 0.08, 0.15), at(area, 0.92, 0.85));
            painter.rect_stroke(win, 1.0, stroke, StrokeKind::Inside);
            let split_x = win.left() + win.width() * 0.65;
            painter.line_segment(
                [
                    Pos2::new(split_x, win.top()),
                    Pos2::new(split_x, win.bottom()),
                ],
                stroke,
            );
            let pane_rect = Rect::from_min_max(Pos2::new(split_x, win.top()), win.right_bottom());
            painter.rect_filled(pane_rect, 0.0, color.gamma_multiply(0.25));
        }
        ButtonIcon::DetailsView => {
            for fy in [0.25, 0.50, 0.75] {
                painter.rect_filled(
                    Rect::from_center_size(at(area, 0.18, fy), egui::vec2(2.5, 2.5)),
                    0.5,
                    color,
                );
                painter.line_segment([at(area, 0.35, fy), at(area, 0.90, fy)], stroke);
            }
        }
        ButtonIcon::LargeIconsView => {
            let h = area.height() * 0.65;
            let w = area.width() * 0.36;
            painter.rect_stroke(
                Rect::from_min_size(at(area, 0.10, 0.18), egui::vec2(w, h)),
                1.0,
                stroke,
                StrokeKind::Inside,
            );
            painter.rect_stroke(
                Rect::from_min_size(at(area, 0.54, 0.18), egui::vec2(w, h)),
                1.0,
                stroke,
                StrokeKind::Inside,
            );
        }
    }
}

/// A square button like `arrow_button`, with a painted icon. `selected`
/// draws it pressed, for toggles like the search button.
pub fn icon_button(ui: &mut egui::Ui, icon: ButtonIcon, selected: bool) -> egui::Response {
    let side = ui.text_style_height(&TextStyle::Button) + 2.0 * ui.spacing().button_padding.y;
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), Sense::click());
    if ui.is_rect_visible(rect) {
        let visuals = ui.style().interact_selectable(&response, selected);
        let painter = ui.painter();
        painter.rect(
            rect,
            visuals.corner_radius,
            visuals.weak_bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );
        let color = visuals.fg_stroke.color;
        let area = rect.shrink(side * 0.3);
        paint_button_icon(painter, area, icon, color);
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

pub fn paint_folder(painter: &egui::Painter, rect: Rect, theme: &Theme) {
    let tab = Rect::from_min_max(at(rect, 0.05, 0.12), at(rect, 0.45, 0.35));
    painter.rect_filled(tab, 2.0, theme.d_yellow);
    let back = Rect::from_min_max(at(rect, 0.05, 0.22), at(rect, 0.95, 0.85));
    painter.rect_filled(back, 2.5, theme.d_yellow);
    let inside = Rect::from_min_max(at(rect, 0.12, 0.25), at(rect, 0.88, 0.50));
    painter.rect_filled(inside, 1.5, theme.l_yellow);
    let body = Rect::from_min_max(at(rect, 0.05, 0.32), at(rect, 0.95, 0.88));
    painter.rect_filled(body, 2.5, theme.yellow);
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

/// Icons for the navigation pane.
pub fn paint_place_icon(painter: &egui::Painter, rect: Rect, kind: PlaceKind, theme: &Theme) {
    match kind {
        PlaceKind::Home => paint_home(painter, rect, theme),
        PlaceKind::Folder | PlaceKind::Bookmark => paint_folder(painter, rect, theme),
        PlaceKind::Network => paint_drive(painter, rect, theme.blue, theme.on_blue),
    }
}

pub fn paint_drive_icon(painter: &egui::Painter, rect: Rect, kind: DriveKind, theme: &Theme) {
    let (body, light) = match kind {
        DriveKind::System => (theme.on_surface_variant, theme.green),
        DriveKind::Removable => (theme.outline, theme.yellow),
        DriveKind::Network => (theme.blue, theme.on_blue),
    };
    paint_drive(painter, rect, body, light);
}

/// Icons for places in the navigation pane. Recognized names get their
/// dedicated Windows 11 category icon.
pub fn paint_quick_access_icon(
    painter: &egui::Painter,
    rect: Rect,
    name: &str,
    kind: PlaceKind,
    theme: &Theme,
) {
    if kind == PlaceKind::Home || name.eq_ignore_ascii_case("Home") {
        paint_home(painter, rect, theme);
        return;
    }
    if name.eq_ignore_ascii_case("Desktop") {
        let screen = Rect::from_min_max(at(rect, 0.12, 0.15), at(rect, 0.88, 0.70));
        painter.rect_filled(screen, 2.0, theme.blue);
        let inner = Rect::from_min_max(at(rect, 0.18, 0.21), at(rect, 0.82, 0.64));
        painter.rect_filled(inner, 1.0, theme.surface);
        let stand = Stroke::new((rect.height() / 12.0).max(1.5), theme.on_surface_variant);
        painter.line_segment([at(rect, 0.5, 0.70), at(rect, 0.5, 0.85)], stand);
        painter.line_segment([at(rect, 0.35, 0.85), at(rect, 0.65, 0.85)], stand);
        return;
    }
    if name.eq_ignore_ascii_case("Downloads") {
        paint_folder(painter, rect, theme);
        let badge = Rect::from_min_max(at(rect, 0.45, 0.45), at(rect, 0.95, 0.95));
        painter.circle_filled(badge.center(), badge.width() * 0.45, theme.blue);
        let tip = badge.center() + egui::vec2(0.0, badge.height() * 0.25);
        let stroke = Stroke::new(1.5, theme.on_blue);
        painter.line_segment(
            [badge.center() - egui::vec2(0.0, badge.height() * 0.25), tip],
            stroke,
        );
        painter.line_segment(
            [
                tip - egui::vec2(badge.width() * 0.2, badge.height() * 0.2),
                tip,
            ],
            stroke,
        );
        painter.line_segment(
            [
                tip + egui::vec2(badge.width() * 0.2, -badge.height() * 0.2),
                tip,
            ],
            stroke,
        );
        return;
    }
    if name.eq_ignore_ascii_case("Documents") {
        paint_page(painter, rect, FileKind::Document, theme);
        return;
    }
    if name.eq_ignore_ascii_case("Pictures") {
        paint_page(painter, rect, FileKind::Image, theme);
        return;
    }
    if name.eq_ignore_ascii_case("Music") {
        paint_page(painter, rect, FileKind::Audio, theme);
        return;
    }
    if name.eq_ignore_ascii_case("Videos") {
        paint_page(painter, rect, FileKind::Video, theme);
        return;
    }
    paint_place_icon(painter, rect, kind, theme);
}

/// A house: a roof over a box with a door.
pub fn paint_home(painter: &egui::Painter, rect: Rect, theme: &Theme) {
    let roof = vec![
        at(rect, 0.5, 0.10),
        at(rect, 0.92, 0.46),
        at(rect, 0.08, 0.46),
    ];
    painter.add(Shape::convex_polygon(roof, theme.primary, Stroke::NONE));
    let body = Rect::from_min_max(at(rect, 0.18, 0.44), at(rect, 0.82, 0.90));
    painter.rect_filled(body, 1.5, theme.blue);
    let door = Rect::from_min_max(at(rect, 0.40, 0.60), at(rect, 0.60, 0.90));
    painter.rect_filled(door, 1.0, theme.surface);
}

/// A drive: a flat box with a small status light.
pub fn paint_drive(painter: &egui::Painter, rect: Rect, body: Color32, light: Color32) {
    let case = Rect::from_min_max(at(rect, 0.06, 0.26), at(rect, 0.94, 0.74));
    painter.rect_filled(case, 2.5, body);
    let bar = Rect::from_min_max(at(rect, 0.16, 0.54), at(rect, 0.62, 0.62));
    painter.rect_filled(bar, 1.0, light);
    painter.circle_filled(at(rect, 0.78, 0.58), rect.width() * 0.06, light);
}
