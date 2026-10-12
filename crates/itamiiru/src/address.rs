//! The address bar: clickable breadcrumbs, which turn into a text
//! field on Ctrl+L, Alt+D, F4 or a click on the empty space beside them.

use std::path::{Path, PathBuf};

use eframe::egui::{self, Key, Sense, StrokeKind, TextStyle};

use crate::icons::{Arrow, paint_arrow};
use crate::theme::{self, Theme};

/// Space kept free right of the crumbs, so there's always somewhere to
/// click to start typing.
const MIN_CLICK_SPACE: f32 = 48.0;
/// Width of the arrow between crumbs, including its margins.
const SEPARATOR_WIDTH: f32 = 14.0;
const THIS_PC: &str = "This PC";

pub enum AddressAction {
    /// A crumb was clicked.
    GoTo(PathBuf),
    ThisPc,
    /// Enter was pressed in the text field; the app parses the text.
    Typed(String),
}

#[derive(Default)]
pub struct AddressBar {
    editing: bool,
    text: String,
    /// The text field still needs focus and its text selected.
    needs_focus: bool,
}

impl AddressBar {
    /// Switches to the text field, showing `current` (None means This PC).
    pub fn start_editing(&mut self, current: Option<&Path>) {
        self.editing = true;
        self.needs_focus = true;
        self.text = match current {
            Some(path) => path.to_string_lossy().into_owned(),
            None => THIS_PC.to_string(),
        };
    }

    pub fn show(
        &mut self,
        ui: &mut egui::Ui,
        current: Option<&Path>,
        theme: &Theme,
    ) -> Option<AddressAction> {
        if self.editing {
            return self.text_field(ui);
        }
        return self.breadcrumbs(ui, current, theme);
    }

    fn text_field(&mut self, ui: &mut egui::Ui) -> Option<AddressAction> {
        let id = ui.id().with("address-edit");
        let edit = egui::TextEdit::singleline(&mut self.text)
            .id(id)
            .desired_width(f32::INFINITY);
        let response = ui.add(edit);
        if self.needs_focus {
            self.needs_focus = false;
            response.request_focus();
            select_all(ui.ctx(), id, &self.text);
        }
        if response.lost_focus() {
            // Enter navigates; Escape or clicking away just goes back to crumbs.
            self.editing = false;
            if ui.input(|i| i.key_pressed(Key::Enter)) {
                return Some(AddressAction::Typed(self.text.clone()));
            }
        }
        return None;
    }

    fn breadcrumbs(
        &mut self,
        ui: &mut egui::Ui,
        current: Option<&Path>,
        theme: &Theme,
    ) -> Option<AddressAction> {
        let mut action = None;
        let height = ui.text_style_height(&TextStyle::Body) + 2.0 * ui.spacing().button_padding.y;
        let size = egui::vec2(ui.available_width(), height);
        let (frame_rect, _) = ui.allocate_exact_size(size, Sense::hover());

        // Looks like the text field it turns into (ponkotsu's `input-like`).
        let visuals = ui.visuals().widgets.inactive;
        ui.painter().rect(
            frame_rect,
            theme::RADIUS_SMALL,
            visuals.bg_fill,
            visuals.bg_stroke,
            StrokeKind::Inside,
        );

        let mut crumbs = vec![(THIS_PC.to_string(), None)];
        if let Some(path) = current {
            for (label, crumb_path) in crumbs_for(path) {
                crumbs.push((label, Some(crumb_path)));
            }
        }

        // Measure, then fold crumbs from the left into "..." until they fit.
        let padding = ui.spacing().button_padding.x;
        let font = TextStyle::Body.resolve(ui.style());
        let mut widths = Vec::with_capacity(crumbs.len());
        for (label, _) in &crumbs {
            let galley = ui
                .painter()
                .layout_no_wrap(label.clone(), font.clone(), theme.on_surface);
            widths.push(galley.size().x + 2.0 * padding);
        }
        let overflow_width = ui
            .painter()
            .layout_no_wrap("...".to_string(), font, theme.on_surface)
            .size()
            .x
            + 2.0 * padding
            + SEPARATOR_WIDTH;
        let available = frame_rect.width() - MIN_CLICK_SPACE;
        let first = first_visible(&widths, SEPARATOR_WIDTH, overflow_width, available);

        let inner = frame_rect.shrink2(egui::vec2(theme::GAP_SMALL, 0.0));
        let mut child = ui.new_child(
            egui::UiBuilder::new()
                .max_rect(inner)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
        );
        child.spacing_mut().item_spacing.x = 0.0;

        if first > 0 {
            child.menu_button("...", |ui| {
                // Hidden crumbs, nearest first, like Explorer's list.
                for (label, path) in crumbs[..first].iter().rev() {
                    if ui.button(label).clicked() {
                        action = Some(crumb_action(path));
                        ui.close();
                    }
                }
            });
            separator(&mut child);
        }
        for (i, (label, path)) in crumbs.iter().enumerate().skip(first) {
            let button = egui::Button::new(label.as_str()).frame_when_inactive(false);
            let response = child.add(button);
            if let Some(path) = path {
                response.clone().on_hover_text(path.to_string_lossy());
            }
            if response.clicked() {
                action = Some(crumb_action(path));
            }
            if i + 1 < crumbs.len() {
                separator(&mut child);
            }
        }

        // The rest of the bar: click to type a path.
        let rest = child.available_rect_before_wrap();
        let background = child.allocate_rect(rest, Sense::click());
        if background.on_hover_cursor(egui::CursorIcon::Text).clicked() {
            self.start_editing(current);
        }
        return action;
    }
}

