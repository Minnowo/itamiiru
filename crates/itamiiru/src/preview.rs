//! The preview pane, an optional feature: `preview_pane` in the
//! config, off by default; Alt+P or the toolbar checkbox toggles it.
//! With `preview_auto_hide` it only shows while there's a picture or
//! text to show.
//!
//! Shows the one selected item: a large thumbnail for images, the start of
//! the file for text, otherwise a big icon. Then its name, type, size and
//! date, like the Windows details pane.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui::{self, RichText, Sense, TextStyle};
use itamiiru_core::entry::{Entry, EntryKind};
use itamiiru_core::filekind::{FileKind, file_kind};
use itamiiru_core::format::{format_local_time, format_size};
use itamiiru_core::sort::type_label;
use itamiiru_core::textpreview::{self, TextPreview};
use itamiiru_core::thumbcache::ThumbSize;

use crate::icons::paint_entry_icon;
use crate::theme::{self, Theme};
use crate::thumbs::Thumbs;

/// The icon shown when there's nothing better, in points.
const BIG_ICON: f32 = 96.0;

enum Text {
    /// Not a text file, or not asked for.
    None,
    Loading,
    Ready {
        text: String,
        truncated: bool,
    },
}

pub struct Preview {
    /// The item being shown, as it was when it got selected.
    entry: Option<Entry>,
    text: Text,
    /// The thumbnail size asked for, from the pane's width.
    size: ThumbSize,
    /// With auto-hide: the pane was showing last frame.
    was_open: bool,
    tx: Sender<TextPreview>,
    rx: Receiver<TextPreview>,
}

impl Preview {
    pub fn new() -> Preview {
        let (tx, rx) = mpsc::channel();
        return Preview {
            entry: None,
            text: Text::None,
            size: ThumbSize::XLarge,
            was_open: false,
            tx,
            rx,
        };
    }

    /// Follows the selection and asks for its thumbnail. Called every
    /// frame while the pane is on, also while it's hidden, so the app can
    /// tell when there's something to show. `selected` is the one selected
    /// item, if exactly one is; `width` is the pane's width in points.
    pub fn prepare(
        &mut self,
        ctx: &egui::Context,
        entries: &[Entry],
        selected: Option<&Path>,
        thumbs: &mut Thumbs,
        width: f32,
    ) {
        self.follow_selection(ctx, entries, selected);
        self.receive();
        // Bigger than the pane is pointless; x-large (512 px) is plenty.
        let pixels = width * ctx.pixels_per_point();
        self.size = ThumbSize::for_pixels(pixels).min(ThumbSize::XLarge);
        if let Some(entry) = &self.entry {
            thumbs.want(&[entry], self.size);
        }
    }

    /// With auto-hide: whether the pane showed last frame.
    pub fn was_open(&self) -> bool {
        return self.was_open;
    }

    /// With auto-hide: whether the pane shows this frame. It opens once
    /// there's a picture or text to show, not just an icon, and stays open
    /// while the next item's preview loads, so going from one file to the
    /// next doesn't make it flicker.
    pub fn auto_open(&mut self, thumbs: &Thumbs) -> bool {
        let Some(entry) = &self.entry else {
            self.was_open = false;
            return false;
        };
        let ready =
            thumbs.get(entry, self.size).is_some() || matches!(self.text, Text::Ready { .. });
        let loading = thumbs.is_pending(entry, self.size) || matches!(self.text, Text::Loading);
        self.was_open = ready || (self.was_open && loading);
        return self.was_open;
    }

