//! The main window: tabs, toolbar, sidebar, the folder panes and the
//! status bar. Each tab holds one pane or two (split view); `pane.rs`
//! holds a pane's folder, listing and selection.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui::{self, Key, Modifiers, Sense};
use itamiiru_core::config::Config;
use itamiiru_core::fileops::Op;
use itamiiru_core::listing;
use itamiiru_core::places::{self, DiskUsage, Places, PlacesMsg};
use itamiiru_core::sort::{SortColumn, SortOrder};
use itamiiru_core::thumbcache::ThumbSize;

use crate::details::{self, TableAction, TableInput};
use crate::iconview::{self, KeySteps, ThumbSource};
use crate::menus::{self, Command, MenuState};
use crate::ops_ui::{self, RunningOp};
use crate::pane::{Filter, Pane};
use crate::picker::{FooterAction, Picker, ProbeOutcome};
use crate::preview::Preview;
use crate::settings::{ColumnWidths, Settings};
use crate::sidebar::{self, SidebarAction};
use crate::tabs::{self, MAX_PANES, MIN_PANE_WIDTH, Tab, TabAction};
use crate::theme::{self, Theme};
use crate::thispc::{self, TileAction};
use crate::thumbs::Thumbs;
use crate::toolbar::{self, ToolbarAction, ToolbarState, ViewMode};
use crate::zoom;

/// How often to look for new drives and bookmarks while focused.
const PLACES_SCAN_INTERVAL: Duration = Duration::from_secs(5);
/// Settings are saved this long after the last change (window drags and
/// column resizes change them every frame).
const SAVE_DELAY: f64 = 1.0;
/// How many rows Page Up / Page Down move.
const PAGE_ROWS: isize = 20;

/// Files cut or copied inside the app, waiting to be pasted.
struct FileClipboard {
    paths: Vec<PathBuf>,
    /// The same paths, for a fast "is this one cut?" check while drawing.
    set: HashSet<PathBuf>,
    cut: bool,
}

/// What happened in one pane this frame, applied after drawing.
#[derive(Default)]
struct PaneOutput {
    /// Clicked somewhere in the pane: it becomes the active one.
    activated: bool,
    action: Option<TableAction>,
    /// From the right-click menu on empty space.
    command: Option<Command>,
    tile: Option<TileAction>,
}

pub struct App {
    theme: Theme,

    tabs: Vec<Tab>,
    active_tab: usize,
    /// Each new pane gets the next number, to keep its widgets apart.
    next_pane_id: u64,

    // Shared by every pane.
    sort: SortOrder,
    view: ViewMode,
    show_hidden: bool,

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

    places: Places,
    usage: HashMap<PathBuf, DiskUsage>,
    /// Drives whose usage is being measured. A hung drive stays here and
    /// is never asked again, so threads don't pile up.
    usage_pending: HashSet<PathBuf>,
    places_scan_running: bool,
    last_places_scan: f64,
    places_tx: Sender<PlacesMsg>,
    places_rx: Receiver<PlacesMsg>,

    /// The config file's contents, kept so unknown keys and comments survive.
    config: Config,
    config_path: Option<PathBuf>,
    /// The settings as last written, and as of this frame.
    saved_settings: Settings,
    latest_settings: Settings,
    settings_changed_at: Option<f64>,
    sidebar_width: f32,
    column_widths: ColumnWidths,
    auto_refresh_folder: bool,
    auto_refresh_drives: bool,
    thumbnails_enabled: bool,
    thumbs: Thumbs,
    preview_pane: bool,
    preview_width: f32,
    preview: Preview,

    /// Set in picker mode (`itami --pick`). The picker has one tab and no
    /// split view.
    picker: Option<Picker>,
}

/// What the panes list with. A free function, so it can borrow these
/// fields while a pane is borrowed mutably from `tabs`.
fn make_filter(sort: SortOrder, show_hidden: bool, picker: &Option<Picker>) -> Filter<'_> {
    return Filter {
        sort,
        show_hidden,
        picker: picker.as_ref(),
    };
}

impl App {
    pub fn new(
        ctx: egui::Context,
        theme: Theme,
        start: PathBuf,
        select: Vec<PathBuf>,
        config: Config,
        config_path: Option<PathBuf>,
        picker: Option<Picker>,
    ) -> App {
        let settings = Settings::from_config(&config);
        let (places_tx, places_rx) = mpsc::channel();
        let thumbs = Thumbs::new(&ctx, settings.thumbnails);
        let mut first = Pane::new(0, ctx.clone(), start, false);
        // Items from `--select` get selected when the first listing is done.
        first.after_listing.select = select;
        let mut app = App {
            theme,
            tabs: vec![Tab::new(first)],
            active_tab: 0,
            next_pane_id: 1,
            sort: settings.sort,
            view: settings.view,
            show_hidden: settings.show_hidden,
            ctx,
            click_fix: zoom::ClickPositionFix::default(),
            clipboard: None,
            no_cut: HashSet::new(),
            running: None,
            queued: VecDeque::new(),
            op_errors: Vec::new(),
            confirm_delete: None,
            places: Places::default(),
            usage: HashMap::new(),
            usage_pending: HashSet::new(),
            places_scan_running: false,
            last_places_scan: 0.0,
            places_tx,
            places_rx,
            config,
            config_path,
            saved_settings: settings.clone(),
            latest_settings: settings.clone(),
            settings_changed_at: None,
            sidebar_width: settings.sidebar_width,
            column_widths: settings.columns,
            auto_refresh_folder: settings.auto_refresh_folder,
            auto_refresh_drives: settings.auto_refresh_drives,
            thumbnails_enabled: settings.thumbnails,
            thumbs,
            preview_pane: settings.preview_pane,
            preview_width: settings.preview_width,
            preview: Preview::new(),
            picker,
        };
        app.scan_places();
        return app;
    }

    /// The active pane of the active tab.
    fn pane(&self) -> &Pane {
        return self.tabs[self.active_tab].pane();
    }

    fn pane_mut(&mut self) -> &mut Pane {
        return self.tabs[self.active_tab].pane_mut();
    }

    /// The active pane and the filter it lists with, borrowed together.
    fn pane_and_filter(&mut self) -> (&mut Pane, Filter<'_>) {
        let pane = self.tabs[self.active_tab].pane_mut();
        let filter = make_filter(self.sort, self.show_hidden, &self.picker);
        return (pane, filter);
    }

    /// A new pane showing the same place as the active one, or `path`.
    fn new_pane(&mut self, path: Option<PathBuf>) -> Pane {
        let (start, this_pc) = match path {
            Some(path) => (path, false),
            None => (self.pane().current.clone(), self.pane().this_pc),
        };
        let id = self.next_pane_id;
        self.next_pane_id += 1;
        return Pane::new(id, self.ctx.clone(), start, this_pc);
    }

