//! Drag to select: dragging from empty space draws a box, and the items
//! it touches get selected. Ctrl (Cmd on macOS) adds them to the
//! selection instead. The `drag_select` setting turns it off.
//!
//! The start is kept relative to the view's content, not the screen, so
//! scrolling during a drag (the wheel, or holding the pointer past an
//! edge) keeps it where it was. It lives in egui's memory under the
//! view's id until the button is released.

use eframe::egui::{self, Pos2, Rect, StrokeKind, Vec2};

use crate::theme::{self, Theme};

/// How fast holding the pointer past an edge scrolls: points per second
/// for each point the pointer is past it.
const SCROLL_SPEED: f32 = 10.0;

/// Starts a band where the pointer was pressed. `origin` is where the
/// view's content starts on screen.
pub fn start(ctx: &egui::Context, id: egui::Id, origin: Pos2) {
    let Some(press) = ctx.input(|i| i.pointer.press_origin()) else {
        return;
    };
    let start: Vec2 = press - origin;
    ctx.data_mut(|d| d.insert_temp(id, start));
}

/// The band in content coordinates while a drag goes on. Ends it once
/// the button is up.
pub fn current(ctx: &egui::Context, id: egui::Id, origin: Pos2) -> Option<Rect> {
    let start: Vec2 = ctx.data(|d| d.get_temp(id))?;
    let (down, pointer) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
    if !down {
        ctx.data_mut(|d| d.remove::<Vec2>(id));
        return None;
    }
    let pointer = pointer?;
    let end = pointer - origin;
    return Some(Rect::from_two_pos(start.to_pos2(), end.to_pos2()));
}

/// While a band is being drawn and the pointer is past an edge of the
/// scroll area `ui` is in, scrolls toward it.
pub fn auto_scroll(ui: &egui::Ui, id: egui::Id) {
    if ui.data(|d| d.get_temp::<Vec2>(id)).is_none() {
        return;
    }
    let Some(pointer) = ui.input(|i| i.pointer.latest_pos()) else {
        return;
    };
    let clip = ui.clip_rect();
    // Positive scrolls toward the start (up, or left).
    let mut past = Vec2::ZERO;
    if pointer.y < clip.top() {
        past.y = clip.top() - pointer.y;
    } else if pointer.y > clip.bottom() {
        past.y = clip.bottom() - pointer.y;
    }
    if pointer.x < clip.left() {
        past.x = clip.left() - pointer.x;
    } else if pointer.x > clip.right() {
        past.x = clip.right() - pointer.x;
    }
    if past == Vec2::ZERO {
        return;
    }
    let dt = ui.input(|i| i.stable_dt).min(0.1);
    ui.scroll_with_delta(past * SCROLL_SPEED * dt);
    ui.ctx().request_repaint();
}

/// Draws the band (content coordinates) as a tinted box.
pub fn paint(painter: &egui::Painter, band: Rect, origin: Pos2, theme: &Theme) {
    let rect = band.translate(origin.to_vec2());
    painter.rect(
        rect,
        theme::RADIUS_SMALL,
        theme.primary.gamma_multiply(0.2),
        egui::Stroke::new(1.0, theme.primary),
        StrokeKind::Inside,
    );
}

/// The rows of a list `step` apart that a band (content coordinates,
/// row 0 at y = 0) touches.
pub fn rows_in(band: Rect, step: f32, count: usize) -> Vec<usize> {
    let mut rows = Vec::new();
    if count == 0 || step <= 0.0 || band.max.y < 0.0 {
        return rows;
    }
    let first = (band.min.y.max(0.0) / step).floor() as usize;
    let last = ((band.max.y / step).floor() as usize).min(count - 1);
    for row in first..=last {
        rows.push(row);
    }
    return rows;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn band(top: f32, bottom: f32) -> Rect {
        return Rect::from_min_max(egui::pos2(0.0, top), egui::pos2(50.0, bottom));
    }

    #[test]
    fn rows_touched() {
        assert_eq!(rows_in(band(5.0, 15.0), 20.0, 10), [0]);
        assert_eq!(rows_in(band(15.0, 45.0), 20.0, 10), [0, 1, 2]);
        // Started above the first row, or below the last.
        assert_eq!(rows_in(band(-30.0, 25.0), 20.0, 10), [0, 1]);
        assert_eq!(rows_in(band(170.0, 900.0), 20.0, 10), [8, 9]);
        // Entirely outside the rows.
        assert!(rows_in(band(-30.0, -5.0), 20.0, 10).is_empty());
        assert!(rows_in(band(250.0, 300.0), 20.0, 10).is_empty());
        assert!(rows_in(band(5.0, 15.0), 20.0, 0).is_empty());
    }
}