    /// Draws the pane, as of the last `prepare`.
    pub fn show(&self, ui: &mut egui::Ui, thumbs: &Thumbs, theme: &Theme) {
        let Some(entry) = &self.entry else {
            ui.centered_and_justified(|ui| {
                ui.weak("Select a file to preview.");
            });
            return;
        };

        let width = ui.available_width();

        // The details go at the bottom; the preview takes the space above.
        let line = ui.text_style_height(&TextStyle::Body);
        let details_height = 6.0 * line + 4.0 * theme::GAP;
        let preview_height = (ui.available_height() - details_height).max(BIG_ICON);

        if let Some(texture) = thumbs.get(entry, self.size) {
            let [w, h] = texture.size();
            let natural = egui::vec2(w as f32, h as f32) / ui.ctx().pixels_per_point();
            let scale = (width / natural.x).min(preview_height / natural.y).min(1.0);
            let image = egui::Image::new(texture).fit_to_exact_size(natural * scale);
            ui.vertical_centered(|ui| ui.add(image));
        } else if let Text::Ready { text, truncated } = &self.text {
            // The pane is a surface-1 box, so the text sits in surface 2.
            theme.surface(2).show(ui, |ui| {
                egui::ScrollArea::both()
                    .max_height(preview_height - 2.0 * theme::PADDING as f32)
                    .auto_shrink([false, true])
                    .show(ui, |ui| {
                        let mut shown = text.clone();
                        if *truncated {
                            shown.push_str("\n...");
                        }
                        let label = egui::Label::new(RichText::new(shown).monospace())
                            .extend()
                            .selectable(true);
                        ui.add(label);
                    });
            });
        } else {
            ui.vertical_centered(|ui| {
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(BIG_ICON, BIG_ICON), Sense::hover());
                paint_entry_icon(ui.painter(), rect, entry, theme);
            });
        }

        ui.add_space(theme::GAP_LARGE);
        ui.label(RichText::new(entry.name.to_string_lossy()).strong());
        ui.add_space(theme::GAP_SMALL);
        detail(ui, "Type", &type_label(entry));
        if !entry.is_dir() {
            detail(ui, "Size", &format_size(entry.size));
        }
        if let Some(mtime) = entry.mtime {
            detail(ui, "Date modified", &format_local_time(mtime));
        }
        if let Some(created) = entry.created {
            detail(ui, "Date created", &format_local_time(created));
        }
    }

    /// Notices a new selection: remembers its entry and, for text files,
    /// starts reading the start of the file.
    fn follow_selection(
        &mut self,
        ctx: &egui::Context,
        entries: &[Entry],
        selected: Option<&Path>,
    ) {
        let shown: Option<&Path> = match &self.entry {
            Some(entry) => Some(entry.path.as_path()),
            None => None,
        };
        if shown == selected {
            return;
        }
        self.entry = None;
        self.text = Text::None;
        let Some(selected) = selected else {
            return;
        };
        for entry in entries {
            if entry.path == selected {
                self.entry = Some(entry.clone());
                break;
            }
        }
        let Some(entry) = &self.entry else {
            return;
        };
        if wants_text(entry) {
            self.text = Text::Loading;
            let ctx = ctx.clone();
            let wake = Box::new(move || ctx.request_repaint());
            textpreview::start(entry.path.clone(), self.tx.clone(), wake);
        }
    }

    /// Takes the text read on the worker, if it's for the item still shown.
    fn receive(&mut self) {
        while let Ok(result) = self.rx.try_recv() {
            let current: Option<&PathBuf> = match &self.entry {
                Some(entry) => Some(&entry.path),
                None => None,
            };
            if current != Some(&result.path) {
                continue;
            }
            self.text = match result.text {
                Some(text) => Text::Ready {
                    text,
                    truncated: result.truncated,
                },
                None => Text::None,
            };
        }
    }
}

/// Text files by extension, plus files with no known type (README,
/// Makefile, ...). The reader itself rejects anything binary.
fn wants_text(entry: &Entry) -> bool {
    if entry.kind != EntryKind::File && entry.kind != EntryKind::SymlinkToFile {
        return false;
    }
    if entry.size == 0 {
        return false;
    }
    let kind = file_kind(entry);
    return kind == FileKind::Text || kind == FileKind::Other;
}

fn detail(ui: &mut egui::Ui, label: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.weak(label);
        ui.label(value);
    });
}
