//! Tabs and split view. A tab holds one pane, or two side by side; the
//! toolbar, keys and sidebar act on the active pane of the active tab.

use eframe::egui::{self, CornerRadius, Rect, Sense, Stroke, TextStyle};

use crate::icons::{ButtonIcon, icon_button};
use crate::pane::Pane;
use crate::theme::{self, Theme};

/// At most this many panes side by side.
pub const MAX_PANES: usize = 2;
/// Longer folder names are cut short on the tab.
const MAX_TITLE_CHARS: usize = 24;
/// A tab is at least this wide, so short names still look like tabs.
const MIN_TAB_WIDTH: f32 = 120.0;
/// Split view: neither pane gets narrower than this, in points.
pub const MIN_PANE_WIDTH: f32 = 200.0;

pub struct Tab {
    /// One pane, or two in split view.
    pub panes: Vec<Pane>,
    /// The pane keys and the toolbar act on.
    pub active: usize,
    /// Split view: the left pane's share of the width, moved by dragging
    /// the handle between the panes.
    pub split_ratio: f32,
}

impl Tab {
    pub fn new(pane: Pane) -> Tab {
        return Tab {
            panes: vec![pane],
            active: 0,
            split_ratio: 0.5,
        };
    }

    pub fn pane(&self) -> &Pane {
        return &self.panes[self.active];
    }

    pub fn pane_mut(&mut self) -> &mut Pane {
        return &mut self.panes[self.active];
    }

    pub fn is_split(&self) -> bool {
        return self.panes.len() > 1;
    }

    /// Turns split view off, keeping the active pane.
    pub fn unsplit(&mut self) {
        if self.panes.len() > 1 {
            let pane = self.panes.remove(self.active);
            self.panes = vec![pane];
            self.active = 0;
        }
    }

    /// F6: the other pane becomes active.
    pub fn switch_pane(&mut self) {
        self.active = (self.active + 1) % self.panes.len();
    }
}

pub enum TabAction {
    Select(usize),
    Close(usize),
    New,
}

/// The row of tabs, with a close button on each (when there's more than
/// one) and a new-tab button at the end. Middle-click closes a tab. The
/// active tab has the toolbar's color and touches the panel's bottom
/// edge, so it joins the toolbar below like a browser tab.
pub fn tab_bar(ui: &mut egui::Ui, tabs: &[Tab], active: usize, theme: &Theme) -> Option<TabAction> {
    let mut action = None;
    egui::ScrollArea::horizontal()
        .id_salt("tab-bar")
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = theme::GAP_SMALL;
                for (i, tab) in tabs.iter().enumerate() {
                    let closable = tabs.len() > 1;
                    let (response, close) =
                        tab_button(ui, tab.pane(), i == active, closable, theme);
                    if close || (response.middle_clicked() && closable) {
                        action = Some(TabAction::Close(i));
                    } else if response.clicked() {
                        action = Some(TabAction::Select(i));
                    }
                }
                let new = icon_button(ui, ButtonIcon::Plus, false);
                if new.on_hover_text("New tab (Ctrl+T)").clicked() {
                    action = Some(TabAction::New);
                }
            });
        });
    return action;
}

/// One tab: the folder name and a close button. Returns the tab's
/// response, and whether the close button was clicked.
fn tab_button(
    ui: &mut egui::Ui,
    pane: &Pane,
    active: bool,
    closable: bool,
    theme: &Theme,
) -> (egui::Response, bool) {
    let font = TextStyle::Button.resolve(ui.style());
    let text_color = if active {
        theme.on_surface
    } else {
        theme.on_surface_variant
    };
    let galley = ui
        .painter()
        .layout_no_wrap(short_title(&pane.title()), font, text_color);
    let padding = theme::PADDING as f32;
    let close_side = theme::ICON_SIZE;
    let mut width = padding + galley.size().x + padding;
    if closable {
        width += close_side;
    }
    let height = galley.size().y + 2.0 * theme::GAP;
    let size = egui::vec2(width.max(MIN_TAB_WIDTH), height);
    let (rect, response) = ui.allocate_exact_size(size, Sense::click());
    let full_path = if pane.this_pc {
        "This PC".to_string()
    } else {
        pane.current.to_string_lossy().into_owned()
    };
    let response = response.on_hover_text(full_path);

    // Registered after the tab, so it gets the clicks on top of it.
    let mut close_clicked = false;
    let mut close_hovered = false;
    let close_rect = Rect::from_center_size(
        egui::pos2(
            rect.right() - padding / 2.0 - close_side / 2.0,
            rect.center().y,
        ),
        egui::vec2(close_side, close_side),
    );
    if closable {
        let close = ui.interact(close_rect, response.id.with("close"), Sense::click());
        close_hovered = close.hovered();
        close_clicked = close.on_hover_text("Close tab (Ctrl+W)").clicked();
    }

    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        if active {
            // Rounded on top only: the bottom runs into the toolbar.
            let corners = CornerRadius {
                nw: theme::RADIUS_MEDIUM,
                ne: theme::RADIUS_MEDIUM,
                sw: 0,
                se: 0,
            };
            painter.rect_filled(rect, corners, theme.surface_container_1);
        } else if response.hovered() {
            let pill = rect.shrink2(egui::vec2(0.0, theme::GAP_SMALL));
            painter.rect_filled(pill, theme::RADIUS_MEDIUM, theme.surface_container_1);
        }
        let text_pos = egui::pos2(
            rect.left() + padding,
            rect.center().y - galley.size().y / 2.0,
        );
        painter
            .with_clip_rect(rect)
            .galley(text_pos, galley, text_color);
        if closable {
            if close_hovered {
                painter.rect_filled(close_rect, theme::RADIUS_SMALL, theme.surface_container_3);
            }
            let cross = close_rect.shrink(close_side * 0.3);
            let stroke = Stroke::new(1.5, text_color);
            painter.line_segment([cross.left_top(), cross.right_bottom()], stroke);
            painter.line_segment([cross.right_top(), cross.left_bottom()], stroke);
        }
    }
    return (response, close_clicked);
}

/// The title cut to MAX_TITLE_CHARS, with "..." when it was cut.
pub fn short_title(title: &str) -> String {
    let count = title.chars().count();
    if count <= MAX_TITLE_CHARS {
        return title.to_string();
    }
    let mut short: String = title.chars().take(MAX_TITLE_CHARS - 3).collect();
    short.push_str("...");
    return short;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_titles_are_cut() {
        assert_eq!(short_title("Downloads"), "Downloads");
        let long = "a".repeat(40);
        let short = short_title(&long);
        assert_eq!(short.chars().count(), MAX_TITLE_CHARS);
        assert!(short.ends_with("..."));
        // Cut by characters, not bytes, so it never splits one.
        let accented = "\u{e9}".repeat(40);
        assert_eq!(short_title(&accented).chars().count(), MAX_TITLE_CHARS);
    }
}
