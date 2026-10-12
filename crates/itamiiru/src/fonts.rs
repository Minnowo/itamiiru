//! The system's fonts (PLAN.md section 6, Fonts): the text fonts the OS
//! picks (fontconfig's sans-serif and monospace on Linux) become the main
//! ones, and fonts covering CJK and other scripts are added as fallbacks.
//! egui's built-in fonts stay last, for the emoji they include. egui draws
//! text itself, so it can't hand it to the system; it can use the same
//! font files, though.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use eframe::egui::epaint::text::{
    FontDefinitions, FontInsert, FontPriority, Fonts, InsertFontFamily, TextOptions,
};
use eframe::egui::{self, FontData, FontFamily};
use itamiiru_core::platform::FontFile;

/// Font files bigger than this are skipped. Big CJK collections are ~30 MB.
const MAX_FONT_FILE_SIZE: u64 = 64 * 1024 * 1024;
/// How long window setup waits for the main fonts, so the first frame
/// doesn't flash from egui's font to the system's.
const MAIN_FONT_WAIT: Duration = Duration::from_millis(200);

/// Fonts found and read on the font thread, in two batches.
pub enum FontMsg {
    /// The system's sans-serif and monospace fonts (fast to find).
    Main(Vec<FontInsert>),
    /// Fonts for other scripts (one `fc-match` per script).
    Fallbacks(Vec<FontInsert>),
}

/// Starts finding and reading fonts on a worker thread. Call it before
/// the window opens; hand the receiver to `install`.
pub fn start() -> Receiver<FontMsg> {
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("fonts".to_string())
        .spawn(move || {
            let main_fonts = itamiiru_core::platform::main_font_files();
            let mut main = Vec::new();
            let mut used = Vec::new();
            if let Some((path, index)) = main_fonts.sans {
                let family = FontFamily::Proportional;
                if let Some(font) = read_font("system-sans", &path, index, family) {
                    main.push(font);
                    used.push((path, index));
                }
            }
            if let Some((path, index)) = main_fonts.mono {
                let family = FontFamily::Monospace;
                if let Some(font) = read_font("system-mono", &path, index, family) {
                    main.push(font);
                    used.push((path, index));
                }
            }
            if tx.send(FontMsg::Main(main)).is_err() {
                return;
            }
            let mut loaded = Vec::new();
            for (path, _) in used {
                loaded.push(path);
            }
            let mut log = Vec::new();
            let scripts = itamiiru_core::platform::fallback_font_files();
            let fallbacks = choose_fallbacks(scripts, &mut loaded, &mut log);
            for line in log {
                itamiiru_core::debug!("fonts: {line}");
            }
            let _ = tx.send(FontMsg::Fallbacks(fallbacks));
        });
    if let Err(err) = spawned {
        itamiiru_core::debug!("fonts: could not start thread: {err}");
    }
    return rx;
}

/// Adds the fonts to egui: waits briefly for the main ones (window setup
/// calls this before the first frame), then adds the fallbacks whenever
/// they arrive, from a small thread.
pub fn install(ctx: &egui::Context, rx: Receiver<FontMsg>) {
    match rx.recv_timeout(MAIN_FONT_WAIT) {
        Ok(msg) => add(ctx, msg),
        Err(_) => itamiiru_core::debug!("fonts: main fonts not ready yet; adding them later"),
    }
    let ctx = ctx.clone();
    let spawned = std::thread::Builder::new()
        .name("fonts-add".to_string())
        .spawn(move || {
            while let Ok(msg) = rx.recv() {
                add(&ctx, msg);
                ctx.request_repaint();
            }
        });
    if let Err(err) = spawned {
        itamiiru_core::debug!("fonts: could not start thread: {err}");
    }
}

fn add(ctx: &egui::Context, msg: FontMsg) {
    let fonts = match msg {
        FontMsg::Main(fonts) | FontMsg::Fallbacks(fonts) => fonts,
    };
    for font in fonts {
        itamiiru_core::debug!("fonts: adding {}", font.name);
        ctx.add_font(font);
    }
}

/// A main font: first choice for its family.
fn read_font(name: &str, path: &Path, index: u32, family: FontFamily) -> Option<FontInsert> {
    let data = read_data(path, index)?;
    let families = vec![InsertFontFamily {
        family,
        priority: FontPriority::Highest,
    }];
    let name = format!("{name}-{}-{index}", path.display());
    return Some(FontInsert::new(&name, data, families));
}

