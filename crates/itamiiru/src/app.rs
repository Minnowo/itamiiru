//! The main window: toolbar, file list and status bar.

use std::collections::{HashMap, HashSet, VecDeque};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Key, Modifiers, Sense};
use itamiiru_core::entry::Entry;
use itamiiru_core::fileops::Op;
use itamiiru_core::listing::{self, ListMsg, MtimeMsg};
use itamiiru_core::places::{self, DiskUsage, Places, PlacesMsg};
use itamiiru_core::sort::{self, SortColumn, SortOrder};

use crate::address::{AddressAction, AddressBar};
use crate::details::{self, Rename, TableAction, TableInput};
use crate::icons::{Arrow, arrow_button};
use crate::iconview::{self, KeySteps};
use crate::menus::{self, Command, MenuState};
use crate::ops_ui::{self, RunningOp};
use crate::selection::{Selection, TypeAhead};
use crate::sidebar::{self, SidebarAction};
use crate::theme::{self, Theme};
use crate::thispc::{self, TileAction};
use crate::toolbar::{self, ToolbarAction, ToolbarState, ViewMode};
use crate::zoom;

/// How often to check whether the open folder changed (PLAN.md D9).
const CHANGE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// How often to look for new drives and bookmarks while focused.
const PLACES_SCAN_INTERVAL: Duration = Duration::from_secs(5);
/// How many rows Page Up / Page Down move.
const PAGE_ROWS: isize = 20;

/// A place in the Back/Forward history.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Location {
    Folder(PathBuf),
    ThisPc,
}

/// Files cut or copied inside the app, waiting to be pasted.
struct FileClipboard {
    paths: Vec<PathBuf>,
    /// The same paths, for a fast "is this one cut?" check while drawing.
    set: HashSet<PathBuf>,
    cut: bool,
}

/// What to do once the next listing of the current folder completes.
#[derive(Default)]
struct AfterListing {
    /// Select these (e.g. just pasted or renamed items).
    select: Vec<PathBuf>,
    /// Start renaming this one (a new folder).
    rename: Option<PathBuf>,
}

pub struct App {
    theme: Theme,

    /// The folder being shown.
    current: PathBuf,
    back: Vec<Location>,
    forward: Vec<Location>,
    /// Showing the This PC page instead of `current`.
    this_pc: bool,
    /// The selected tile on the This PC page.
    this_pc_selected: Option<PathBuf>,
    address: AddressBar,

    /// The current folder's entries, kept sorted, hidden ones included.
    entries: Vec<Entry>,
    /// Indices into `entries` of the rows on screen, top to bottom.
    visible: Vec<usize>,
    show_hidden: bool,

    /// Bumped on every navigation or refresh; results from older listings are ignored.
    generation: u64,
    loading: bool,
    /// A refresh collects entries here and swaps them in when complete, so
    /// the list doesn't flash empty. A navigation streams into `entries`.
    refreshing: bool,
    pending: Vec<Entry>,
    error: Option<String>,
    /// Entries the last listing couldn't read.
    skipped: usize,

    selection: Selection,
    type_ahead: TypeAhead,
    /// Row to scroll into view on the next frame.
    scroll_to: Option<usize>,

    sort: SortOrder,
    view: ViewMode,
    /// How the arrow keys move in the current view, from the last frame.
    key_steps: KeySteps,

    /// The folder's modified time when it was last listed.
    dir_mtime: Option<SystemTime>,
    mtime_check_running: bool,
    last_mtime_check: f64,

    // Every worker thread gets a clone of a Sender; results come back on the Receivers.
    list_tx: Sender<ListMsg>,
    list_rx: Receiver<ListMsg>,
    mtime_tx: Sender<MtimeMsg>,
    mtime_rx: Receiver<MtimeMsg>,
    ctx: egui::Context,
    click_fix: zoom::ClickPositionFix,

    clipboard: Option<FileClipboard>,
    /// An empty set, for drawing when nothing is cut.
    no_cut: HashSet<PathBuf>,
    running: Option<RunningOp>,
    /// Operations waiting for the running one to finish.
    queued: VecDeque<Op>,
    /// Failed items from file operations, shown in a banner.
    op_errors: Vec<String>,
    /// Paths waiting for "Delete permanently?" to be answered.
    confirm_delete: Option<Vec<PathBuf>>,
    rename: Option<Rename>,
    after_listing: AfterListing,

    places: Places,
    usage: HashMap<PathBuf, DiskUsage>,
    /// Drives whose usage is being measured. A hung drive stays here and
    /// is never asked again, so threads don't pile up.
    usage_pending: HashSet<PathBuf>,
    places_scan_running: bool,
    last_places_scan: f64,
    places_tx: Sender<PlacesMsg>,
    places_rx: Receiver<PlacesMsg>,
}