    /// Opens a tab after the active one: the same folder, or `path`.
    /// `switch` makes it the active tab (middle-click opens it behind).
    fn new_tab(&mut self, path: Option<PathBuf>, switch: bool) {
        if self.picker.is_some() {
            return;
        }
        let pane = self.new_pane(path);
        self.tabs.insert(self.active_tab + 1, Tab::new(pane));
        if switch {
            self.active_tab += 1;
        }
    }

    /// Closes a tab. The last one stays open.
    fn close_tab(&mut self, index: usize) {
        if self.tabs.len() <= 1 || index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        if self.active_tab > index || self.active_tab >= self.tabs.len() {
            self.active_tab -= 1;
        }
    }

    /// Ctrl+Tab and Ctrl+Shift+Tab.
    fn cycle_tab(&mut self, forward: bool) {
        let count = self.tabs.len();
        self.active_tab = if forward {
            (self.active_tab + 1) % count
        } else {
            (self.active_tab + count - 1) % count
        };
    }

    /// Split view on: a second pane opens beside the first, in the same
    /// folder. Off: the active pane stays.
    fn toggle_split(&mut self) {
        if self.picker.is_some() {
            return;
        }
        if self.tabs[self.active_tab].is_split() {
            self.tabs[self.active_tab].unsplit();
            return;
        }
        let pane = self.new_pane(None);
        let tab = &mut self.tabs[self.active_tab];
        if tab.panes.len() < MAX_PANES {
            tab.panes.push(pane);
        }
    }