/// For each script in `scripts` (the platform's ranked candidates), the
/// first candidate font egui can read, skipping
/// scripts a file in `loaded` already covers (the main font, or a CJK
/// collection picked for another language). Each file is loaded once.
/// What happened goes into `log`, for `--font-info`.
fn choose_fallbacks(
    scripts: Vec<(String, Vec<FontFile>)>,
    loaded: &mut Vec<PathBuf>,
    log: &mut Vec<String>,
) -> Vec<FontInsert> {
    let mut fonts = Vec::new();
    for (script, candidates) in scripts {
        if let Some((path, _)) = candidates.iter().find(|(path, _)| loaded.contains(path)) {
            log.push(format!("{script}: covered by {}", path.display()));
            continue;
        }
        let mut chosen = false;
        for (path, index) in &candidates {
            match load_font(path, *index) {
                Ok(data) => {
                    log.push(format!("{script}: {} (face {index}): ok", path.display()));
                    fonts.push(fallback_insert(data, path, *index));
                    loaded.push(path.clone());
                    chosen = true;
                    break;
                }
                Err(err) => {
                    log.push(format!(
                        "{script}: {} (face {index}): SKIPPED: {err}",
                        path.display()
                    ));
                }
            }
        }
        if !chosen {
            log.push(format!("{script}: no usable font"));
        }
    }
    return fonts;
}

/// A fallback: last choice, only for characters the other fonts lack.
fn fallback_insert(data: FontData, path: &Path, index: u32) -> FontInsert {
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
    let name = format!("fallback-{}-{index}", path.display());
    return FontInsert::new(&name, data, families);
}

fn read_data(path: &Path, index: u32) -> Option<FontData> {
    return match load_font(path, index) {
        Ok(data) => Some(data),
        Err(err) => {
            itamiiru_core::debug!("fonts: skipping {}: {err}", path.display());
            None
        }
    };
}

/// Reads a font file and makes sure egui can use it. egui panics on
/// font data it can't parse, at the next frame, which would take the
/// whole window down; so each font is tried on its own here first.
fn load_font(path: &Path, index: u32) -> Result<FontData, String> {
    let size = std::fs::metadata(path)
        .map_err(|err| err.to_string())?
        .len();
    if size > MAX_FONT_FILE_SIZE {
        return Err(format!("too big ({size} bytes)"));
    }
    let bytes = std::fs::read(path).map_err(|err| err.to_string())?;
    let mut data = FontData::from_owned(bytes);
    data.index = index;
    egui_can_use(&data)?;
    return Ok(data);
}

/// Builds a throwaway set of egui fonts holding just this one, and
/// catches the panic if egui can't parse it.
fn egui_can_use(data: &FontData) -> Result<(), String> {
    let mut definitions = FontDefinitions::empty();
    definitions
        .font_data
        .insert("test".to_string(), Arc::new(data.clone()));
    definitions
        .families
        .insert(FontFamily::Proportional, vec!["test".to_string()]);
    let result = std::panic::catch_unwind(move || {
        let _ = Fonts::new(TextOptions::default(), definitions);
    });
    if result.is_err() {
        return Err("egui can't read this font".to_string());
    }
    return Ok(());
}

/// `itami --font-info`: what fontconfig picks, whether egui can read each
/// file, and whether text in other scripts would show. For finding out
/// why characters show as boxes on a machine we can't look at.
pub fn print_report() {
    let mut out = String::new();
    push_line(
        &mut out,
        &format!("itamiiru {} font report", env!("CARGO_PKG_VERSION")),
    );
    out.push('\n');
    for line in itamiiru_core::platform::font_report() {
        push_line(&mut out, &line);
    }
    out.push('\n');
    push_line(&mut out, "Files itamiiru loads:");
    let main_fonts = itamiiru_core::platform::main_font_files();
    let mut loaded = Vec::new();
    for (role, font) in [
        ("main sans", main_fonts.sans),
        ("main mono", main_fonts.mono),
    ] {
        let Some((path, index)) = font else {
            continue;
        };
        let status = match load_font(&path, index) {
            Ok(_) => {
                loaded.push(path.clone());
                "ok".to_string()
            }
            Err(err) => format!("SKIPPED: {err}"),
        };
        push_line(
            &mut out,
            &format!("  {role}: {} (face {index}): {status}", path.display()),
        );
    }
    let mut log = Vec::new();
    let scripts = itamiiru_core::platform::fallback_font_files();
    choose_fallbacks(scripts, &mut loaded, &mut log);
    for line in log {
        push_line(&mut out, &format!("  {line}"));
    }

    // Load them all as the app does and try sample text.
    let ctx = egui::Context::default();
    let frame = |ctx: &egui::Context| {
        let mut output = ctx.run_ui(egui::RawInput::default(), |_ui| {});
        output.textures_delta.clear();
    };
    frame(&ctx);
    let rx = start();
    while let Ok(msg) = rx.recv() {
        add(&ctx, msg);
    }
    frame(&ctx);
    let font = egui::FontId::proportional(14.0);
    let first = ctx.fonts_mut(|f| {
        f.definitions().families[&FontFamily::Proportional]
            .first()
            .cloned()
            .unwrap_or_default()
    });
    out.push('\n');
    push_line(&mut out, &format!("First font: {first}"));
    push_line(
        &mut out,
        "Can egui show these? (\"no\" can be wrong for characters the first font has)",
    );
    let samples = [
        ("Japanese", "\u{65e5}\u{672c}\u{8a9e}\u{3042}\u{30a2}"),
        ("Chinese", "\u{4e2d}\u{6587}"),
        ("Korean", "\u{d55c}\u{ad6d}\u{c5b4}"),
        ("Thai", "\u{e44}\u{e17}\u{e22}"),
        ("Hebrew", "\u{5e2}\u{5d1}\u{5e8}\u{5d9}\u{5ea}"),
        ("Arabic", "\u{639}\u{631}\u{628}\u{64a}"),
        ("Hindi", "\u{939}\u{93f}\u{928}\u{94d}\u{926}\u{940}"),
    ];
    for (name, text) in samples {
        let yes = ctx.fonts_mut(|f| f.has_glyphs(&font, text));
        push_line(
            &mut out,
            &format!("  {name}: {}", if yes { "yes" } else { "no" }),
        );
    }
    // Written in one go, ignoring a closed pipe (`itami --font-info | head`
    // would otherwise panic).
    let _ = std::io::Write::write_all(&mut std::io::stdout(), out.as_bytes());
}