impl App {
    pub fn new(ctx: egui::Context, theme: Theme, start: PathBuf) -> App {
        let (list_tx, list_rx) = mpsc::channel();
        let (mtime_tx, mtime_rx) = mpsc::channel();
        let (places_tx, places_rx) = mpsc::channel();
        let mut app = App {
            theme,
            current: start,
            back: Vec::new(),
            forward: Vec::new(),
            this_pc: false,
            this_pc_selected: None,
            address: AddressBar::default(),
            entries: Vec::new(),
            visible: Vec::new(),
            show_hidden: false,
            generation: 0,
            loading: false,
            refreshing: false,
            pending: Vec::new(),
            error: None,
            skipped: 0,
            selection: Selection::default(),
            type_ahead: TypeAhead::default(),
            scroll_to: None,
            sort: SortOrder {
                column: SortColumn::Name,
                ascending: true,
            },
            view: ViewMode::Details,
            key_steps: KeySteps {
                vertical: 1,
                horizontal: 0,
            },
            dir_mtime: None,
            mtime_check_running: false,
            last_mtime_check: 0.0,
            list_tx,
            list_rx,
            mtime_tx,
            mtime_rx,
            ctx,
            click_fix: zoom::ClickPositionFix::default(),
            clipboard: None,
            no_cut: HashSet::new(),
            running: None,
            queued: VecDeque::new(),
            op_errors: Vec::new(),
            confirm_delete: None,
            rename: None,
            after_listing: AfterListing::default(),
            places: Places::default(),
            usage: HashMap::new(),
            usage_pending: HashSet::new(),
            places_scan_running: false,
            last_places_scan: 0.0,
            places_tx,
            places_rx,
        };
        app.load();
        app.scan_places();
        return app;
    }

    /// Where we are, for the history.
    fn location(&self) -> Location {
        if self.this_pc {
            return Location::ThisPc;
        }
        return Location::Folder(self.current.clone());
    }

    /// Goes to `path`, remembering where we were for Back.
    fn navigate(&mut self, path: PathBuf) {
        if !self.this_pc && path == self.current {
            return;
        }
        self.back.push(self.location());
        self.forward.clear();
        self.this_pc = false;
        self.current = path;
        self.load();
    }

    /// Shows the This PC page, remembering where we were for Back.
    fn show_this_pc(&mut self) {
        if self.this_pc {
            return;
        }
        self.back.push(self.location());
        self.forward.clear();
        self.enter_this_pc();
    }

    fn enter_this_pc(&mut self) {
        self.this_pc = true;
        self.this_pc_selected = None;
        self.rename = None;
        self.selection.clear();
        // Fresh usage numbers for the tiles.
        self.scan_places();
    }

    /// Goes somewhere from the history, without adding to it.
    fn go_to_location(&mut self, location: Location) {
        match location {
            Location::Folder(path) => {
                self.this_pc = false;
                self.current = path;
                self.load();
            }
            Location::ThisPc => self.enter_this_pc(),
        }
    }

    fn go_back(&mut self) {
        if let Some(location) = self.back.pop() {
            self.forward.push(self.location());
            self.go_to_location(location);
        }
    }

    fn go_forward(&mut self) {
        if let Some(location) = self.forward.pop() {
            self.back.push(self.location());
            self.go_to_location(location);
        }
    }

    /// Up one folder. Up from the top of the filesystem goes to This PC,
    /// like going up from C:\ in Explorer.
    fn go_up(&mut self) {
        if self.this_pc {
            return;
        }
        match self.current.parent() {
            Some(parent) => self.navigate(parent.to_path_buf()),
            None => self.show_this_pc(),
        }
    }

    /// Starts listing the current folder from scratch. Any older listing
    /// still running is left alone; its results carry an old generation.
    fn load(&mut self) {
        self.entries.clear();
        self.visible.clear();
        self.selection.clear();
        self.rename = None;
        self.error = None;
        self.refreshing = false;
        self.start_listing();
    }

    /// Lists the current folder again, keeping the list and selection on
    /// screen until the new listing is complete (F5 and change detection).
    fn refresh(&mut self) {
        if self.loading && !self.refreshing {
            // A navigation is still streaming in; it will be up to date.
            return;
        }
        self.refreshing = true;
        self.start_listing();
    }

    fn start_listing(&mut self) {
        self.generation += 1;
        self.loading = true;
        self.skipped = 0;
        self.pending.clear();
        let ctx = self.ctx.clone();
        let wake: listing::Wake = Box::new(move || ctx.request_repaint());
        listing::start(
            self.current.clone(),
            self.generation,
            self.list_tx.clone(),
            wake,
        );
    }

