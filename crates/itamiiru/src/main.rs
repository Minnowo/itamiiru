// One of the two renderers must be built in (see Cargo.toml).
#[cfg(not(any(feature = "glow", feature = "wgpu")))]
compile_error!("build with a renderer: --features glow (the default) or --features wgpu");

mod address;
mod app;
mod band;
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
mod startup;
mod tabs;
mod theme;
mod thispc;
mod thumbs;
mod toolbar;
mod zoom;

use std::ffi::OsString;
use std::io::{IsTerminal, Read};
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

/// How to call it, for `--help` and mistakes.
const USAGE: &str = "usage: itami [--foreground] [FOLDER] [--select ITEM]...
       itami --pick < request      (file dialog, used by the portal)
       itami --font-info           (which fonts are used, for bug reports)

Started from a terminal, itami moves to the background; --foreground
(or ITAMI_DEBUG=1) keeps it attached.";

/// The command line: `itami [FOLDER] [--select ITEM]...`, `itami --pick`,
/// or `itami --font-info`. FOLDER and ITEM may be paths or `file://` URIs.
struct Args {
    pick: bool,
    /// Print the font report and exit.
    font_info: bool,
    /// Stay attached to the terminal instead of moving to the background.
    foreground: bool,
    path: Option<PathBuf>,
    /// Items to select once the folder is listed ("Show in folder").
    select: Vec<PathBuf>,
}