fn push_line(out: &mut String, text: &str) {
    out.push_str(text);
    out.push('\n');
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(ctx: &egui::Context) {
        let mut output = ctx.run_ui(egui::RawInput::default(), |ui| {
            ui.label("\u{65e5}\u{672c}\u{8a9e}");
        });
        output.textures_delta.clear();
    }

    #[test]
    fn unreadable_fonts_are_refused_not_crashed_on() {
        let garbage = FontData::from_owned(b"this is not a font file".to_vec());
        assert!(egui_can_use(&garbage).is_err());
        let built_in = egui::FontDefinitions::default();
        let ubuntu = built_in.font_data["Ubuntu-Light"].as_ref().clone();
        assert!(egui_can_use(&ubuntu).is_ok());
    }

    #[test]
    fn an_unreadable_candidate_falls_through_to_the_next() {
        let dir = std::env::temp_dir().join(format!("itamiiru-fonts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("temp dir");
        let garbage = dir.join("broken.ttf");
        std::fs::write(&garbage, b"not a font").expect("write");
        let good = dir.join("good.ttf");
        let built_in = egui::FontDefinitions::default();
        std::fs::write(&good, &built_in.font_data["Ubuntu-Light"].font[..]).expect("write");

        let scripts = vec![
            (
                "ja".to_string(),
                vec![(garbage.clone(), 0), (good.clone(), 0)],
            ),
            // Covered by a file loaded for "ja": not loaded again.
            ("ko".to_string(), vec![(good.clone(), 1)]),
        ];
        let mut loaded = Vec::new();
        let mut log = Vec::new();
        let fonts = choose_fallbacks(scripts, &mut loaded, &mut log);
        assert_eq!(fonts.len(), 1);
        assert_eq!(loaded, vec![good.clone()]);
        assert!(
            log[0].contains("broken.ttf") && log[0].contains("SKIPPED"),
            "{log:?}"
        );
        assert!(
            log[1].contains("good.ttf") && log[1].ends_with("ok"),
            "{log:?}"
        );
        assert!(log[2].starts_with("ko: covered by"), "{log:?}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Loads this machine's fonts the way the app does and checks the
    /// system's font comes first and Japanese is covered.
    /// Needs fontconfig and a CJK font, so it's run by hand:
    /// `cargo test -p itamiiru system_fonts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn system_fonts() {
        let ctx = egui::Context::default();
        run(&ctx);
        let font = egui::FontId::proportional(14.0);
        let japanese = "\u{65e5}\u{672c}\u{8a9e}\u{3042}\u{30a2}";
        assert!(!ctx.fonts_mut(|f| f.has_glyphs(&font, japanese)));

        install(&ctx, start());
        // The fallbacks arrive from a thread; give them a moment.
        for _ in 0..100 {
            run(&ctx);
            if ctx.fonts_mut(|f| f.has_glyphs(&font, japanese)) {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // Japanese comes from a fallback, not egui's replacement box. (egui's
        // has_glyph says false for anything in the first font, so it can't
        // check the system font itself; fontconfig says DejaVu Sans, the
        // usual sans-serif, has U+FFFD.)
        assert!(ctx.fonts_mut(|f| f.has_glyphs(&font, japanese)), "Japanese");
        let first =
            ctx.fonts_mut(|f| f.definitions().families[&FontFamily::Proportional][0].clone());
        println!("first proportional font: {first}");
        assert!(first.starts_with("system-sans"), "{first}");
    }
}
