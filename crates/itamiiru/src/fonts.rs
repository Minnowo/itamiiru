//! System fallback fonts, so CJK and other non-Latin file names don't show
//! as boxes (PLAN.md section 6, Fonts). egui's built-in fonts already
//! include monochrome emoji.

use eframe::egui::epaint::text::{FontInsert, FontPriority, InsertFontFamily};
use eframe::egui::{self, FontData, FontFamily};

/// Font files bigger than this are skipped. Big CJK collections are ~30 MB.
const MAX_FONT_FILE_SIZE: u64 = 64 * 1024 * 1024;

/// Finds and reads the fallback fonts on a worker thread, then adds them to
/// egui. Until they arrive (a fraction of a second), those glyphs show as boxes.
pub fn load_in_background(ctx: egui::Context) {
    let spawned = std::thread::Builder::new()
        .name("fonts".to_string())
        .spawn(move || load(ctx));
    if let Err(err) = spawned {
        itamiiru_core::debug!("fonts: could not start thread: {err}");
    }
}

fn load(ctx: egui::Context) {
    let mut added = 0;
    for (path, index) in itamiiru_core::platform::fallback_font_files() {
        let size = match std::fs::metadata(&path) {
            Ok(meta) => meta.len(),
            Err(_) => continue,
        };
        if size > MAX_FONT_FILE_SIZE {
            itamiiru_core::debug!("fonts: skipping {} ({size} bytes)", path.display());
            continue;
        }
        let bytes = match std::fs::read(&path) {
            Ok(bytes) => bytes,
            Err(err) => {
                itamiiru_core::debug!("fonts: can't read {}: {err}", path.display());
                continue;
            }
        };
        let mut data = FontData::from_owned(bytes);
        data.index = index;
        // Lowest priority: only used for characters the earlier fonts lack.
        let families = vec![
            InsertFontFamily {
                family: FontFamily::Proportional,
                priority: FontPriority::Lowest,
            },
            InsertFontFamily {
                family: FontFamily::Monospace,
                priority: FontPriority::Lowest,
            },
        ];
        let name = format!("fallback-{}-{}", path.display(), index);
        itamiiru_core::debug!("fonts: adding {name}");
        ctx.add_font(FontInsert::new(&name, data, families));
        added += 1;
    }
    if added > 0 {
        ctx.request_repaint();
    }
}