    /// Takes every waiting message from the listing threads. Never blocks.
    fn receive_listing(&mut self) {
        // Several batches can arrive between frames; sort once at the end.
        let mut needs_sort = false;
        let mut finished = false;
        while let Ok(msg) = self.list_rx.try_recv() {
            match msg {
                ListMsg::Batch {
                    generation,
                    mut entries,
                } if generation == self.generation => {
                    if self.refreshing {
                        self.pending.append(&mut entries);
                        continue;
                    }
                    // Sort the new batch, then append. The list is now two
                    // sorted runs, which Rust's sort merges in about one pass.
                    sort::sort_entries(&mut entries, self.sort.column, self.sort.ascending);
                    self.entries.append(&mut entries);
                    needs_sort = true;
                }
                ListMsg::Finished {
                    generation,
                    skipped,
                    dir_mtime,
                } if generation == self.generation => {
                    if self.refreshing {
                        self.entries = std::mem::take(&mut self.pending);
                        self.refreshing = false;
                        needs_sort = true;
                    }
                    self.loading = false;
                    self.skipped = skipped;
                    self.dir_mtime = dir_mtime;
                    self.error = None;
                    finished = true;
                }
                ListMsg::Failed { generation, error } if generation == self.generation => {
                    needs_sort = false;
                    self.loading = false;
                    self.refreshing = false;
                    self.entries.clear();
                    self.visible.clear();
                    self.selection.clear();
                    self.error = Some(error.to_string());
                }
                // A result from a folder we already left, or an older refresh.
                _ => {}
            }
        }
        if needs_sort {
            self.sort_and_filter();
        }
        if finished {
            self.apply_after_listing();
        }
    }

    /// Selects just-created items, and starts renaming a new folder.
    fn apply_after_listing(&mut self) {
        let after = std::mem::take(&mut self.after_listing);
        if !after.select.is_empty() {
            let mut selection = std::mem::take(&mut self.selection);
            let paths = self.visible_paths();
            self.scroll_to = selection.select_paths(&paths, &after.select);
            self.selection = selection;
        }
        if let Some(path) = after.rename {
            self.start_rename(&path);
        }
    }

    /// Re-sorts `entries` and rebuilds `visible` from it.
    fn sort_and_filter(&mut self) {
        sort::sort_entries(&mut self.entries, self.sort.column, self.sort.ascending);
        self.visible.clear();
        for (i, entry) in self.entries.iter().enumerate() {
            if self.show_hidden || !entry.hidden {
                self.visible.push(i);
            }
        }
        // Drop selected rows that disappeared (deleted, or now hidden).
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        selection.retain_visible(&paths);
        self.selection = selection;
    }

    /// The visible rows' paths, top to bottom. Built only when needed
    /// (a click or key), not every frame.
    fn visible_paths(&self) -> Vec<&Path> {
        let mut paths = Vec::with_capacity(self.visible.len());
        for &i in &self.visible {
            paths.push(self.entries[i].path.as_path());
        }
        return paths;
    }

    fn set_sort(&mut self, column: SortColumn) {
        if column == self.sort.column {
            self.sort.ascending = !self.sort.ascending;
        } else {
            self.sort.column = column;
            self.sort.ascending = true;
        }
        self.sort_and_filter();
    }

    /// The toolbar's Sort menu sets column and direction directly.
    fn set_sort_order(&mut self, sort: SortOrder) {
        self.sort = sort;
        self.sort_and_filter();
    }

    /// The command row under the address bar.
    fn command_row(&mut self, ui: &mut egui::Ui) {
        let state = ToolbarState {
            selected: self.selection.count(),
            can_paste: self.clipboard.is_some(),
            this_pc: self.this_pc,
            view: self.view,
            sort: self.sort,
            show_hidden: self.show_hidden,
        };
        match toolbar::show(ui, &state) {
            Some(ToolbarAction::Command(command)) => self.run_command(command),
            Some(ToolbarAction::SetView(view)) => self.view = view,
            Some(ToolbarAction::SetSort(sort)) => self.set_sort_order(sort),
            None => {}
        }
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.sort_and_filter();
    }

    /// Double-click: opens a folder, or a file in its default app.
    fn open_row(&mut self, row: usize) {
        let Some(&index) = self.visible.get(row) else {
            return;
        };
        let entry = &self.entries[index];
        if entry.is_dir() {
            self.navigate(entry.path.clone());
            return;
        }
        let path = entry.path.clone();
        self.open_file(&path);
    }

    /// Enter or "Open": one selected folder is opened in place; selected
    /// files are opened in their default apps.
    fn open_selected(&mut self) {
        let selected = self.selected_entries();
        if selected.len() == 1 && selected[0].is_dir() {
            let path = selected[0].path.clone();
            self.navigate(path);
            return;
        }
        let mut files = Vec::new();
        for entry in selected {
            if !entry.is_dir() {
                files.push(entry.path.clone());
            }
        }
        for path in files {
            self.open_file(&path);
        }
    }