fn crumb_action(path: &Option<PathBuf>) -> AddressAction {
    return match path {
        Some(path) => AddressAction::GoTo(path.clone()),
        None => AddressAction::ThisPc,
    };
}

/// A small right-pointing arrow between crumbs.
fn separator(ui: &mut egui::Ui) {
    let size = egui::vec2(SEPARATOR_WIDTH, ui.available_height());
    let (rect, _) = ui.allocate_exact_size(size, Sense::hover());
    let arrow = egui::Rect::from_center_size(rect.center(), egui::vec2(6.0, 6.0));
    paint_arrow(
        ui.painter(),
        arrow,
        Arrow::Right,
        ui.visuals().weak_text_color(),
    );
}

fn select_all(ctx: &egui::Context, id: egui::Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    let range = CCursorRange::two(CCursor::new(0), CCursor::new(text.chars().count()));
    state.cursor.set_char_range(Some(range));
    state.store(ctx, id);
}

/// `/home/me/docs` -> [("/", /), ("home", /home), ("me", /home/me), ("docs", /home/me/docs)].
pub fn crumbs_for(path: &Path) -> Vec<(String, PathBuf)> {
    let mut crumbs = Vec::new();
    for ancestor in path.ancestors() {
        let label = match ancestor.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            // The root ("/", or "C:\" on Windows).
            None => ancestor.to_string_lossy().into_owned(),
        };
        if label.is_empty() {
            continue;
        }
        crumbs.push((label, ancestor.to_path_buf()));
    }
    crumbs.reverse();
    return crumbs;
}

/// How many crumbs from the left to fold into the "..." menu so the rest
/// fit in `available`. The last crumb always stays.
pub fn first_visible(widths: &[f32], separator: f32, overflow_width: f32, available: f32) -> usize {
    if widths.is_empty() {
        return 0;
    }
    let mut total = 0.0;
    for width in widths {
        total += width + separator;
    }
    let mut first = 0;
    while first < widths.len() - 1 {
        let needed = if first > 0 {
            total + overflow_width
        } else {
            total
        };
        if needed <= available {
            break;
        }
        total -= widths[first] + separator;
        first += 1;
    }
    return first;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn crumbs() {
        let crumbs = crumbs_for(Path::new("/home/me/docs"));
        let labels: Vec<&str> = crumbs.iter().map(|(l, _)| l.as_str()).collect();
        assert_eq!(labels, ["/", "home", "me", "docs"]);
        assert_eq!(crumbs[2].1, Path::new("/home/me"));
        assert_eq!(crumbs_for(Path::new("/")).len(), 1);
    }

    #[test]
    fn folding() {
        let widths = [50.0, 50.0, 50.0, 50.0];
        // Everything fits: 4 * (50 + 10) = 240.
        assert_eq!(first_visible(&widths, 10.0, 30.0, 240.0), 0);
        // One too narrow: dropping the first saves 60 but "..." costs 30.
        assert_eq!(first_visible(&widths, 10.0, 30.0, 239.0), 1);
        assert_eq!(first_visible(&widths, 10.0, 30.0, 150.0), 2);
        // Never fold the last crumb, even if it doesn't fit.
        assert_eq!(first_visible(&widths, 10.0, 30.0, 1.0), 3);
        assert_eq!(first_visible(&[], 10.0, 30.0, 100.0), 0);
    }
}