fn main() {
    startup::begin();
    // args_os keeps non-UTF-8 paths intact.
    let args = match parse_args(std::env::args_os().skip(1).collect()) {
        Ok(args) => args,
        Err(message) => {
            eprintln!("itami: {message}");
            eprintln!("{USAGE}");
            std::process::exit(2);
        }
    };
    if args.font_info {
        fonts::print_report();
        return;
    }
    let debugging = std::env::var_os("ITAMI_DEBUG").is_some_and(|v| v == "1");
    let in_terminal = std::io::stdout().is_terminal() || std::io::stderr().is_terminal();
    if should_detach(&args, in_terminal, debugging) {
        // The copy's output isn't a terminal, so it won't detach again.
        let started = match std::env::current_exe() {
            Ok(exe) => {
                let arguments: Vec<OsString> = std::env::args_os().skip(1).collect();
                itamiiru_core::platform::spawn_detached(&exe, &arguments)
            }
            Err(err) => Err(err),
        };
        match started {
            Ok(()) => return,
            Err(err) => eprintln!("itami: can't move to the background ({err}); staying here"),
        }
    }

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

    startup::mark("config, theme and time zone loaded");
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
        None => "File Explorer".to_string(),
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
    // Fonts are found while the window opens.
    let font_rx = fonts::start();
    if cfg!(feature = "glow") {
        startup::mark("opening the window (renderer: glow, OpenGL)");
    } else {
        startup::mark("opening the window (renderer: wgpu)");
    }
    let result = eframe::run_native(
        "itamiiru",
        options,
        Box::new(|cc| {
            startup::mark("window and OpenGL ready");
            theme.install(&cc.egui_ctx);
            cc.egui_ctx.set_zoom_factor(settings.zoom);
            fonts::install(&cc.egui_ctx, font_rx);
            startup::mark("fonts installed");
            let ctx = cc.egui_ctx.clone();
            let app = app::App::new(ctx, theme, start, select, config, config_path, picker);
            startup::mark("app built");
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

/// Started from a terminal, the explorer moves to the background so the
/// prompt comes back. Not the picker (the portal reads its output and exit
/// code), not with --foreground, and not while debugging (the log goes to
/// the terminal).
fn should_detach(args: &Args, in_terminal: bool, debugging: bool) -> bool {
    return in_terminal && !args.pick && !args.foreground && !debugging;
}

/// `arguments` is the command line without the program name. An unknown
/// `--option` is an error, not a folder to open. `--help` prints the usage
/// and exits.
fn parse_args(arguments: Vec<OsString>) -> Result<Args, String> {
    let mut args = Args {
        pick: false,
        font_info: false,
        foreground: false,
        path: None,
        select: Vec::new(),
    };
    let mut items = arguments.into_iter();
    while let Some(arg) = items.next() {
        if arg == "--pick" {
            args.pick = true;
        } else if arg == "--font-info" {
            args.font_info = true;
        } else if arg == "--foreground" {
            args.foreground = true;
        } else if arg == "--select" {
            if let Some(item) = items.next() {
                args.select.push(arg_to_path(item));
            }
        } else if arg == "--help" || arg == "-h" {
            println!("{USAGE}");
            std::process::exit(0);
        } else if arg.as_encoded_bytes().starts_with(b"--") {
            return Err(format!("unknown option {}", arg.to_string_lossy()));
        } else if args.path.is_none() {
            args.path = Some(arg_to_path(arg));
        }
    }
    return Ok(args);
}

/// A command-line path or `file://` URI (file managers and the .desktop
/// file pass URIs) as an absolute path, without `.` and `..` parts
/// (`itami ..` opens the parent folder, not `/home/me/Downloads/..`).
fn arg_to_path(arg: OsString) -> PathBuf {
    use itamiiru_core::pathnorm::normalize;
    if let Some(text) = arg.to_str()
        && text.starts_with("file://")
        && let Some(path) = itamiiru_core::percent::file_uri_to_path(text)
    {
        return normalize(&path);
    }
    let path = PathBuf::from(arg);
    if path.is_absolute() {
        return normalize(&path);
    }
    if let Ok(cwd) = std::env::current_dir() {
        return normalize(&cwd.join(path));
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
        return parse_args(arguments).expect("valid arguments");
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
    fn dots_are_expanded() {
        let cwd = std::env::current_dir().expect("cwd");
        let parent = cwd.parent().expect("a parent").to_path_buf();
        assert_eq!(parse(&[".."]).path, Some(parent.clone()));
        assert_eq!(parse(&["."]).path, Some(cwd.clone()));
        assert_eq!(parse(&["./sub/../other"]).path, Some(cwd.join("other")));
        assert_eq!(
            parse(&["/home/me/Downloads/.."]).path,
            Some(PathBuf::from("/home/me"))
        );
        assert_eq!(
            parse(&["file:///tmp/a/./b/.."]).path,
            Some(PathBuf::from("/tmp/a"))
        );
        let args = parse(&["--select", "/tmp/x/../y.txt"]);
        assert_eq!(args.select, vec![PathBuf::from("/tmp/y.txt")]);
    }

    #[test]
    fn flags() {
        assert!(parse(&["--pick"]).pick);
        assert!(parse(&["--font-info"]).font_info);
        assert!(!parse(&["/tmp"]).font_info);
    }

    #[test]
    fn detaching() {
        // From a terminal, the explorer detaches.
        assert!(should_detach(&parse(&["/tmp"]), true, false));
        // Not from a desktop launcher, xdg-open or "Show in folder".
        assert!(!should_detach(&parse(&["/tmp"]), false, false));
        assert!(!should_detach(
            &parse(&["--foreground", "/tmp"]),
            true,
            false
        ));
        assert!(!should_detach(&parse(&["--pick"]), true, false));
        assert!(!should_detach(&parse(&["/tmp"]), true, true));
        assert!(parse(&["--foreground"]).foreground);
    }

    #[test]
    fn unknown_options_are_errors_not_folders() {
        let arguments = vec![OsString::from("--font-infoo")];
        let error = parse_args(arguments).err().expect("an error");
        assert!(error.contains("--font-infoo"), "{error}");
        // A folder whose name starts with one dash still opens.
        let args = parse(&["-notes"]);
        assert!(args.path.is_some_and(|p| p.ends_with("-notes")));
    }
}
