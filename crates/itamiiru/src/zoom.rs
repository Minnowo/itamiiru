//! UI zoom controls for the status bar. The keys (Ctrl+Plus, Ctrl+Minus,
//! Ctrl+0) are egui's built-in ones, in 10% steps; these buttons use the
//! same steps. egui's zoom scales text, icons and spacing together.

use eframe::egui;

/// Draws `[-] 110% [+]`. Clicking the percentage resets to 100%.
/// Lay out right to left, so the widgets are added in reverse.
pub fn controls_right_to_left(ui: &mut egui::Ui) {
    let ctx = ui.ctx().clone();
    if ui
        .button("+")
        .on_hover_text("Zoom in (Ctrl+Plus)")
        .clicked()
    {
        egui::gui_zoom::zoom_in(&ctx);
    }
    let reset = ui.button(percent_text(ctx.zoom_factor()));
    if reset.on_hover_text("Reset zoom (Ctrl+0)").clicked() {
        ctx.set_zoom_factor(1.0);
    }
    if ui
        .button("-")
        .on_hover_text("Zoom out (Ctrl+Minus)")
        .clicked()
    {
        egui::gui_zoom::zoom_out(&ctx);
    }
}

/// Works around stale click positions after a zoom change.
///
/// egui-winit remembers the mouse position in points and only updates it
/// when the mouse moves. After a zoom change, a click without moving first
/// is reported at the old position, so it misses what's under the mouse
/// (e.g. clicking a zoom button twice). The stale position is off by
/// exactly (zoom when the mouse last moved) / (zoom now), so we scale
/// clicks by that until the next real mouse move.
pub struct ClickPositionFix {
    zoom_at_last_move: f32,
}

impl Default for ClickPositionFix {
    fn default() -> Self {
        return ClickPositionFix {
            zoom_at_last_move: 1.0,
        };
    }
}

impl ClickPositionFix {
    /// Call with each frame's raw input, before egui sees it.
    pub fn fix(&mut self, current_zoom: f32, raw_input: &mut egui::RawInput) {
        for event in raw_input.events.iter_mut() {
            match event {
                // A fresh position, converted at the current zoom.
                egui::Event::PointerMoved(_) => self.zoom_at_last_move = current_zoom,
                egui::Event::PointerButton { pos, .. }
                    if self.zoom_at_last_move != current_zoom =>
                {
                    *pos *= self.zoom_at_last_move / current_zoom;
                }
                _ => {}
            }
        }
    }
}

/// "110%".
pub fn percent_text(zoom: f32) -> String {
    return format!("{:.0}%", zoom * 100.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn click_at(x: f32, y: f32) -> egui::Event {
        return egui::Event::PointerButton {
            pos: egui::pos2(x, y),
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        };
    }

    fn first_click_pos(raw: &egui::RawInput) -> egui::Pos2 {
        for event in &raw.events {
            if let egui::Event::PointerButton { pos, .. } = event {
                return *pos;
            }
        }
        panic!("no click in the input");
    }

    #[test]
    fn click_after_zoom_is_rescaled() {
        let mut fix = ClickPositionFix::default();
        // Mouse moved at 100%, stored as (100, 50) points.
        let mut raw = egui::RawInput::default();
        raw.events
            .push(egui::Event::PointerMoved(egui::pos2(100.0, 50.0)));
        fix.fix(1.0, &mut raw);

        // Zoomed to 125% without moving: the stale (100, 50) is really (80, 40).
        let mut raw = egui::RawInput::default();
        raw.events.push(click_at(100.0, 50.0));
        fix.fix(1.25, &mut raw);
        assert_eq!(first_click_pos(&raw), egui::pos2(80.0, 40.0));
    }

    #[test]
    fn click_after_a_move_is_untouched() {
        let mut fix = ClickPositionFix::default();
        let mut raw = egui::RawInput::default();
        raw.events
            .push(egui::Event::PointerMoved(egui::pos2(10.0, 10.0)));
        raw.events.push(click_at(10.0, 10.0));
        fix.fix(1.5, &mut raw);
        assert_eq!(first_click_pos(&raw), egui::pos2(10.0, 10.0));
    }

    #[test]
    fn percent() {
        assert_eq!(percent_text(1.0), "100%");
        assert_eq!(percent_text(1.1), "110%");
        assert_eq!(percent_text(0.9), "90%");
    }
}
