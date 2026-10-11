mod address;
mod app;
mod details;
mod fonts;
mod icons;
mod iconview;
mod menus;
mod ops_ui;
mod picker;
mod selection;
mod settings;
mod sidebar;
mod theme;
mod thispc;
mod toolbar;
mod zoom;

use std::ffi::OsString;
use std::io::Read;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};

use eframe::egui;
use itamiiru_core::config::{self, Config};
use itamiiru_core::mime::MimeDb;
use itamiiru_core::protocol::{self, Pattern, Request};
use picker::Picker;
use settings::Settings;
use theme::Theme;

/// The command line: `itami [PATH]` or `itami --pick`.
struct Args {
    pick: bool,
    path: Option<OsString>,
}

fn main() {
    let args = parse_args();

    // Startup is the one time the UI thread may read small files: the
    // request, the config, the theme, the MIME globs if a filter needs them,
    // and the time zone (loaded by the first local-time call).
    let request = if args.pick {
        match read_request() {
            Ok(request) => Some(request),
            Err(err) => {
                eprintln!("itami --pick: {err}");
                std::process::exit(picker::EXIT_ERROR);
            }
        }
    } else {
        None
    };
    let config_path = config::default_path();
    let config = match &config_path {
        Some(path) => Config::load(path),
        None => Config::default(),
    };
    let settings = Settings::from_config(&config);
    let theme = Theme::load();
    let _ = itamiiru_core::format::format_local_time(std::time::SystemTime::now());

    let start = start_folder(&args, request.as_ref(), &config);
    // Closing the window without choosing means "cancelled".
    let exit_code = Arc::new(AtomicI32::new(picker::EXIT_CANCELLED));
    let picker = match request {
        Some(request) => {
            let mime = if needs_mime(&request) {
                Some(MimeDb::load())
            } else {
                None
            };
            Some(Picker::new(request, mime.as_ref(), exit_code.clone()))
        }
        None => None,
    };
    let title = match &picker {
        Some(picker) => picker.window_title(),
        None => "itamiiru".to_string(),
    };

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_app_id("itamiiru")
            .with_inner_size(settings.window_size)
            .with_maximized(settings.maximized),
        ..Default::default()
    };
    let is_picker = picker.is_some();
    let result = eframe::run_native(
        "itamiiru",
        options,
        Box::new(|cc| {
            theme.install(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(settings.zoom);
            fonts::load_in_background(cc.egui_ctx.clone());
            let ctx = cc.egui_ctx.clone();
            let app = app::App::new(ctx, theme, start, config, config_path, picker);
            return Ok(Box::new(app));
        }),
    );
    if let Err(err) = result {
        eprintln!("itami: {err}");
        std::process::exit(picker::EXIT_ERROR);
    }
    if is_picker {
        std::process::exit(exit_code.load(Ordering::Relaxed));
    }
}

fn parse_args() -> Args {
    let mut args = Args {
        pick: false,
        path: None,
    };
    // args_os keeps non-UTF-8 paths intact.
    for arg in std::env::args_os().skip(1) {
        if arg == "--pick" {
            args.pick = true;
        } else if args.path.is_none() {
            args.path = Some(arg);
        }
    }
    return args;
}

/// Reads and parses the request from stdin, refusing oversized input.
fn read_request() -> Result<Request, Box<dyn std::error::Error>> {
    let mut bytes = Vec::new();
    let limit = protocol::MAX_REQUEST_SIZE as u64 + 1;
    std::io::stdin().take(limit).read_to_end(&mut bytes)?;
    if bytes.len() > protocol::MAX_REQUEST_SIZE {
        return Err("request is too large".into());
    }
    let text = String::from_utf8(bytes)?;
    return Ok(protocol::parse_request(&text)?);
}

fn needs_mime(request: &Request) -> bool {
    for filter in &request.filters {
        for pattern in &filter.patterns {
            if matches!(pattern, Pattern::Mime(_)) {
                return true;
            }
        }
    }
    return false;
}

/// Where to start: the picker request's folder (or the file's folder), then
/// the folder this app used last, then a command-line path, then home.
fn start_folder(args: &Args, request: Option<&Request>, config: &Config) -> PathBuf {
    if let Some(request) = request {
        if let Some(folder) = &request.folder {
            return folder.clone();
        }
        if let Some(file) = &request.file
            && let Some(parent) = file.parent()
        {
            return parent.to_path_buf();
        }
        if !request.app_id.is_empty()
            && let Some(value) = config.get(&app::last_folder_key(&request.app_id))
            && let Ok(bytes) = itamiiru_core::percent::decode(value)
            && let Some(path) = itamiiru_core::platform::path_from_bytes(bytes)
        {
            return path;
        }
    }
    if let Some(arg) = &args.path {
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