    fn open_file(&mut self, path: &Path) {
        if let Err(err) = itamiiru_core::platform::open_with_default_app(path) {
            let name = path.to_string_lossy();
            ops_ui::push_error(&mut self.op_errors, format!("{name}: can't open: {err}"));
        }
    }

    /// The selected entries, in display order.
    fn selected_entries(&self) -> Vec<&Entry> {
        let mut selected = Vec::new();
        for &i in &self.visible {
            let entry = &self.entries[i];
            if self.selection.is_selected(&entry.path) {
                selected.push(entry);
            }
        }
        return selected;
    }

    fn selected_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for entry in self.selected_entries() {
            paths.push(entry.path.clone());
        }
        return paths;
    }

    /// Right-click on a row: like Explorer, an unselected row becomes the
    /// only selection; a selected one keeps the whole selection.
    fn right_click_row(&mut self, row: usize) {
        let Some(&index) = self.visible.get(row) else {
            return;
        };
        if !self.selection.is_selected(&self.entries[index].path) {
            self.click_row(row, false, false);
        }
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::Open => self.open_selected(),
            Command::Cut => self.cut_or_copy(true),
            Command::Copy => self.cut_or_copy(false),
            Command::Paste => self.paste(),
            Command::CopyPath => {
                let text = paths_as_text(&self.selected_paths());
                if !text.is_empty() {
                    self.ctx.copy_text(text);
                }
            }
            Command::Rename => {
                if let Some(cursor) = self.selection.cursor() {
                    let path = cursor.to_path_buf();
                    self.start_rename(&path);
                }
            }
            Command::Trash => {
                let paths = self.selected_paths();
                if paths.is_empty() {
                    return;
                }
                if trash_supported() {
                    self.start_op(Op::Trash { paths });
                } else {
                    // No trash on this OS yet: fall back to asking first.
                    self.confirm_delete = Some(paths);
                }
            }
            Command::DeletePermanently => {
                let paths = self.selected_paths();
                if !paths.is_empty() {
                    self.confirm_delete = Some(paths);
                }
            }
            Command::NewFolder => {
                let parent = self.current.clone();
                self.start_op(Op::NewFolder { parent });
            }
            Command::SelectAll => self.select_all(),
            Command::Refresh => self.refresh(),
            Command::ToggleHidden => self.toggle_hidden(),
        }
    }

    /// Ctrl+X / Ctrl+C: remembers the selection for Paste. The paths also go
    /// on the system clipboard as text, so they can be pasted into a
    /// terminal, and so Ctrl+V reaches us (egui only reports it when the
    /// system clipboard holds text).
    fn cut_or_copy(&mut self, cut: bool) {
        let paths = self.selected_paths();
        if paths.is_empty() {
            return;
        }
        self.ctx.copy_text(paths_as_text(&paths));
        let set = paths.iter().cloned().collect();
        self.clipboard = Some(FileClipboard { paths, set, cut });
    }

    fn paste(&mut self) {
        let Some(clipboard) = &self.clipboard else {
            return;
        };
        let sources = clipboard.paths.clone();
        let dest_dir = self.current.clone();
        if clipboard.cut {
            // Cut items can only be pasted once.
            self.clipboard = None;
            self.start_op(Op::Move { sources, dest_dir });
        } else {
            self.start_op(Op::Copy { sources, dest_dir });
        }
    }

    /// Starts an operation, or queues it behind the running one.
    fn start_op(&mut self, op: Op) {
        if self.running.is_some() {
            self.queued.push_back(op);
            return;
        }
        self.running = Some(RunningOp::start(op, &self.ctx));
    }

    /// Takes messages from the running operation. When it finishes, the
    /// folder is refreshed and the new items get selected.
    fn receive_op(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        let Some(created) = running.poll(&mut self.op_errors) else {
            return;
        };
        let was_new_folder = running.is_new_folder;
        self.running = None;

        // Only items in the folder on screen can be selected.
        let mut here = Vec::new();
        for path in created {
            if path.parent() == Some(self.current.as_path()) {
                here.push(path);
            }
        }
        if was_new_folder {
            self.after_listing.rename = here.first().cloned();
        }
        self.after_listing.select = here;
        self.refresh();

        if let Some(next) = self.queued.pop_front() {
            self.start_op(next);
        }
    }

    /// F2: edits the name in place.
    fn start_rename(&mut self, path: &Path) {
        let mut entry_name = None;
        for entry in &self.entries {
            if entry.path == path {
                entry_name = Some(entry.name.clone());
                break;
            }
        }
        let Some(name) = entry_name else {
            return;
        };
        let Some(text) = name.to_str() else {
            // Editing would replace the invalid bytes; don't risk it.
            let shown = name.to_string_lossy();
            let error = format!("{shown}: this name isn't valid text; rename it in a terminal");
            ops_ui::push_error(&mut self.op_errors, error);
            return;
        };
        self.rename = Some(Rename {
            path: path.to_path_buf(),
            text: text.to_string(),
            needs_focus: true,
        });
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        self.scroll_to = selection.select_paths(&paths, &[path.to_path_buf()]);
        self.selection = selection;
    }

    fn finish_rename(&mut self, commit: bool) {
        let Some(rename) = self.rename.take() else {
            return;
        };
        if !commit {
            return;
        }
        let new_name = OsString::from(rename.text.trim());
        if Some(new_name.as_os_str()) == rename.path.file_name() {
            return;
        }
        self.start_op(Op::Rename {
            from: rename.path,
            new_name,
        });
    }

    fn scan_places(&mut self) {
        if self.places_scan_running {
            return;
        }
        self.places_scan_running = true;
        let ctx = self.ctx.clone();
        places::start_scan(
            self.places_tx.clone(),
            Box::new(move || ctx.request_repaint()),
        );
    }

    /// Rescans places every few seconds while the window is focused, so
    /// plugged-in drives and new bookmarks show up.
    fn check_places(&mut self, ctx: &egui::Context) {
        let (focused, now) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.time));
        if !focused {
            return;
        }
        ctx.request_repaint_after(PLACES_SCAN_INTERVAL);
        if now - self.last_places_scan >= PLACES_SCAN_INTERVAL.as_secs_f64() {
            self.last_places_scan = now;
            self.scan_places();
        }
    }

    fn receive_places(&mut self) {
        while let Ok(msg) = self.places_rx.try_recv() {
            match msg {
                PlacesMsg::Places(places) => {
                    self.places_scan_running = false;
                    self.places = places;
                    // Measure every drive again, except ones still busy.
                    let mut to_measure = Vec::new();
                    for drive in &self.places.drives {
                        if self.usage_pending.insert(drive.mount_point.clone()) {
                            to_measure.push(drive.mount_point.clone());
                        }
                    }
                    for mount_point in to_measure {
                        let ctx = self.ctx.clone();
                        let wake: listing::Wake = Box::new(move || ctx.request_repaint());
                        places::start_usage(mount_point, self.places_tx.clone(), wake);
                    }
                }
                PlacesMsg::Usage { mount_point, usage } => {
                    self.usage_pending.remove(&mount_point);
                    match usage {
                        Some(usage) => self.usage.insert(mount_point, usage),
                        None => self.usage.remove(&mount_point),
                    };
                }
            }
        }
    }

    fn menu_state(&self) -> MenuState {
        return MenuState {
            selected: self.selection.count(),
            can_paste: self.clipboard.is_some(),
            show_hidden: self.show_hidden,
            trash_supported: trash_supported(),
        };
    }

    fn click_row(&mut self, row: usize, ctrl: bool, shift: bool) {
        // The selection is taken out of `self` while it works on paths
        // borrowed from `self.entries`; Rust won't allow both borrows at once.
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        if shift {
            selection.shift_click(&paths, row);
        } else if ctrl {
            selection.ctrl_click(&paths, row);
        } else {
            selection.click(&paths, row);
        }
        self.selection = selection;
    }

    fn move_cursor(&mut self, delta: isize, extend: bool) {
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        self.scroll_to = selection.move_cursor(&paths, delta, extend);
        self.selection = selection;
    }

    fn select_all(&mut self) {
        let mut selection = std::mem::take(&mut self.selection);
        selection.select_all(&self.visible_paths());
        self.selection = selection;
    }

    fn type_ahead(&mut self, text: &str, now: f64) {
        let mut names = Vec::with_capacity(self.visible.len());
        let mut cursor_row = None;
        let cursor = self.selection.cursor();
        for (row, &i) in self.visible.iter().enumerate() {
            let entry = &self.entries[i];
            names.push(entry.name.to_string_lossy().into_owned());
            if cursor == Some(entry.path.as_path()) {
                cursor_row = Some(row);
            }
        }
        if let Some(row) = self.type_ahead.type_text(text, now, &names, cursor_row) {
            self.click_row(row, false, false);
            self.scroll_to = Some(row);
        }
    }

    /// Turns the address bar's text into a path: `~` is the home folder,
    /// and relative paths are relative to the current folder.
    fn parse_address(&self, text: &str) -> PathBuf {
        let text = text.trim();
        if (text == "~" || text.starts_with("~/"))
            && let Some(home) = itamiiru_core::platform::home_dir()
        {
            return home.join(text.trim_start_matches('~').trim_start_matches('/'));
        }
        return self.current.join(text);
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Leave keys alone while the user is typing in a text field, or a
        // dialog is waiting for an answer.
        let dialog_open = self.confirm_delete.is_some()
            || self.running.as_ref().is_some_and(|op| op.has_conflict());
        if ctx.egui_wants_keyboard_input() || dialog_open {
            return;
        }
        let all_rows = self.visible.len() as isize;
        let mut back = false;
        let mut forward = false;
        let mut up = false;
        let mut open = false;
        let mut select_all = false;
        let mut toggle_hidden = false;
        let mut refresh = false;
        let mut cursor_move: Option<(isize, bool)> = None;
        let mut typed = String::new();
        let mut now = 0.0;
        let mut commands = Vec::new();
        let mut edit_address = false;

        ctx.input_mut(|input| {
            // egui ignores an extra Shift or Alt when matching a key, so the
            // more specific combination must be checked first.
            back = input.consume_key(Modifiers::ALT, Key::ArrowLeft);
            forward = input.consume_key(Modifiers::ALT, Key::ArrowRight);
            up = input.consume_key(Modifiers::ALT, Key::ArrowUp)
                || input.consume_key(Modifiers::NONE, Key::Backspace);
            open = input.consume_key(Modifiers::NONE, Key::Enter);
            if input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::N) {
                commands.push(Command::NewFolder);
            }
            if input.consume_key(Modifiers::NONE, Key::F2) {
                commands.push(Command::Rename);
            }
            if input.consume_key(Modifiers::SHIFT, Key::Delete) {
                commands.push(Command::DeletePermanently);
            } else if input.consume_key(Modifiers::NONE, Key::Delete) {
                commands.push(Command::Trash);
            }
            select_all = input.consume_key(Modifiers::COMMAND, Key::A);
            toggle_hidden = input.consume_key(Modifiers::COMMAND, Key::H);
            refresh = input.consume_key(Modifiers::NONE, Key::F5);

            // Up/Down and Left/Right step by what the view's layout says,
            // e.g. a whole row in the icon grid. Alt+Left/Right (back and
            // forward) were consumed above, so they don't reach here.
            let steps = self.key_steps;
            let moves = [
                (Key::ArrowDown, steps.vertical),
                (Key::ArrowUp, -steps.vertical),
                (Key::ArrowRight, steps.horizontal),
                (Key::ArrowLeft, -steps.horizontal),
                (Key::PageDown, PAGE_ROWS * steps.vertical),
                (Key::PageUp, -PAGE_ROWS * steps.vertical),
                (Key::End, all_rows),
                (Key::Home, -all_rows),
            ];
            for (key, delta) in moves {
                if delta == 0 {
                    continue;
                }
                if input.consume_key(Modifiers::SHIFT, key) {
                    cursor_move = Some((delta, true));
                } else if input.consume_key(Modifiers::NONE, key) {
                    cursor_move = Some((delta, false));
                }
            }

            // Letters typed into the list, for type-ahead. egui doesn't send
            // text events for Ctrl or Alt shortcuts.
            // egui turns Ctrl+C/X/V into Copy/Cut/Paste events, not keys.
            for event in &input.events {
                match event {
                    egui::Event::Text(text) => typed.push_str(text),
                    egui::Event::Copy => commands.push(Command::Copy),
                    egui::Event::Cut => commands.push(Command::Cut),
                    egui::Event::Paste(_) => commands.push(Command::Paste),
                    _ => {}
                }
            }
            now = input.time;
            edit_address = input.consume_key(Modifiers::COMMAND, Key::L)
                || input.consume_key(Modifiers::ALT, Key::D)
                || input.consume_key(Modifiers::NONE, Key::F4);
        });

        if edit_address {
            if self.this_pc {
                self.address.start_editing(None);
            } else {
                self.address.start_editing(Some(&self.current));
            }
            return;
        }
        if self.this_pc {
            // Only navigation makes sense on the This PC page.
            if back {
                self.go_back();
            }
            if forward {
                self.go_forward();
            }
            if toggle_hidden {
                self.toggle_hidden();
            }
            if refresh {
                self.scan_places();
            }
            if open && let Some(path) = self.this_pc_selected.clone() {
                self.navigate(path);
            }
            return;
        }

        if back {
            self.go_back();
        }
        if forward {
            self.go_forward();
        }
        if up {
            self.go_up();
        }
        if open {
            self.open_selected();
        }
        for command in commands {
            self.run_command(command);
        }
        if select_all {
            self.select_all();
        }
        if toggle_hidden {
            self.toggle_hidden();
        }
        if refresh {
            self.refresh();
        }
        if let Some((delta, extend)) = cursor_move {
            self.move_cursor(delta, extend);
        }
        if !typed.is_empty() {
            self.type_ahead(&typed, now);
        }
    }

    /// Every couple of seconds while the window is focused, checks on a
    /// worker whether the folder's modified time changed, and refreshes if so.
    fn check_for_changes(&mut self, ctx: &egui::Context) {
        let (focused, now) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.time));
        if !focused || self.loading || self.error.is_some() || self.this_pc {
            return;
        }
        // Keep frames coming while idle, so this check keeps running.
        ctx.request_repaint_after(CHANGE_CHECK_INTERVAL);
        if self.mtime_check_running
            || now - self.last_mtime_check < CHANGE_CHECK_INTERVAL.as_secs_f64()
        {
            return;
        }
        self.last_mtime_check = now;
        // If a check hangs on a dead mount, this stays true and no more
        // checks start for this folder. That's the safe outcome.
        self.mtime_check_running = true;
        let ctx = self.ctx.clone();
        let wake: listing::Wake = Box::new(move || ctx.request_repaint());
        listing::start_mtime_check(
            self.current.clone(),
            self.generation,
            self.mtime_tx.clone(),
            wake,
        );
    }

    fn receive_mtime(&mut self) {
        while let Ok(msg) = self.mtime_rx.try_recv() {
            self.mtime_check_running = false;
            if msg.generation == self.generation && msg.mtime != self.dir_mtime {
                itamiiru_core::debug!("folder changed, refreshing: {}", self.current.display());
                self.refresh();
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let can_go_back = !self.back.is_empty();
            let back = ui.add_enabled_ui(can_go_back, |ui| arrow_button(ui, Arrow::Left));
            if back.inner.on_hover_text("Back (Alt+Left)").clicked() {
                self.go_back();
            }
            let can_go_forward = !self.forward.is_empty();
            let forward = ui.add_enabled_ui(can_go_forward, |ui| arrow_button(ui, Arrow::Right));
            if forward.inner.on_hover_text("Forward (Alt+Right)").clicked() {
                self.go_forward();
            }
            let can_go_up = !self.this_pc;
            let up = ui.add_enabled_ui(can_go_up, |ui| arrow_button(ui, Arrow::Up));
            if up.inner.on_hover_text("Up (Alt+Up)").clicked() {
                self.go_up();
            }

            let current = if self.this_pc {
                None
            } else {
                Some(self.current.clone())
            };
            match self.address.show(ui, current.as_deref(), &self.theme) {
                Some(AddressAction::GoTo(path)) => self.navigate(path),
                Some(AddressAction::ThisPc) => self.show_this_pc(),
                Some(AddressAction::Typed(text)) => {
                    if text.trim().eq_ignore_ascii_case("this pc") {
                        self.show_this_pc();
                    } else {
                        let path = self.parse_address(&text);
                        self.navigate(path);
                    }
                }
                None => {}
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let mut count = self.visible.len();
            if self.this_pc {
                count = 0;
                for drive in &self.places.drives {
                    if self.show_hidden || !drive.hidden {
                        count += 1;
                    }
                }
            }
            let items = if count == 1 { "item" } else { "items" };
            ui.label(format!("{count} {items}"));
            let selected = self.selection.count();
            if selected > 0 {
                ui.label(format!("{selected} selected"));
            }
            if self.loading {
                ui.spinner();
            }
            if self.skipped > 0 {
                ui.weak(format!("({} could not be read)", self.skipped));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                zoom::controls_right_to_left(ui);
            });
        });
    }
}