    /// Drops queued thumbnail jobs for panes that moved, and scans drives
    /// for panes that just opened This PC.
    fn handle_navigation(&mut self) {
        let mut scan = false;
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if pane.navigated {
                    pane.navigated = false;
                    self.thumbs.navigated();
                    if pane.this_pc {
                        scan = true;
                    }
                }
            }
        }
        if scan {
            self.scan_places();
        }
    }

    /// Takes waiting results for every pane, including background tabs.
    fn receive_panes(&mut self) {
        let filter = make_filter(self.sort, self.show_hidden, &self.picker);
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                if let Some(error) = pane.receive_listing(&filter) {
                    ops_ui::push_error(&mut self.op_errors, error);
                }
                pane.receive_search(&filter);
                pane.receive_mtime(&filter);
                pane.check_search_typing(&self.ctx, &filter);
            }
        }
    }

    /// Re-sorts and re-filters every pane (sort order or picker filter changed).
    fn sort_all(&mut self) {
        let filter = make_filter(self.sort, self.show_hidden, &self.picker);
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.sort_and_filter(&filter);
            }
        }
    }

    fn set_sort(&mut self, column: SortColumn) {
        if column == self.sort.column {
            self.sort.ascending = !self.sort.ascending;
        } else {
            self.sort.column = column;
            self.sort.ascending = true;
        }
        self.sort_all();
    }

    /// The toolbar's Sort menu sets column and direction directly.
    fn set_sort_order(&mut self, sort: SortOrder) {
        self.sort = sort;
        self.sort_all();
    }

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        let filter = make_filter(self.sort, self.show_hidden, &self.picker);
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                pane.hidden_changed(&filter);
            }
        }
    }

    /// The command row under the tabs.
    fn command_row(&mut self, ui: &mut egui::Ui) {
        let split = if self.picker.is_some() {
            None
        } else {
            Some(self.tabs[self.active_tab].is_split())
        };
        let state = ToolbarState {
            selected: self.pane().selection.count(),
            can_paste: self.clipboard.is_some(),
            this_pc: self.pane().this_pc,
            view: self.view,
            sort: self.sort,
            show_hidden: self.show_hidden,
            preview_pane: self.preview_pane,
            split,
        };
        match toolbar::show(ui, &state) {
            Some(ToolbarAction::Command(command)) => self.run_command(command),
            Some(ToolbarAction::SetView(view)) => self.view = view,
            Some(ToolbarAction::SetSort(sort)) => self.set_sort_order(sort),
            None => {}
        }
    }

    /// Double-click: opens a folder, or a file in its default app.
    fn open_row(&mut self, row: usize) {
        let Some(entry) = self.pane().entry_at(row) else {
            return;
        };
        let path = entry.path.clone();
        if entry.is_dir() {
            self.pane_mut().navigate(path);
            return;
        }
        if let Some(picker) = &mut self.picker {
            // In the picker, double-clicking a file picks it.
            if picker.mode() == itamiiru_core::protocol::Mode::Save {
                picker.name = entry_name_text(&path);
                self.picker_accept();
            } else {
                self.picker_finish(vec![path]);
            }
            return;
        }
        self.open_file(&path);
    }

    /// Middle-click: a folder opens in a new tab, behind this one.
    fn middle_click_row(&mut self, row: usize) {
        let Some(entry) = self.pane().entry_at(row) else {
            return;
        };
        if entry.is_dir() {
            let path = entry.path.clone();
            self.new_tab(Some(path), false);
        }
    }

    /// Enter or "Open": one selected folder is opened in place; selected
    /// files are opened in their default apps.
    fn open_selected(&mut self) {
        let selected = self.pane().selected_entries();
        if selected.len() == 1 && selected[0].is_dir() {
            let path = selected[0].path.clone();
            self.pane_mut().navigate(path);
            return;
        }
        if self.picker.is_some() {
            // Enter on files in the picker is the Open/Save button.
            self.picker_accept();
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

    /// The one selected folder, for "Open in new tab".
    fn selected_folder(&self) -> Option<PathBuf> {
        let selected = self.pane().selected_entries();
        if selected.len() == 1 && selected[0].is_dir() {
            return Some(selected[0].path.clone());
        }
        return None;
    }

    fn open_file(&mut self, path: &Path) {
        if let Err(err) = itamiiru_core::platform::open_with_default_app(path) {
            let name = path.to_string_lossy();
            ops_ui::push_error(&mut self.op_errors, format!("{name}: can't open: {err}"));
        }
    }

    fn run_command(&mut self, command: Command) {
        match command {
            Command::Open => self.open_selected(),
            Command::Cut => self.cut_or_copy(true),
            Command::Copy => self.cut_or_copy(false),
            Command::Paste => self.paste(),
            Command::CopyPath => {
                let text = paths_as_text(&self.pane().selected_paths());
                if !text.is_empty() {
                    self.ctx.copy_text(text);
                }
            }
            Command::Rename => {
                if let Some(cursor) = self.pane().selection.cursor() {
                    let path = cursor.to_path_buf();
                    if let Err(error) = self.pane_mut().start_rename(&path) {
                        ops_ui::push_error(&mut self.op_errors, error);
                    }
                }
            }
            Command::Trash => {
                let paths = self.pane().selected_paths();
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
                let paths = self.pane().selected_paths();
                if !paths.is_empty() {
                    self.confirm_delete = Some(paths);
                }
            }
            Command::NewFolder => {
                let parent = self.pane().current.clone();
                self.start_op(Op::NewFolder { parent });
            }
            Command::SelectAll => {
                if !self.single_select() {
                    self.pane_mut().select_all();
                }
            }
            Command::Refresh => {
                if self.pane().this_pc {
                    self.scan_places();
                } else {
                    let (pane, filter) = self.pane_and_filter();
                    pane.refresh(&filter);
                }
            }
            Command::ToggleHidden => self.toggle_hidden(),
            Command::TogglePreview => self.preview_pane = !self.preview_pane,
            Command::OpenInNewTab => {
                if let Some(path) = self.selected_folder() {
                    self.new_tab(Some(path), true);
                }
            }
            Command::ToggleSplit => self.toggle_split(),
            Command::CopyToOtherPane => self.send_to_other_pane(false),
            Command::MoveToOtherPane => self.send_to_other_pane(true),
        }
    }

    /// Split view: copies or moves the selection into the other pane's
    /// folder, like Copy then Paste over there.
    fn send_to_other_pane(&mut self, move_items: bool) {
        let tab = &self.tabs[self.active_tab];
        if !tab.is_split() {
            return;
        }
        let other = &tab.panes[1 - tab.active];
        if other.this_pc || other.current == self.pane().current {
            return;
        }
        let dest_dir = other.current.clone();
        let sources = self.pane().selected_paths();
        if sources.is_empty() {
            return;
        }
        if move_items {
            self.start_op(Op::Move { sources, dest_dir });
        } else {
            self.start_op(Op::Copy { sources, dest_dir });
        }
    }

    /// Ctrl+X / Ctrl+C: remembers the selection for Paste. The paths also go
    /// on the system clipboard as text, so they can be pasted into a
    /// terminal, and so Ctrl+V reaches us (egui only reports it when the
    /// system clipboard holds text).
    fn cut_or_copy(&mut self, cut: bool) {
        let paths = self.pane().selected_paths();
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
        let dest_dir = self.pane().current.clone();
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
    /// panes on screen are refreshed (a move between the two panes changes
    /// both), and the new items get selected wherever they're showing.
    fn receive_op(&mut self) {
        let Some(running) = &mut self.running else {
            return;
        };
        let Some(created) = running.poll(&mut self.op_errors) else {
            return;
        };
        let was_new_folder = running.is_new_folder;
        self.running = None;

        let filter = make_filter(self.sort, self.show_hidden, &self.picker);
        let tab = &mut self.tabs[self.active_tab];
        for (index, pane) in tab.panes.iter_mut().enumerate() {
            if pane.this_pc {
                continue;
            }
            // Only items in the folder on screen can be selected.
            let mut here = Vec::new();
            for path in &created {
                if path.parent() == Some(pane.current.as_path()) {
                    here.push(path.clone());
                }
            }
            if was_new_folder && index == tab.active {
                pane.after_listing.rename = here.first().cloned();
            }
            pane.after_listing.select = here;
            pane.refresh(&filter);
        }

        if let Some(next) = self.queued.pop_front() {
            self.start_op(next);
        }
    }

    fn finish_rename(&mut self, commit: bool) {
        if let Some(op) = self.pane_mut().finish_rename(commit) {
            self.start_op(op);
        }
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
        if !focused || !self.auto_refresh_drives {
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

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Leave keys alone while the user is typing in a text field, or a
        // dialog is waiting for an answer.
        let dialog_open = self.confirm_delete.is_some()
            || self.running.as_ref().is_some_and(|op| op.has_conflict());
        if ctx.egui_wants_keyboard_input() || dialog_open {
            return;
        }
        let all_rows = self.pane().visible.len() as isize;
        let steps = self.pane().key_steps;
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
        let mut focus_search = false;
        let mut open_filter = false;
        let mut escape = false;
        let mut new_tab = false;
        let mut close_tab = false;
        let mut next_tab = false;
        let mut previous_tab = false;
        let mut switch_pane = false;

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
            if input.consume_key(Modifiers::ALT, Key::P) {
                commands.push(Command::TogglePreview);
            }
            toggle_hidden = input.consume_key(Modifiers::COMMAND, Key::H);
            refresh = input.consume_key(Modifiers::NONE, Key::F5);
            new_tab = input.consume_key(Modifiers::COMMAND, Key::T);
            close_tab = input.consume_key(Modifiers::COMMAND, Key::W);
            previous_tab = input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::Tab);
            next_tab = input.consume_key(Modifiers::COMMAND, Key::Tab);
            switch_pane = input.consume_key(Modifiers::NONE, Key::F6);

            // Up/Down and Left/Right step by what the view's layout says,
            // e.g. a whole row in the icon grid. Alt+Left/Right (back and
            // forward) were consumed above, so they don't reach here.
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
            // Before Ctrl+F, which would also match Ctrl+Shift+F.
            open_filter = input.consume_key(Modifiers::COMMAND | Modifiers::SHIFT, Key::F);
            focus_search = input.consume_key(Modifiers::COMMAND, Key::F)
                || input.consume_key(Modifiers::NONE, Key::F3);
            escape = input.consume_key(Modifiers::NONE, Key::Escape);
        });

        if self.picker.is_none() {
            if new_tab {
                self.new_tab(None, true);
                return;
            }
            if close_tab {
                self.close_tab(self.active_tab);
                return;
            }
            if next_tab || previous_tab {
                self.cycle_tab(next_tab);
                return;
            }
            if switch_pane {
                self.tabs[self.active_tab].switch_pane();
                return;
            }
        }
        if focus_search && !self.pane().this_pc {
            self.pane_mut().open_search();
            return;
        }
        if open_filter && !self.pane().this_pc {
            self.pane_mut().open_filter();
            return;
        }
        if escape && self.pane().filter_open {
            let (pane, filter) = self.pane_and_filter();
            pane.close_filter(&filter);
            return;
        }
        if escape && self.pane().search_open {
            self.pane_mut().close_search();
            return;
        }
        if escape && self.picker.is_some() {
            self.picker_cancel();
            return;
        }

        if edit_address {
            self.pane_mut().edit_address();
            return;
        }
        if self.pane().this_pc {
            // Only navigation makes sense on the This PC page.
            if back {
                self.pane_mut().go_back();
            }
            if forward {
                self.pane_mut().go_forward();
            }
            if toggle_hidden {
                self.toggle_hidden();
            }
            if refresh {
                self.scan_places();
            }
            if open && let Some(path) = self.pane().this_pc_selected.clone() {
                self.pane_mut().navigate(path);
            }
            return;
        }

        if back {
            self.pane_mut().go_back();
        }
        if forward {
            self.pane_mut().go_forward();
        }
        if up {
            self.pane_mut().go_up();
        }
        if open {
            self.open_selected();
        }
        for command in commands {
            self.run_command(command);
        }
        if select_all {
            self.run_command(Command::SelectAll);
        }
        if toggle_hidden {
            self.toggle_hidden();
        }
        if refresh {
            self.run_command(Command::Refresh);
        }
        if let Some((delta, extend)) = cursor_move {
            let extend = extend && !self.single_select();
            self.pane_mut().move_cursor(delta, extend);
        }
        if !typed.is_empty() {
            self.pane_mut().type_ahead(&typed, now);
        }
    }

    /// While the window is focused, the panes on screen check every couple
    /// of seconds whether their folder changed, and refresh if so.
    fn check_for_changes(&mut self, ctx: &egui::Context) {
        let (focused, now) = ctx.input(|i| (i.viewport().focused.unwrap_or(true), i.time));
        if !focused || !self.auto_refresh_folder {
            return;
        }
        for pane in &mut self.tabs[self.active_tab].panes {
            pane.check_for_changes(now);
        }
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        let pane = self.tabs[self.active_tab].pane();
        ui.horizontal(|ui| {
            let mut count = pane.visible.len();
            if pane.this_pc {
                count = 0;
                for drive in &self.places.drives {
                    if self.show_hidden || !drive.hidden {
                        count += 1;
                    }
                }
            }
            if let Some(active) = &pane.search {
                let results = if count == 1 { "result" } else { "results" };
                ui.label(format!("{count} {results} for \"{}\"", active.query));
                if active.running {
                    ui.spinner();
                }
                if active.truncated {
                    ui.weak("(stopped early: too many results or folders too deep)");
                }
            } else {
                let items = if count == 1 { "item" } else { "items" };
                ui.label(format!("{count} {items}"));
            }
            if pane.is_filtered() {
                ui.weak("(filtered)");
            }
            let selected = pane.selection.count();
            if selected > 0 {
                ui.label(format!("{selected} selected"));
            }
            if pane.loading {
                ui.spinner();
            }
            if pane.skipped > 0 {
                ui.weak(format!("({} could not be read)", pane.skipped));
            }

            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                zoom::controls_right_to_left(ui);
            });
        });
    }

    /// The central area: the operation error banner, then one pane, or
    /// two side by side with a drag handle between them.
    fn panes(&mut self, ui: &mut egui::Ui) {
        ops_ui::error_banner(ui, &self.theme, &mut self.op_errors);
        let full = ui.available_rect_before_wrap();
        let mut rects = vec![full];
        if self.tabs[self.active_tab].is_split() {
            let (left, handle, right) = self.split_handle(ui, full);
            rects = vec![left, right];
            ui.allocate_rect(handle, Sense::hover());
        }
        let mut outputs = Vec::new();
        for (index, rect) in rects.into_iter().enumerate() {
            let id = self.tabs[self.active_tab].panes[index].id;
            let builder = egui::UiBuilder::new()
                .id_salt(("pane", id))
                .max_rect(rect)
                .layout(egui::Layout::top_down(egui::Align::Min));
            let mut child = ui.new_child(builder);
            child.set_clip_rect(rect.intersect(ui.clip_rect()));
            outputs.push(self.show_pane(&mut child, index));
        }
        ui.allocate_rect(full, Sense::hover());
        for (index, output) in outputs.into_iter().enumerate() {
            let used = output.activated
                || output.action.is_some()
                || output.command.is_some()
                || output.tile.is_some();
            if used {
                self.tabs[self.active_tab].active = index;
                self.apply_pane_output(output);
            }
        }
    }

    /// The handle between the two panes of a split: drag it to share the
    /// width differently, double-click it for halves again. Returns the
    /// left pane's, the handle's and the right pane's rectangles.
    fn split_handle(
        &mut self,
        ui: &mut egui::Ui,
        full: egui::Rect,
    ) -> (egui::Rect, egui::Rect, egui::Rect) {
        let handle_width = theme::GAP;
        let usable = full.width() - handle_width;
        let tab = &mut self.tabs[self.active_tab];
        let left_width = split_width(usable, tab.split_ratio);
        let handle = egui::Rect::from_min_size(
            egui::pos2(full.left() + left_width, full.top()),
            egui::vec2(handle_width, full.height()),
        );
        let response = ui.interact(
            handle,
            ui.id().with("split-handle"),
            Sense::click_and_drag(),
        );
        if response.hovered() || response.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if response.dragged()
            && let Some(pointer) = response.interact_pointer_pos()
            && usable > 0.0
        {
            tab.split_ratio = (pointer.x - full.left() - handle_width / 2.0) / usable;
        }
        if response.double_clicked() {
            tab.split_ratio = 0.5;
        }
        // A grip down the middle, brighter while it's being used.
        let color = if response.hovered() || response.dragged() {
            self.theme.outline
        } else {
            self.theme.outline_variant
        };
        let grip =
            egui::Rect::from_center_size(handle.center(), egui::vec2(2.0, theme::GAP_LARGE * 2.0));
        ui.painter().rect_filled(grip, theme::RADIUS_SMALL, color);

        let left_width = split_width(usable, tab.split_ratio);
        let left = egui::Rect::from_min_size(full.min, egui::vec2(left_width, full.height()));
        let right = egui::Rect::from_min_max(
            egui::pos2(full.left() + left_width + handle_width, full.top()),
            full.max,
        );
        let handle = egui::Rect::from_min_size(
            egui::pos2(full.left() + left_width, full.top()),
            egui::vec2(handle_width, full.height()),
        );
        return (left, handle, right);
    }

    /// Draws one pane of the active tab: its navigation bar, then This PC,
    /// an error, or the file list. Actions are returned, not applied, so
    /// they reach the right pane after it becomes the active one.
    fn show_pane(&mut self, ui: &mut egui::Ui, index: usize) -> PaneOutput {
        let mut out = PaneOutput::default();
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
            out.activated = true;
        }
        let tab = &mut self.tabs[self.active_tab];
        let split = tab.is_split();
        let is_active = tab.active == index;
        // The other pane's folder name, and whether it's this pane's folder.
        let mut other_name = None;
        let mut other_same = false;
        if split {
            let this = &tab.panes[index];
            let that = &tab.panes[1 - index];
            if !that.this_pc && !this.this_pc {
                other_name = Some(tabs::short_title(&that.title()));
                other_same = that.current == this.current;
            }
        }
        let pane = &mut tab.panes[index];
        let theme = &self.theme;
        let filter = make_filter(self.sort, self.show_hidden, &self.picker);

        // In split view the active pane's bar is a step lighter.
        let bar_fill = if split && is_active {
            theme.surface_container_3
        } else {
            theme.surface_container_1
        };
        egui::Frame::new()
            .fill(bar_fill)
            .corner_radius(theme::RADIUS_MEDIUM)
            .inner_margin(theme::GAP as i8)
            .show(ui, |ui| pane.nav_bar(ui, theme, &filter));
        ui.add_space(theme::GAP_SMALL);

        if pane.this_pc {
            out.tile = thispc::show(
                ui,
                &self.places,
                &self.usage,
                self.show_hidden,
                pane.this_pc_selected.as_deref(),
                theme,
            );
            return out;
        }
        if let Some(error) = &pane.error {
            show_error(ui, theme, &pane.current, error);
            return out;
        }
        let menu = MenuState {
            selected: pane.selection.count(),
            can_paste: self.clipboard.is_some(),
            show_hidden: self.show_hidden,
            trash_supported: trash_supported(),
            open_in_new_tab: self.picker.is_none() && selected_is_one_folder(pane),
            other_pane: other_name,
            other_pane_same: other_same,
        };
        let cut = match &self.clipboard {
            Some(clipboard) if clipboard.cut => &clipboard.set,
            _ => &self.no_cut,
        };
        let input = TableInput {
            entries: &pane.entries,
            visible: &pane.visible,
            selection: &pane.selection,
            sort: self.sort,
            theme,
            cut,
            menu: &menu,
            show_folder: pane.search.is_some(),
            columns: self.column_widths,
        };
        let scroll_to = pane.scroll_to.take();
        out.action = match self.view {
            ViewMode::Details => {
                pane.key_steps = KeySteps {
                    vertical: 1,
                    horizontal: 0,
                };
                let (action, widths) = details::show(ui, &input, &mut pane.rename, scroll_to);
                // Columns: Name, [Folder,] Date modified, Type, Size. Only
                // the active pane's widths are saved.
                if is_active
                    && let Some(widths) = widths
                    && widths.len() >= 4
                {
                    self.column_widths.name = widths[0];
                    let mut next = 1;
                    if input.show_folder && widths.len() >= 5 {
                        self.column_widths.folder = widths[1];
                        next = 2;
                    }
                    self.column_widths.modified = widths[next];
                    self.column_widths.kind = widths[next + 1];
                }
                action
            }
            ViewMode::List => {
                let (action, steps) = iconview::show_list(ui, &input, &mut pane.rename, scroll_to);
                pane.key_steps = steps;
                action
            }
            ViewMode::MediumIcons | ViewMode::LargeIcons | ViewMode::ExtraLargeIcons => {
                let icon = self.view.icon_size();
                // In pixels, so zoom and HiDPI screens get sharp thumbnails.
                let size = ThumbSize::for_pixels(icon * ui.ctx().pixels_per_point());
                let source = ThumbSource {
                    thumbs: &self.thumbs,
                    size,
                };
                let (action, steps, on_screen) = iconview::show_icons(
                    ui,
                    &input,
                    &mut pane.rename,
                    scroll_to,
                    icon,
                    Some(source),
                );
                pane.key_steps = steps;
                let mut wanted = Vec::new();
                for index in on_screen {
                    wanted.push(&pane.entries[pane.visible[index]]);
                }
                self.thumbs.want(&wanted, size);
                action
            }
        };

        // The empty space below the rows: click to deselect, right-click
        // for the folder menu.
        let rest = ui.available_rect_before_wrap();
        let background = ui.allocate_rect(rest, Sense::click());
        background.context_menu(|ui| {
            out.command = menus::background_menu(ui, &menu);
        });
        if background.clicked() || background.secondary_clicked() {
            out.action = Some(TableAction::ClearSelection);
        }
        return out;
    }

    /// Applies what happened in a pane; it's the active pane by now.
    fn apply_pane_output(&mut self, output: PaneOutput) {
        match output.tile {
            Some(TileAction::Select(path)) => self.pane_mut().this_pc_selected = Some(path),
            Some(TileAction::Open(path)) => self.pane_mut().navigate(path),
            Some(TileAction::CopyPath(path)) => {
                self.ctx.copy_text(path.to_string_lossy().into_owned());
            }
            None => {}
        }
        match output.action {
            Some(TableAction::Sort(column)) => self.set_sort(column),
            Some(TableAction::Click { row, ctrl, shift }) => {
                let single = self.single_select();
                self.pane_mut().click_row(row, ctrl, shift, single);
            }
            Some(TableAction::Open(row)) => self.open_row(row),
            Some(TableAction::RightClick(row)) => self.pane_mut().right_click_row(row),
            Some(TableAction::MiddleClick(row)) => self.middle_click_row(row),
            Some(TableAction::Command(command)) => self.run_command(command),
            Some(TableAction::RenameDone { commit }) => self.finish_rename(commit),
            Some(TableAction::ClearSelection) => self.pane_mut().selection.clear(),
            None => {}
        }
        if let Some(command) = output.command {
            self.run_command(command);
        }
    }
}

