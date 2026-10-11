mod address;
mod app;
mod details;
mod fonts;
mod icons;
mod iconview;
mod menus;
mod ops_ui;
mod selection;
mod sidebar;
mod theme;
mod thispc;
mod toolbar;
mod zoom;

use std::path::PathBuf;

use eframe::egui;
use theme::Theme;

fn main() -> eframe::Result {
    // Startup is the one time the UI thread may read small files:
    // the theme, and the time zone (loaded by the first local-time call).
    let theme = Theme::load();
    let _ = itamiiru_core::format::format_local_time(std::time::SystemTime::now());
    let start = start_folder();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("itamiiru")
            .with_app_id("itamiiru")
            .with_inner_size([900.0, 600.0]),
        ..Default::default()
    };
    return eframe::run_native(
        "itamiiru",
        options,
        Box::new(|cc| {
            theme.install(&cc.egui_ctx);
            fonts::load_in_background(cc.egui_ctx.clone());
            let app = app::App::new(cc.egui_ctx.clone(), theme, start);
            return Ok(Box::new(app));
        }),
    );
}

/// `itami [PATH]`: the folder from the command line, else the home folder.
fn start_folder() -> PathBuf {
    // args_os keeps non-UTF-8 paths intact.
    if let Some(arg) = std::env::args_os().nth(1) {
        let path = PathBuf::from(arg);
        if path.is_absolute() {
            return path;
        }
        if let Ok(cwd) = std::env::current_dir() {
            return cwd.join(path);
        }
    }
    if let Some(home) = itamiiru_core::platform::home_dir() {
        return home;
    }
    return PathBuf::from("/");
}