impl eframe::App for App {
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.click_fix.fix(ctx.zoom_factor(), raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.receive_listing();
        self.receive_mtime();
        self.receive_op();
        self.receive_places();
        self.handle_keys(ui.ctx());
        self.check_for_changes(ui.ctx());
        self.check_places(ui.ctx());
        self.dialogs(ui.ctx());

        let bar_frame = egui::Frame::new()
            .fill(self.theme.surface_container_1)
            .inner_margin(theme::GAP as i8);

        egui::Panel::top("toolbar").frame(bar_frame).show(ui, |ui| {
            self.toolbar(ui);
            ui.add_space(theme::GAP_SMALL);
            self.command_row(ui);
        });
        egui::Panel::bottom("status")
            .frame(bar_frame)
            .show(ui, |ui| self.status_bar(ui));
        if let Some(running) = &mut self.running {
            egui::Panel::bottom("operation")
                .frame(bar_frame)
                .show(ui, |ui| running.progress_panel(ui));
        }

        let mut sidebar_action = None;
        egui::Panel::left("places")
            .resizable(true)
            .default_size(220.0)
            .frame(bar_frame)
            .show(ui, |ui| {
                let current = if self.this_pc {
                    None
                } else {
                    Some(self.current.as_path())
                };
                sidebar_action = sidebar::show(
                    ui,
                    &self.places,
                    &self.usage,
                    current,
                    self.show_hidden,
                    &self.theme,
                );
            });
        match sidebar_action {
            Some(SidebarAction::GoTo(path)) => self.navigate(path),
            Some(SidebarAction::ThisPc) => self.show_this_pc(),
            None => {}
        }

        egui::CentralPanel::default().show(ui, |ui| {
            if self.this_pc {
                ops_ui::error_banner(ui, &self.theme, &mut self.op_errors);
                let action = thispc::show(
                    ui,
                    &self.places,
                    &self.usage,
                    self.show_hidden,
                    self.this_pc_selected.as_deref(),
                    &self.theme,
                );
                match action {
                    Some(TileAction::Select(path)) => self.this_pc_selected = Some(path),
                    Some(TileAction::Open(path)) => self.navigate(path),
                    Some(TileAction::CopyPath(path)) => {
                        self.ctx.copy_text(path.to_string_lossy().into_owned());
                    }
                    None => {}
                }
                return;
            }
            if let Some(error) = &self.error {
                show_error(ui, &self.theme, &self.current, error);
                return;
            }
            ops_ui::error_banner(ui, &self.theme, &mut self.op_errors);
            let menu = self.menu_state();
            let cut = match &self.clipboard {
                Some(clipboard) if clipboard.cut => &clipboard.set,
                _ => &self.no_cut,
            };
            let input = TableInput {
                entries: &self.entries,
                visible: &self.visible,
                selection: &self.selection,
                sort: self.sort,
                theme: &self.theme,
                cut,
                menu: &menu,
            };
            let scroll_to = self.scroll_to.take();
            let action = match self.view {
                ViewMode::Details => {
                    self.key_steps = KeySteps {
                        vertical: 1,
                        horizontal: 0,
                    };
                    details::show(ui, &input, &mut self.rename, scroll_to)
                }
                ViewMode::List => {
                    let (action, steps) =
                        iconview::show_list(ui, &input, &mut self.rename, scroll_to);
                    self.key_steps = steps;
                    action
                }
                ViewMode::MediumIcons | ViewMode::LargeIcons | ViewMode::ExtraLargeIcons => {
                    let icon = self.view.icon_size();
                    let (action, steps) =
                        iconview::show_icons(ui, &input, &mut self.rename, scroll_to, icon);
                    self.key_steps = steps;
                    action
                }
            };

            // The empty space below the rows: click to deselect, right-click
            // for the folder menu.
            let rest = ui.available_rect_before_wrap();
            let background = ui.allocate_rect(rest, Sense::click());
            let mut background_command = None;
            background.context_menu(|ui| {
                background_command = menus::background_menu(ui, &menu);
            });
            if background.clicked() || background.secondary_clicked() {
                self.selection.clear();
            }

            match action {
                Some(TableAction::Sort(column)) => self.set_sort(column),
                Some(TableAction::Click { row, ctrl, shift }) => self.click_row(row, ctrl, shift),
                Some(TableAction::Open(row)) => self.open_row(row),
                Some(TableAction::RightClick(row)) => self.right_click_row(row),
                Some(TableAction::Command(command)) => self.run_command(command),
                Some(TableAction::RenameDone { commit }) => self.finish_rename(commit),
                Some(TableAction::ClearSelection) => self.selection.clear(),
                None => {}
            }
            if let Some(command) = background_command {
                self.run_command(command);
            }
        });
    }
}