impl App {
    /// True in a picker that returns one file or folder.
    fn single_select(&self) -> bool {
        return match &self.picker {
            Some(picker) => !picker.allows_multiple(),
            None => false,
        };
    }

    /// The Open/Save button, or Enter in the name field.
    fn picker_accept(&mut self) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        let outcome = {
            let pane = self.pane();
            let selected = pane.selected_entries();
            let current = pane.current.clone();
            let resolve = |text: &str| pane.parse_address(text);
            picker.accept(&current, &selected, &self.ctx, &resolve)
        };
        self.picker = Some(picker);
        if let Some(outcome) = outcome {
            self.picker_apply(outcome);
        }
    }

    fn picker_apply(&mut self, outcome: ProbeOutcome) {
        match outcome {
            ProbeOutcome::Finish(paths) => self.picker_finish(paths),
            ProbeOutcome::Navigate(path) => {
                if let Some(picker) = &mut self.picker {
                    picker.name = if picker.mode() == itamiiru_core::protocol::Mode::Save {
                        picker.suggested_name()
                    } else {
                        String::new()
                    };
                }
                self.pane_mut().navigate(path);
            }
            ProbeOutcome::ConfirmOverwrite(path) => {
                if let Some(picker) = &mut self.picker {
                    picker.confirm_overwrite = Some(path);
                }
            }
            ProbeOutcome::Error(message) => {
                if let Some(picker) = &mut self.picker {
                    picker.error = Some(message);
                }
            }
        }
    }

    /// Answers the request, remembers the folder for this app, and closes.
    fn picker_finish(&mut self, paths: Vec<PathBuf>) {
        let Some(picker) = &self.picker else {
            return;
        };
        picker.finish(&paths);
        let app_id = picker.request.app_id.clone();
        if !app_id.is_empty() {
            let key = last_folder_key(&app_id);
            let current = self.pane().current.as_os_str().as_encoded_bytes();
            let value = itamiiru_core::percent::encode(current);
            self.config.set(&key, &value);
            // Written now: the window is about to close.
            if let Some(path) = &self.config_path
                && let Err(err) = self.config.save(path)
            {
                itamiiru_core::debug!("config: can't save {}: {err}", path.display());
            }
        }
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn picker_cancel(&mut self) {
        if let Some(picker) = &self.picker {
            picker.cancel();
        }
        self.ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    /// Per-frame picker work: follow the selection, take probe results,
    /// re-filter when the filter changed, and the overwrite question.
    fn picker_frame(&mut self, ctx: &egui::Context) {
        let Some(mut picker) = self.picker.take() else {
            return;
        };
        {
            let pane = self.pane();
            let selected = pane.selected_entries();
            picker.follow_selection(&selected, pane.selection.cursor());
        }
        let outcome = picker.receive_probe();
        let filter_changed = picker.take_filter_changed();
        // Read before the dialog: answering it clears the path.
        let overwrite_path = picker.confirm_overwrite.clone();
        let overwrite = picker.overwrite_dialog(ctx, &self.theme);
        self.picker = Some(picker);

        if filter_changed {
            self.sort_all();
        }
        if let Some(outcome) = outcome {
            self.picker_apply(outcome);
        }
        if overwrite == Some(true)
            && let Some(path) = overwrite_path
        {
            self.picker_finish(vec![path]);
        }
    }

    /// The settings as they are this frame.
    fn current_settings(&self, ctx: &egui::Context) -> Settings {
        let zoom = ctx.zoom_factor();
        let (inner, maximized) = ctx.input(|i| {
            (
                i.viewport().inner_rect,
                i.viewport().maximized.unwrap_or(false),
            )
        });
        // Keep the normal size while maximized, so un-maximizing next time
        // restores it. egui reports the size in zoomed points; undo the zoom.
        let mut window_size = self.latest_settings.window_size;
        if !maximized && let Some(rect) = inner {
            window_size = [rect.width() * zoom, rect.height() * zoom];
        }
        return Settings {
            view: self.view,
            sort: self.sort,
            show_hidden: self.show_hidden,
            zoom,
            sidebar_width: self.sidebar_width,
            window_size,
            maximized,
            columns: self.column_widths,
            auto_refresh_folder: self.auto_refresh_folder,
            auto_refresh_drives: self.auto_refresh_drives,
            thumbnails: self.thumbnails_enabled,
            preview_pane: self.preview_pane,
            preview_width: self.preview_width,
        };
    }

    /// Saves settings a moment after they stop changing, on a worker thread.
    fn persist_settings(&mut self, ctx: &egui::Context) {
        self.latest_settings = self.current_settings(ctx);
        if self.latest_settings == self.saved_settings {
            self.settings_changed_at = None;
            return;
        }
        let now = ctx.input(|i| i.time);
        let changed_at = *self.settings_changed_at.get_or_insert(now);
        if now - changed_at < SAVE_DELAY {
            let wait = SAVE_DELAY - (now - changed_at);
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(wait));
            return;
        }
        self.settings_changed_at = None;
        self.saved_settings = self.latest_settings.clone();
        self.latest_settings.write_to(&mut self.config);
        let Some(path) = self.config_path.clone() else {
            return;
        };
        let config = self.config.clone();
        std::thread::spawn(move || {
            if let Err(err) = config.save(&path) {
                itamiiru_core::debug!("config: can't save {}: {err}", path.display());
            }
        });
    }
}

