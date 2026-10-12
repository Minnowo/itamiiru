mod address;
mod app;
mod details;
mod fonts;
mod icons;
mod iconview;
mod menus;
mod ops_ui;
mod pane;
mod picker;
mod preview;
mod selection;
mod settings;
mod settings_page;
mod sidebar;
mod tabs;
mod theme;
mod thispc;
mod thumbs;
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

/// The command line: `itami [FOLDER] [--select ITEM]...` or `itami --pick`.
/// FOLDER and ITEM may be paths or `file://` URIs.
struct Args {
    pick: bool,
    path: Option<PathBuf>,
    /// Items to select once the folder is listed ("Show in folder").
    select: Vec<PathBuf>,
}

fn main() {
    // args_os keeps non-UTF-8 paths intact.
    let args = parse_args(std::env::args_os().skip(1).collect());

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
    let select = if args.pick { Vec::new() } else { args.select };
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
        // Without a position, some window managers (xfwm4's smart
        // placement) put big windows in a corner. Wayland ignores this;
        // the compositor places windows there.
        centered: true,
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
            let app = app::App::new(ctx, theme, start, select, config, config_path, picker);
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

/// `arguments` is the command line without the program name.
fn parse_args(arguments: Vec<OsString>) -> Args {
    let mut args = Args {
        pick: false,
        path: None,
        select: Vec::new(),
    };
    let mut items = arguments.into_iter();
    while let Some(arg) = items.next() {
        if arg == "--pick" {
            args.pick = true;
        } else if arg == "--select" {
            if let Some(item) = items.next() {
                args.select.push(arg_to_path(item));
            }
        } else if args.path.is_none() {
            args.path = Some(arg_to_path(arg));
        }
    }
    return args;
}

/// A command-line path or `file://` URI (file managers and the .desktop
/// file pass URIs) as an absolute path.
fn arg_to_path(arg: OsString) -> PathBuf {
    if let Some(text) = arg.to_str()
        && text.starts_with("file://")
        && let Some(path) = itamiiru_core::percent::file_uri_to_path(text)
    {
        return path;
    }
    let path = PathBuf::from(arg);
    if path.is_absolute() {
        return path;
    }
    if let Ok(cwd) = std::env::current_dir() {
        return cwd.join(path);
    }
    return path;
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
    if let Some(path) = &args.path {
        return path.clone();
    }
    // Only items given: open the first one's folder.
    if let Some(item) = args.select.first()
        && let Some(parent) = item.parent()
    {
        return parent.to_path_buf();
    }
    if let Some(home) = itamiiru_core::platform::home_dir() {
        return home;
    }
    return PathBuf::from("/");
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(items: &[&str]) -> Args {
        let mut arguments = Vec::new();
        for item in items {
            arguments.push(OsString::from(item));
        }
        return parse_args(arguments);
    }

    #[test]
    fn folder_and_selection() {
        let args = parse(&[
            "/home/me",
            "--select",
            "/home/me/a.txt",
            "--select",
            "file:///home/me/b%20c.txt",
        ]);
        assert!(!args.pick);
        assert_eq!(args.path, Some(PathBuf::from("/home/me")));
        assert_eq!(
            args.select,
            vec![
                PathBuf::from("/home/me/a.txt"),
                PathBuf::from("/home/me/b c.txt")
            ]
        );
    }

    #[test]
    fn uri_folder_from_a_desktop_file() {
        let args = parse(&["file:///tmp/some%20dir"]);
        assert_eq!(args.path, Some(PathBuf::from("/tmp/some dir")));
    }

    #[test]
    fn selection_alone_opens_its_folder() {
        let args = parse(&["--select", "/etc/hosts"]);
        let config = Config::default();
        assert_eq!(start_folder(&args, None, &config), PathBuf::from("/etc"));
    }

    #[test]
    fn relative_paths_become_absolute() {
        let args = parse(&["sub", "--select"]);
        let cwd = std::env::current_dir().expect("cwd");
        assert_eq!(args.path, Some(cwd.join("sub")));
        // A trailing --select with nothing after it is ignored.
        assert!(args.select.is_empty());
    }

    #[test]
    fn pick_flag() {
        assert!(parse(&["--pick"]).pick);
    }
}