impl App {
    /// The conflict question and the delete confirmation, when needed.
    fn dialogs(&mut self, ctx: &egui::Context) {
        if let Some(running) = &mut self.running {
            running.conflict_dialog(ctx, &self.theme);
        }
        if let Some(paths) = &self.confirm_delete
            && let Some(confirmed) = ops_ui::confirm_delete_dialog(ctx, &self.theme, paths)
        {
            let paths = self.confirm_delete.take().unwrap_or_default();
            if confirmed {
                self.start_op(Op::Delete { paths });
            }
        }
    }
}

/// Only Linux has a trash implementation so far (PLAN.md P8.1, P8.3).
fn trash_supported() -> bool {
    return cfg!(target_os = "linux");
}

/// One path per line, for the system clipboard.
fn paths_as_text(paths: &[PathBuf]) -> String {
    let mut text = String::new();
    for path in paths {
        if !text.is_empty() {
            text.push('\n');
        }
        text.push_str(&path.to_string_lossy());
    }
    return text;
}

/// An inline message in place of the list, never a popup (P1.11).
fn show_error(ui: &mut egui::Ui, theme: &Theme, folder: &Path, error: &str) {
    egui::Frame::new()
        .fill(theme.error_container)
        .corner_radius(theme::RADIUS_MEDIUM)
        .inner_margin(theme::PADDING)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            let text = format!("Can't open {}\n{}", folder.to_string_lossy(), error);
            ui.colored_label(theme.on_error_container, text);
        });
}