impl eframe::App for App {
    /// A last save on the way out, if something changed since the last one.
    /// Writing here, on the UI thread, is fine: the window is closing.
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        if self.latest_settings == self.saved_settings {
            return;
        }
        self.latest_settings.write_to(&mut self.config);
        if let Some(path) = &self.config_path
            && let Err(err) = self.config.save(path)
        {
            itamiiru_core::debug!("config: can't save {}: {err}", path.display());
        }
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.click_fix.fix(ctx.zoom_factor(), raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.handle_navigation();
        self.receive_panes();
        self.receive_op();
        self.receive_places();
        self.thumbs.receive(ui.ctx());
        self.picker_frame(ui.ctx());
        self.handle_keys(ui.ctx());
        self.check_for_changes(ui.ctx());
        self.check_places(ui.ctx());
        // Uses the sidebar and column widths measured last frame.
        self.persist_settings(ui.ctx());
        self.dialogs(ui.ctx());

        let bar_frame = egui::Frame::new()
            .fill(self.theme.surface_container_1)
            .inner_margin(theme::GAP as i8);

        // The picker is a dialog: no tabs. The tab strip is a darker band
        // whose active tab runs into the toolbar below it.
        let mut tab_action = None;
        if self.picker.is_none() {
            let strip = egui::Frame::new()
                .fill(self.theme.surface)
                .inner_margin(egui::Margin {
                    left: theme::GAP as i8,
                    right: theme::GAP as i8,
                    top: theme::GAP_SMALL as i8,
                    bottom: 0,
                });
            egui::Panel::top("tabs")
                .frame(strip)
                .show_separator_line(false)
                .show(ui, |ui| {
                    tab_action = tabs::tab_bar(ui, &self.tabs, self.active_tab, &self.theme);
                });
        }
        egui::Panel::top("toolbar")
            .frame(bar_frame)
            .show(ui, |ui| self.command_row(ui));
        match tab_action {
            Some(TabAction::Select(index)) => self.active_tab = index,
            Some(TabAction::Close(index)) => self.close_tab(index),
            Some(TabAction::New) => self.new_tab(None, true),
            None => {}
        }
        egui::Panel::bottom("status")
            .frame(bar_frame)
            .show(ui, |ui| self.status_bar(ui));
        let mut footer_action = None;
        if let Some(picker) = &mut self.picker {
            egui::Panel::bottom("picker")
                .frame(bar_frame)
                .show(ui, |ui| {
                    footer_action = picker.footer(ui, &self.theme);
                });
        }
        match footer_action {
            Some(FooterAction::Accept) => self.picker_accept(),
            Some(FooterAction::Cancel) => self.picker_cancel(),
            None => {}
        }
        if let Some(running) = &mut self.running {
            egui::Panel::bottom("operation")
                .frame(bar_frame)
                .show(ui, |ui| running.progress_panel(ui));
        }

        let mut sidebar_action = None;
        let sidebar = egui::Panel::left("places")
            .resizable(true)
            .default_size(self.sidebar_width)
            .frame(bar_frame)
            .show(ui, |ui| {
                let pane = self.tabs[self.active_tab].pane();
                let current = if pane.this_pc {
                    None
                } else {
                    Some(pane.current.as_path())
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
        self.sidebar_width = sidebar.response.rect.width();
        match sidebar_action {
            Some(SidebarAction::GoTo(path)) => self.pane_mut().navigate(path),
            Some(SidebarAction::ThisPc) => self.pane_mut().show_this_pc(),
            None => {}
        }

        if self.preview_pane && !self.pane().this_pc {
            let panel = egui::Panel::right("preview")
                .resizable(true)
                .default_size(self.preview_width)
                .frame(bar_frame)
                .show(ui, |ui| {
                    let pane = self.tabs[self.active_tab].pane();
                    self.preview.show(
                        ui,
                        &pane.entries,
                        pane.previewed(),
                        &mut self.thumbs,
                        &self.theme,
                    );
                });
            self.preview_width = panel.response.rect.width();
        }

        egui::CentralPanel::default().show(ui, |ui| self.panes(ui));
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

/// The left pane's width in a split `usable` points wide (the handle
/// excluded), keeping both panes at least MIN_PANE_WIDTH when there's room.
fn split_width(usable: f32, ratio: f32) -> f32 {
    let width = usable * ratio;
    if usable < 2.0 * MIN_PANE_WIDTH {
        return usable / 2.0;
    }
    return width.clamp(MIN_PANE_WIDTH, usable - MIN_PANE_WIDTH);
}

/// True when exactly one folder is selected.
fn selected_is_one_folder(pane: &Pane) -> bool {
    let selected = pane.selected_entries();
    return selected.len() == 1 && selected[0].is_dir();
}

/// The config key for an app's last picker folder.
pub fn last_folder_key(app_id: &str) -> String {
    return format!(
        "last_folder.{}",
        itamiiru_core::percent::encode(app_id.as_bytes())
    );
}

fn entry_name_text(path: &Path) -> String {
    return match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => String::new(),
    };
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

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{Event, PointerButton, Pos2, Rect};

    const SCREEN: Rect = Rect::from_min_max(Pos2::ZERO, Pos2::new(1200.0, 800.0));

    /// A folder with two subfolders and a file, removed when dropped.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Folder {
            let path =
                std::env::temp_dir().join(format!("itamiiru-app-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&path);
            std::fs::create_dir_all(path.join("alpha")).expect("create alpha");
            std::fs::create_dir_all(path.join("beta")).expect("create beta");
            std::fs::write(path.join("file.txt"), b"hello").expect("create file");
            return Folder(path);
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn new_app(ctx: &egui::Context, start: &Path) -> App {
        let mut config = Config::default();
        config.set("thumbnails", "false");
        return App::new(
            ctx.clone(),
            Theme::default(),
            start.to_path_buf(),
            Vec::new(),
            config,
            None,
            None,
        );
    }

    /// Runs one frame of the real UI, without a window. Returns the mouse
    /// cursor the frame asked for.
    fn frame(app: &mut App, ctx: &egui::Context, events: Vec<Event>) -> egui::CursorIcon {
        let input = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(input, |ui| eframe::App::ui(app, ui, &mut frame));
        // There's no renderer to hand texture updates to.
        output.textures_delta.clear();
        return output.platform_output.cursor_icon;
    }

    /// Runs frames until every pane on screen has finished listing.
    fn settle(app: &mut App, ctx: &egui::Context) {
        for _ in 0..300 {
            frame(app, ctx, Vec::new());
            let mut busy = false;
            for pane in &app.tabs[app.active_tab].panes {
                if pane.loading {
                    busy = true;
                }
            }
            if !busy {
                frame(app, ctx, Vec::new());
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("listing didn't finish");
    }

    fn key(key: Key, modifiers: Modifiers) -> Vec<Event> {
        return vec![Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }];
    }

    fn click(app: &mut App, ctx: &egui::Context, pos: Pos2) {
        let press = |pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(app, ctx, vec![Event::PointerMoved(pos)]);
        frame(app, ctx, vec![press(true)]);
        frame(app, ctx, vec![press(false)]);
    }

    #[test]
    fn tabs_open_switch_and_close() {
        let folder = Folder::new("tabs");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        assert_eq!(app.pane().visible.len(), 3);

        frame(&mut app, &ctx, key(Key::T, Modifiers::COMMAND));
        settle(&mut app, &ctx);
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 1);
        assert_eq!(app.pane().current, folder.0);

        frame(&mut app, &ctx, key(Key::Tab, Modifiers::COMMAND));
        assert_eq!(app.active_tab, 0);
        frame(
            &mut app,
            &ctx,
            key(Key::Tab, Modifiers::COMMAND | Modifiers::SHIFT),
        );
        assert_eq!(app.active_tab, 1);

        frame(&mut app, &ctx, key(Key::W, Modifiers::COMMAND));
        assert_eq!(app.tabs.len(), 1);
        assert_eq!(app.active_tab, 0);
        // The last tab stays.
        frame(&mut app, &ctx, key(Key::W, Modifiers::COMMAND));
        assert_eq!(app.tabs.len(), 1);
    }

    #[test]
    fn middle_click_opens_a_folder_in_a_tab_behind() {
        let folder = Folder::new("middle");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let mut alpha_row = None;
        for row in 0..app.pane().visible.len() {
            if app
                .pane()
                .entry_at(row)
                .is_some_and(|e| e.path.ends_with("alpha"))
            {
                alpha_row = Some(row);
            }
        }
        app.apply_pane_output(PaneOutput {
            action: Some(TableAction::MiddleClick(alpha_row.expect("alpha listed"))),
            ..Default::default()
        });
        assert_eq!(app.tabs.len(), 2);
        assert_eq!(app.active_tab, 0);
        assert_eq!(app.tabs[1].pane().current, folder.0.join("alpha"));
    }

    #[test]
    fn split_view_panes_are_independent() {
        let folder = Folder::new("split");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);

        app.run_command(Command::ToggleSplit);
        settle(&mut app, &ctx);
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.panes.len(), 2);
        assert_eq!(tab.active, 0);
        assert_eq!(tab.panes[1].visible.len(), 3);
        // At most two panes.
        assert_eq!(MAX_PANES, 2);

        // A click in the empty space of the right half activates that pane.
        click(&mut app, &ctx, Pos2::new(1100.0, 600.0));
        assert_eq!(app.tabs[app.active_tab].active, 1);

        // Navigating moves only the active pane.
        app.pane_mut().navigate(folder.0.join("alpha"));
        settle(&mut app, &ctx);
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.panes[0].current, folder.0);
        assert_eq!(tab.panes[1].current, folder.0.join("alpha"));

        // Every view draws with two panes.
        for view in ViewMode::ALL {
            app.view = view;
            frame(&mut app, &ctx, Vec::new());
            frame(&mut app, &ctx, Vec::new());
        }

        // F6 switches back to the left pane.
        frame(&mut app, &ctx, key(Key::F6, Modifiers::NONE));
        assert_eq!(app.tabs[app.active_tab].active, 0);

        // Turning split view off keeps the active pane.
        app.run_command(Command::ToggleSplit);
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.panes.len(), 1);
        assert_eq!(tab.pane().current, folder.0);
    }

    #[test]
    fn split_width_keeps_both_panes_usable() {
        assert_eq!(split_width(1000.0, 0.5), 500.0);
        assert_eq!(split_width(1000.0, 0.05), MIN_PANE_WIDTH);
        assert_eq!(split_width(1000.0, 0.99), 1000.0 - MIN_PANE_WIDTH);
        // Too narrow for both minimums: halves.
        assert_eq!(split_width(300.0, 0.9), 150.0);
    }

    #[test]
    fn dragging_the_handle_moves_the_split() {
        let folder = Folder::new("handle");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        app.run_command(Command::ToggleSplit);
        settle(&mut app, &ctx);

        // Find the handle by its cursor, starting past the sidebar's own
        // resize edge.
        let full_left = app.sidebar_width + MIN_PANE_WIDTH / 2.0;
        let mut handle_x = None;
        for x in (full_left as i32..1200).step_by(2) {
            let ratio_before = app.tabs[app.active_tab].split_ratio;
            let pos = Pos2::new(x as f32, 600.0);
            let cursor = frame(&mut app, &ctx, vec![Event::PointerMoved(pos)]);
            if cursor == egui::CursorIcon::ResizeHorizontal {
                handle_x = Some(pos.x);
                assert_eq!(app.tabs[app.active_tab].split_ratio, ratio_before);
                break;
            }
        }
        let start = Pos2::new(handle_x.expect("found the handle"), 600.0);
        let end = Pos2::new(start.x - 150.0, 600.0);
        let button = |pos, pressed| Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(&mut app, &ctx, vec![button(start, true)]);
        for step in 1..=10 {
            let x = start.x - 15.0 * step as f32;
            frame(
                &mut app,
                &ctx,
                vec![Event::PointerMoved(Pos2::new(x, 600.0))],
            );
        }
        frame(&mut app, &ctx, vec![button(end, false)]);
        let ratio = app.tabs[app.active_tab].split_ratio;
        assert!(ratio < 0.45, "ratio {ratio}");
    }

    #[test]
    fn filter_hides_names_and_escape_shows_them_again() {
        let folder = Folder::new("filter");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        assert_eq!(app.pane().visible.len(), 3);

        frame(
            &mut app,
            &ctx,
            key(Key::F, Modifiers::COMMAND | Modifiers::SHIFT),
        );
        frame(&mut app, &ctx, Vec::new());
        assert!(app.pane().filter_open);
        assert!(!app.pane().search_open);
        // The filter box has focus; type into it.
        frame(&mut app, &ctx, vec![Event::Text("ALP".to_string())]);
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(app.pane().visible.len(), 1);
        assert!(app.pane().is_filtered());

        frame(&mut app, &ctx, key(Key::Escape, Modifiers::NONE));
        frame(&mut app, &ctx, Vec::new());
        assert!(!app.pane().filter_open);
        assert_eq!(app.pane().visible.len(), 3);
    }

    /// Runs frames until the file operation and the refreshes after it finish.
    fn settle_op(app: &mut App, ctx: &egui::Context) {
        for _ in 0..300 {
            frame(app, ctx, Vec::new());
            if app.running.is_none() && app.queued.is_empty() {
                settle(app, ctx);
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("operation didn't finish");
    }

    /// Selects the row named `name` in the active pane.
    fn select_named(app: &mut App, name: &str) {
        let mut found = None;
        for row in 0..app.pane().visible.len() {
            if app
                .pane()
                .entry_at(row)
                .is_some_and(|e| e.path.ends_with(name))
            {
                found = Some(row);
            }
        }
        let row = found.expect("row is listed");
        app.pane_mut().click_row(row, false, false, false);
    }

    #[test]
    fn copy_and_move_to_the_other_pane() {
        let folder = Folder::new("otherpane");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        // Same folder on both sides: nothing to do.
        app.run_command(Command::ToggleSplit);
        settle(&mut app, &ctx);
        select_named(&mut app, "file.txt");
        app.run_command(Command::CopyToOtherPane);
        assert!(app.running.is_none());

        // Right pane into beta, then back to the left pane.
        app.tabs[app.active_tab].active = 1;
        app.pane_mut().navigate(folder.0.join("beta"));
        settle(&mut app, &ctx);
        app.tabs[app.active_tab].active = 0;

        select_named(&mut app, "file.txt");
        app.run_command(Command::CopyToOtherPane);
        settle_op(&mut app, &ctx);
        assert!(folder.0.join("file.txt").exists());
        assert!(folder.0.join("beta/file.txt").exists());
        // The copy is selected on the right.
        let right = &app.tabs[app.active_tab].panes[1];
        assert!(right.selection.is_selected(&folder.0.join("beta/file.txt")));

        select_named(&mut app, "alpha");
        app.run_command(Command::MoveToOtherPane);
        settle_op(&mut app, &ctx);
        assert!(!folder.0.join("alpha").exists());
        assert!(folder.0.join("beta/alpha").is_dir());
        // Both panes show the change.
        let tab = &app.tabs[app.active_tab];
        assert_eq!(tab.panes[0].visible.len(), 2);
        assert_eq!(tab.panes[1].visible.len(), 2);
    }

    #[test]
    fn search_row_opens_and_escape_closes_it() {
        let folder = Folder::new("search");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);

        frame(&mut app, &ctx, key(Key::F, Modifiers::COMMAND));
        frame(&mut app, &ctx, Vec::new());
        assert!(app.pane().search_open);
        // The search box has focus, so Escape reaches it and closes the row.
        frame(&mut app, &ctx, key(Key::Escape, Modifiers::NONE));
        frame(&mut app, &ctx, Vec::new());
        assert!(!app.pane().search_open);
    }
}
