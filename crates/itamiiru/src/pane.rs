//! One folder view: its Back/Forward history, the listing, the selection
//! and the search. A tab holds one pane, or two side by side (split view).
//! Each pane has its own worker channels, so results never cross panes.

use std::collections::HashSet;
use std::ffi::{OsStr, OsString};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Key};
use itamiiru_core::entry::Entry;
use itamiiru_core::fileops::Op;
use itamiiru_core::glob::Glob;
use itamiiru_core::listing::{self, ListMsg, MtimeMsg};
use itamiiru_core::search::{self, SearchMsg};
use itamiiru_core::sort::{self, SortOrder, type_label};

use crate::address::{AddressAction, AddressBar};
use crate::details::Rename;
use crate::icons::{Arrow, ButtonIcon, arrow_button, icon_button};
use crate::iconview::KeySteps;
use crate::picker::Picker;
use crate::selection::{Selection, TypeAhead};
use crate::theme::{self, Theme};

/// How often to check whether the open folder changed.
pub const CHANGE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// Searching starts this long after the last key typed in the search box.
const SEARCH_DELAY: f64 = 0.3;

/// A place in the Back/Forward history.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Location {
    Folder(PathBuf),
    ThisPc,
}

/// A search running or finished in this pane's folder. While one exists,
/// `entries` holds its results instead of the folder's contents.
pub struct ActiveSearch {
    pub query: String,
    cancel: Arc<AtomicBool>,
    pub running: bool,
    /// Stopped at a limit before searching everything.
    pub truncated: bool,
}

/// What to do once the next listing of the current folder completes.
#[derive(Default)]
pub struct AfterListing {
    /// Select these (e.g. just pasted or renamed items).
    pub select: Vec<PathBuf>,
    /// Start renaming this one (a new folder).
    pub rename: Option<PathBuf>,
}

/// What decides which entries a pane shows, and in what order. The app
/// holds these; every pane uses the same ones.
pub struct Filter<'a> {
    pub sort: SortOrder,
    pub show_hidden: bool,
    /// In picker mode, only folders and files the picker's filter accepts.
    pub picker: Option<&'a Picker>,
}

pub struct Pane {
    /// Keeps this pane's widgets apart from the other pane's in egui.
    pub id: u64,

    /// The folder being shown.
    pub current: PathBuf,
    back: Vec<Location>,
    forward: Vec<Location>,
    /// Showing the This PC page instead of `current`.
    pub this_pc: bool,
    /// The selected tile on the This PC page.
    pub this_pc_selected: Option<PathBuf>,
    pub address: AddressBar,

    /// The current folder's entries, kept sorted, hidden ones included.
    pub entries: Vec<Entry>,
    /// Indices into `entries` of the rows on screen, top to bottom.
    pub visible: Vec<usize>,
    /// The longest Type text among the visible rows, which sizes the
    /// Details view's Type column. Found when the rows change, not per frame.
    pub widest_type: String,

    /// Bumped on every navigation or refresh; results from older listings are ignored.
    generation: u64,
    pub loading: bool,
    /// A refresh collects entries here and swaps them in when complete, so
    /// the list doesn't flash empty. A navigation streams into `entries`.
    refreshing: bool,
    pending: Vec<Entry>,
    pub error: Option<String>,
    /// Entries the last listing couldn't read.
    pub skipped: usize,

    pub selection: Selection,
    /// What a drag-to-select box adds to: the selection when it started
    /// with Ctrl held, else nothing.
    band_base: HashSet<PathBuf>,
    /// A drag-to-select box is being drawn; the app clears this once the
    /// button is up.
    pub banding: bool,
    type_ahead: TypeAhead,
    /// Row to scroll into view on the next frame.
    pub scroll_to: Option<usize>,
    /// How the arrow keys move in the current view, from the last frame.
    pub key_steps: KeySteps,

    /// The folder's modified time when it was last listed.
    dir_mtime: Option<SystemTime>,
    mtime_check_running: bool,
    last_mtime_check: f64,

    pub rename: Option<Rename>,
    pub after_listing: AfterListing,

    /// The search row under the address bar is showing.
    pub search_open: bool,
    search_text: String,
    /// Focus the search box next frame.
    focus_search: bool,
    /// When the search text last changed; the search starts SEARCH_DELAY later.
    search_edited_at: Option<f64>,
    pub search: Option<ActiveSearch>,

    /// The filter row under the address bar is showing. Its text hides
    /// the names that don't match, without touching the disk.
    pub filter_open: bool,
    filter_text: String,
    /// Focus the filter box next frame.
    focus_filter: bool,

    /// Set when the list was emptied for another folder or a search. The
    /// app then drops queued thumbnail jobs, and scans drives for This PC.
    pub navigated: bool,

    // Every worker thread gets a clone of a Sender; results come back on the Receivers.
    list_tx: Sender<ListMsg>,
    list_rx: Receiver<ListMsg>,
    mtime_tx: Sender<MtimeMsg>,
    mtime_rx: Receiver<MtimeMsg>,
    search_tx: Sender<SearchMsg>,
    search_rx: Receiver<SearchMsg>,
    ctx: egui::Context,
}

impl Pane {
    /// A pane showing `start` (or This PC). It starts listing right away.
    pub fn new(id: u64, ctx: egui::Context, start: PathBuf, this_pc: bool) -> Pane {
        let (list_tx, list_rx) = mpsc::channel();
        let (mtime_tx, mtime_rx) = mpsc::channel();
        let (search_tx, search_rx) = mpsc::channel();
        let mut pane = Pane {
            id,
            current: itamiiru_core::pathnorm::normalize(&start),
            back: Vec::new(),
            forward: Vec::new(),
            this_pc: false,
            this_pc_selected: None,
            address: AddressBar::default(),
            entries: Vec::new(),
            visible: Vec::new(),
            widest_type: String::new(),
            generation: 0,
            loading: false,
            refreshing: false,
            pending: Vec::new(),
            error: None,
            skipped: 0,
            selection: Selection::default(),
            band_base: HashSet::new(),
            banding: false,
            type_ahead: TypeAhead::default(),
            scroll_to: None,
            key_steps: KeySteps {
                vertical: 1,
                horizontal: 0,
            },
            dir_mtime: None,
            mtime_check_running: false,
            last_mtime_check: 0.0,
            rename: None,
            after_listing: AfterListing::default(),
            search_open: false,
            search_text: String::new(),
            focus_search: false,
            search_edited_at: None,
            search: None,
            filter_open: false,
            filter_text: String::new(),
            focus_filter: false,
            navigated: false,
            list_tx,
            list_rx,
            mtime_tx,
            mtime_rx,
            search_tx,
            search_rx,
            ctx,
        };
        if this_pc {
            pane.enter_this_pc();
        } else {
            pane.load();
        }
        return pane;
    }

    /// The tab's title: the folder's name.
    pub fn title(&self) -> String {
        if self.this_pc {
            return "This PC".to_string();
        }
        return match self.current.file_name() {
            Some(name) => name.to_string_lossy().into_owned(),
            None => self.current.to_string_lossy().into_owned(),
        };
    }

    /// Where we are, for the history.
    fn location(&self) -> Location {
        if self.this_pc {
            return Location::ThisPc;
        }
        return Location::Folder(self.current.clone());
    }

    /// Goes to `path`, remembering where we were for Back. `.` and `..` in
    /// it are worked out first, wherever the path came from.
    pub fn navigate(&mut self, path: PathBuf) {
        let path = itamiiru_core::pathnorm::normalize(&path);
        if self.search.is_some() {
            self.end_search();
            if !self.this_pc && path == self.current {
                // Clicking the searched folder itself: back to its contents.
                self.load();
                return;
            }
        }
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
    pub fn show_this_pc(&mut self) {
        if self.this_pc {
            return;
        }
        self.back.push(self.location());
        self.forward.clear();
        self.enter_this_pc();
    }

    fn enter_this_pc(&mut self) {
        self.end_search();
        self.this_pc = true;
        self.this_pc_selected = None;
        self.rename = None;
        self.selection.clear();
        // The app scans for fresh usage numbers for the tiles.
        self.navigated = true;
    }

    /// Goes somewhere from the history, without adding to it.
    fn go_to_location(&mut self, location: Location) {
        self.end_search();
        match location {
            Location::Folder(path) => {
                self.this_pc = false;
                self.current = path;
                self.load();
            }
            Location::ThisPc => self.enter_this_pc(),
        }
    }

    pub fn can_go_back(&self) -> bool {
        return !self.back.is_empty();
    }

    pub fn can_go_forward(&self) -> bool {
        return !self.forward.is_empty();
    }

    pub fn go_back(&mut self) {
        if let Some(location) = self.back.pop() {
            self.forward.push(self.location());
            self.go_to_location(location);
        }
    }

    pub fn go_forward(&mut self) {
        if let Some(location) = self.forward.pop() {
            self.back.push(self.location());
            self.go_to_location(location);
        }
    }

    /// Up one folder, with the folder we came from selected once the
    /// parent is listed, so the arrow keys carry on from there. Up from the
    /// top of the filesystem goes to This PC, like going up from C:\ in
    /// Explorer.
    pub fn go_up(&mut self) {
        if self.this_pc {
            return;
        }
        let Some(parent) = self.current.parent() else {
            self.show_this_pc();
            return;
        };
        let parent = parent.to_path_buf();
        let came_from = self.current.clone();
        self.navigate(parent);
        // Not there when hidden files are hidden, or it was just deleted;
        // then nothing gets selected.
        self.after_listing.select = vec![came_from];
    }

    /// Starts listing the current folder from scratch. Any older listing
    /// still running is left alone; its results carry an old generation.
    pub fn load(&mut self) {
        self.navigated = true;
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
    pub fn refresh(&mut self, filter: &Filter) {
        if self.this_pc {
            return;
        }
        if let Some(active) = &self.search {
            let query = active.query.clone();
            self.start_search(query, filter.show_hidden);
            return;
        }
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
    /// Returns an error to show, if starting a rename after it failed.
    pub fn receive_listing(&mut self, filter: &Filter) -> Option<String> {
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
                    sort::sort_entries(&mut entries, filter.sort.column, filter.sort.ascending);
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
            self.sort_and_filter(filter);
        }
        if finished {
            return self.apply_after_listing();
        }
        return None;
    }

    /// Selects just-created items, and starts renaming a new folder.
    fn apply_after_listing(&mut self) -> Option<String> {
        let after = std::mem::take(&mut self.after_listing);
        if !after.select.is_empty() {
            let mut selection = std::mem::take(&mut self.selection);
            let paths = self.visible_paths();
            self.scroll_to = selection.select_paths(&paths, &after.select);
            self.selection = selection;
        }
        if let Some(path) = after.rename {
            return self.start_rename(&path).err();
        }
        return None;
    }

    /// Re-sorts `entries` and rebuilds `visible` from it.
    pub fn sort_and_filter(&mut self, filter: &Filter) {
        sort::sort_entries(&mut self.entries, filter.sort.column, filter.sort.ascending);
        let names = NameFilter::new(&self.filter_text);
        self.visible.clear();
        self.widest_type.clear();
        for (i, entry) in self.entries.iter().enumerate() {
            if !filter.show_hidden && entry.hidden {
                continue;
            }
            if !names.matches(&entry.name) {
                continue;
            }
            if let Some(picker) = filter.picker
                && !picker.shows(entry)
            {
                continue;
            }
            self.visible.push(i);
            let label = type_label(entry);
            if label.chars().count() > self.widest_type.chars().count() {
                self.widest_type = label;
            }
        }
        // Drop selected rows that disappeared (deleted, or now hidden).
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        selection.retain_visible(&paths);
        self.selection = selection;
    }

    /// Hidden files were switched on or off.
    pub fn hidden_changed(&mut self, filter: &Filter) {
        if let Some(active) = &self.search {
            // Hidden matches were never collected; search again.
            let query = active.query.clone();
            self.start_search(query, filter.show_hidden);
            return;
        }
        self.sort_and_filter(filter);
    }

    /// The visible rows' paths, top to bottom. Built only when needed
    /// (a click or key), not every frame.
    pub fn visible_paths(&self) -> Vec<&Path> {
        let mut paths = Vec::with_capacity(self.visible.len());
        for &i in &self.visible {
            paths.push(self.entries[i].path.as_path());
        }
        return paths;
    }

    /// Searches the current folder and everything below it for `text`.
    /// Results replace the file list until the search is cleared.
    fn start_search(&mut self, text: String, show_hidden: bool) {
        if let Some(active) = &self.search {
            active.cancel.store(true, Ordering::Relaxed);
        }
        // A new generation also makes any running listing's results stale.
        self.generation += 1;
        self.navigated = true;
        self.entries.clear();
        self.visible.clear();
        self.selection.clear();
        self.rename = None;
        self.error = None;
        self.loading = false;
        self.refreshing = false;
        let query = search::Query {
            root: self.current.clone(),
            text: text.clone(),
            show_hidden,
            limits: search::Limits::default(),
        };
        let ctx = self.ctx.clone();
        let wake: listing::Wake = Box::new(move || ctx.request_repaint());
        let cancel = search::start(query, self.generation, self.search_tx.clone(), wake);
        self.search = Some(ActiveSearch {
            query: text,
            cancel,
            running: true,
            truncated: false,
        });
    }

    /// Leaves search mode without re-listing (navigation lists anyway).
    fn end_search(&mut self) {
        if let Some(active) = self.search.take() {
            active.cancel.store(true, Ordering::Relaxed);
        }
        self.search_text.clear();
        self.search_edited_at = None;
    }

    /// Escape, or an emptied search box: back to the folder's contents.
    pub fn clear_search(&mut self) {
        let was_searching = self.search.is_some();
        self.end_search();
        if was_searching {
            self.load();
        }
    }

    /// Ctrl+F or the search button: shows the search row and focuses it.
    pub fn open_search(&mut self) {
        if self.this_pc {
            return;
        }
        self.search_open = true;
        self.focus_search = true;
    }

    /// Hides the search row, and the results with it.
    pub fn close_search(&mut self) {
        self.search_open = false;
        self.clear_search();
    }

    /// The filter button: shows the filter row and focuses it.
    pub fn open_filter(&mut self) {
        if self.this_pc {
            return;
        }
        self.filter_open = true;
        self.focus_filter = true;
    }

    /// Hides the filter row and shows every name again.
    pub fn close_filter(&mut self, filter: &Filter) {
        self.filter_open = false;
        if !self.filter_text.is_empty() {
            self.filter_text.clear();
            self.sort_and_filter(filter);
        }
    }

    /// Names are being filtered right now.
    pub fn is_filtered(&self) -> bool {
        return self.filter_open && !self.filter_text.trim().is_empty();
    }

    pub fn receive_search(&mut self, filter: &Filter) {
        let mut needs_sort = false;
        while let Ok(msg) = self.search_rx.try_recv() {
            match msg {
                SearchMsg::Batch {
                    generation,
                    mut entries,
                } if generation == self.generation && self.search.is_some() => {
                    sort::sort_entries(&mut entries, filter.sort.column, filter.sort.ascending);
                    self.entries.append(&mut entries);
                    needs_sort = true;
                }
                SearchMsg::Finished {
                    generation,
                    truncated,
                } if generation == self.generation => {
                    if let Some(active) = &mut self.search {
                        active.running = false;
                        active.truncated = truncated;
                    }
                }
                // From an older search.
                _ => {}
            }
        }
        if needs_sort {
            self.sort_and_filter(filter);
        }
    }

    /// Starts (or clears) the search a moment after typing stops.
    pub fn check_search_typing(&mut self, ctx: &egui::Context, filter: &Filter) {
        let Some(edited_at) = self.search_edited_at else {
            return;
        };
        let now = ctx.input(|i| i.time);
        let waited = now - edited_at;
        if waited < SEARCH_DELAY {
            ctx.request_repaint_after(Duration::from_secs_f64(SEARCH_DELAY - waited));
            return;
        }
        self.search_edited_at = None;
        let text = self.search_text.trim().to_string();
        if text.is_empty() {
            let was_searching = self.search.is_some();
            if let Some(active) = self.search.take() {
                active.cancel.store(true, Ordering::Relaxed);
            }
            if was_searching {
                self.load();
            }
            return;
        }
        if !self.this_pc {
            self.start_search(text, filter.show_hidden);
        }
    }

    /// The selected entries, in display order.
    pub fn selected_entries(&self) -> Vec<&Entry> {
        let mut selected = Vec::new();
        for &i in &self.visible {
            let entry = &self.entries[i];
            if self.selection.is_selected(&entry.path) {
                selected.push(entry);
            }
        }
        return selected;
    }

    pub fn selected_paths(&self) -> Vec<PathBuf> {
        let mut paths = Vec::new();
        for entry in self.selected_entries() {
            paths.push(entry.path.clone());
        }
        return paths;
    }

    /// The entry on a visible row.
    pub fn entry_at(&self, row: usize) -> Option<&Entry> {
        let index = *self.visible.get(row)?;
        return Some(&self.entries[index]);
    }

    /// `single` (a picker for one file) allows only one selected row.
    pub fn click_row(&mut self, row: usize, ctrl: bool, shift: bool, single: bool) {
        let ctrl = ctrl && !single;
        let shift = shift && !single;
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

    /// A drag-to-select box started. With `add` (Ctrl) the selection so far
    /// stays; otherwise it's cleared.
    pub fn band_start(&mut self, add: bool) {
        self.banding = true;
        if add {
            self.band_base = self.selection.selected_set().clone();
        } else {
            self.band_base.clear();
            self.selection.clear();
        }
    }

    /// The drag-to-select box now touches `rows`.
    pub fn band(&mut self, rows: &[usize]) {
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        selection.band(&paths, &self.band_base, rows);
        self.selection = selection;
    }

    /// Right-click or drag on a row: like Explorer, an unselected row
    /// becomes the only selection; a selected one keeps the whole selection.
    pub fn select_row_if_unselected(&mut self, row: usize) {
        let Some(entry) = self.entry_at(row) else {
            return;
        };
        if !self.selection.is_selected(&entry.path) {
            self.click_row(row, false, false, false);
        }
    }

    pub fn move_cursor(&mut self, delta: isize, extend: bool) {
        let mut selection = std::mem::take(&mut self.selection);
        let paths = self.visible_paths();
        self.scroll_to = selection.move_cursor(&paths, delta, extend);
        self.selection = selection;
    }

    pub fn select_all(&mut self) {
        let mut selection = std::mem::take(&mut self.selection);
        selection.select_all(&self.visible_paths());
        self.selection = selection;
    }

    pub fn type_ahead(&mut self, text: &str, now: f64) {
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
            self.click_row(row, false, false, false);
            self.scroll_to = Some(row);
        }
    }

    /// Only a single selection is previewed.
    pub fn previewed(&self) -> Option<&Path> {
        if self.selection.count() == 1
            && let Some(cursor) = self.selection.cursor()
            && self.selection.is_selected(cursor)
        {
            return Some(cursor);
        }
        return None;
    }

    /// Turns the address bar's text into a path: `~` is the home folder,
    /// relative paths are relative to the current folder, and `.` and `..`
    /// are worked out (`..` is the parent folder).
    pub fn parse_address(&self, text: &str) -> PathBuf {
        use itamiiru_core::pathnorm::normalize;
        let text = text.trim();
        if (text == "~" || text.starts_with("~/"))
            && let Some(home) = itamiiru_core::platform::home_dir()
        {
            return normalize(&home.join(text.trim_start_matches('~').trim_start_matches('/')));
        }
        return normalize(&self.current.join(text));
    }

    /// Ctrl+L: edit the address as text.
    pub fn edit_address(&mut self) {
        if self.this_pc {
            self.address.start_editing(None);
        } else {
            self.address.start_editing(Some(&self.current));
        }
    }

    /// F2: edits the name in place. Fails for names that aren't valid text.
    pub fn start_rename(&mut self, path: &Path) -> Result<(), String> {
        let mut entry_name = None;
        for entry in &self.entries {
            if entry.path == path {
                entry_name = Some(entry.name.clone());
                break;
            }
        }
        let Some(name) = entry_name else {
            return Ok(());
        };
        let Some(text) = name.to_str() else {
            // Editing would replace the invalid bytes; don't risk it.
            let shown = name.to_string_lossy();
            return Err(format!(
                "{shown}: this name isn't valid text; rename it in a terminal"
            ));
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
        return Ok(());
    }

    /// The rename box closed. Returns the rename to run, if any.
    pub fn finish_rename(&mut self, commit: bool) -> Option<Op> {
        let rename = self.rename.take()?;
        if !commit {
            return None;
        }
        let new_name = OsString::from(rename.text.trim());
        if Some(new_name.as_os_str()) == rename.path.file_name() {
            return None;
        }
        return Some(Op::Rename {
            from: rename.path,
            new_name,
        });
    }

    /// Every couple of seconds, checks on a worker whether the folder's
    /// modified time changed. The app decides whether checks are on.
    pub fn check_for_changes(&mut self, now: f64) {
        if self.loading || self.error.is_some() || self.this_pc || self.search.is_some() {
            return;
        }
        // Keep frames coming while idle, so this check keeps running.
        self.ctx.request_repaint_after(CHANGE_CHECK_INTERVAL);
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

    pub fn receive_mtime(&mut self, filter: &Filter) {
        while let Ok(msg) = self.mtime_rx.try_recv() {
            self.mtime_check_running = false;
            if msg.generation == self.generation && msg.mtime != self.dir_mtime {
                itamiiru_core::debug!("folder changed, refreshing: {}", self.current.display());
                self.refresh(filter);
            }
        }
    }

    /// Back, Forward, Up, the address bar and the search and filter
    /// buttons, above the file list; then the search and filter rows when
    /// they're open.
    pub fn nav_bar(&mut self, ui: &mut egui::Ui, theme: &Theme, filter: &Filter) {
        ui.horizontal(|ui| {
            let back = ui.add_enabled_ui(self.can_go_back(), |ui| arrow_button(ui, Arrow::Left));
            if back.inner.on_hover_text("Back (Alt+Left)").clicked() {
                self.go_back();
            }
            let forward =
                ui.add_enabled_ui(self.can_go_forward(), |ui| arrow_button(ui, Arrow::Right));
            if forward.inner.on_hover_text("Forward (Alt+Right)").clicked() {
                self.go_forward();
            }
            let up = ui.add_enabled_ui(!self.this_pc, |ui| arrow_button(ui, Arrow::Up));
            if up.inner.on_hover_text("Up (Alt+Up)").clicked() {
                self.go_up();
            }

            // The buttons sit at the right, laid out from the right edge;
            // the address bar fills the rest.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let names = ui.add_enabled_ui(!self.this_pc, |ui| {
                    icon_button(ui, ButtonIcon::Filter, self.filter_open)
                });
                let tip = "Filter the names in this folder (Ctrl+Shift+F)";
                if names.inner.on_hover_text(tip).clicked() {
                    if self.filter_open {
                        self.close_filter(filter);
                    } else {
                        self.open_filter();
                    }
                }
                let search = ui.add_enabled_ui(!self.this_pc, |ui| {
                    icon_button(ui, ButtonIcon::Search, self.search_open)
                });
                if search.inner.on_hover_text("Search (Ctrl+F)").clicked() {
                    if self.search_open {
                        self.close_search();
                    } else {
                        self.open_search();
                    }
                }
                let refresh = icon_button(ui, ButtonIcon::Refresh, false);
                if refresh.on_hover_text("Refresh (F5)").clicked() {
                    if self.this_pc {
                        // The app rescans drives for a pane that just
                        // (re)entered This PC.
                        self.navigated = true;
                    } else {
                        self.refresh(filter);
                    }
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    self.address_bar(ui, theme);
                });
            });
        });
        if self.search_open && !self.this_pc {
            ui.add_space(theme::GAP_SMALL);
            self.search_box(ui);
        }
        if self.filter_open && !self.this_pc {
            ui.add_space(theme::GAP_SMALL);
            self.filter_box(ui, filter);
        }
    }

    /// Filters as you type: it only hides rows, so there's no delay.
    fn filter_box(&mut self, ui: &mut egui::Ui, filter: &Filter) {
        let edit = egui::TextEdit::singleline(&mut self.filter_text)
            .id(ui.id().with("filter-box"))
            .hint_text("Filter names, e.g. report or *.jpg")
            .desired_width(f32::INFINITY);
        let response = ui.add(edit);
        if self.focus_filter {
            self.focus_filter = false;
            response.request_focus();
        }
        if response.changed() {
            self.sort_and_filter(filter);
        }
        if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
            self.close_filter(filter);
        }
    }

    fn address_bar(&mut self, ui: &mut egui::Ui, theme: &Theme) {
        let current = if self.this_pc {
            None
        } else {
            Some(self.current.clone())
        };
        match self.address.show(ui, current.as_deref(), theme) {
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
    }

    fn search_box(&mut self, ui: &mut egui::Ui) {
        let hint = match self.current.file_name() {
            Some(name) => format!("Search {}", name.to_string_lossy()),
            None => "Search".to_string(),
        };
        let edit = egui::TextEdit::singleline(&mut self.search_text)
            .id(ui.id().with("search-box"))
            .hint_text(hint)
            .desired_width(f32::INFINITY);
        let response = ui.add(edit);
        if self.focus_search {
            self.focus_search = false;
            response.request_focus();
        }
        let now = ui.input(|i| i.time);
        if response.changed() {
            self.search_edited_at = Some(now);
        }
        if response.lost_focus() {
            if ui.input(|i| i.key_pressed(Key::Escape)) {
                self.close_search();
            } else if ui.input(|i| i.key_pressed(Key::Enter)) {
                // Enter searches right away.
                self.search_edited_at = Some(now - SEARCH_DELAY);
            }
        }
    }
}

/// The filter row's text as a test on names. Case is ignored. With `*` or
/// `?` it's a glob matching the whole name (`*.jpg`); otherwise any name
/// containing the text matches.
enum NameFilter {
    All,
    Glob(Glob),
    Contains(String),
}

impl NameFilter {
    fn new(text: &str) -> NameFilter {
        let text = text.trim();
        if text.is_empty() {
            return NameFilter::All;
        }
        if Glob::has_wildcards(text) {
            return NameFilter::Glob(Glob::case_insensitive(text));
        }
        return NameFilter::Contains(text.to_lowercase());
    }

    fn matches(&self, name: &OsStr) -> bool {
        return match self {
            NameFilter::All => true,
            NameFilter::Glob(glob) => glob.matches(&name.to_string_lossy()),
            NameFilter::Contains(text) => name.to_string_lossy().to_lowercase().contains(text),
        };
    }
}

/// A closed tab or pane stops its search. Listings stop by themselves:
/// their next send fails once the pane's receiver is gone, but a search
/// that finds nothing would never send and walk on to the end.
impl Drop for Pane {
    fn drop(&mut self) {
        if let Some(active) = &self.search {
            active.cancel.store(true, Ordering::Relaxed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(filter: &str, name: &str) -> bool {
        return NameFilter::new(filter).matches(OsStr::new(name));
    }

    #[test]
    fn name_filter() {
        assert!(matches("", "anything"));
        assert!(matches("  ", "anything"));
        assert!(matches("port", "Report.PDF"));
        assert!(matches("REPORT", "report.pdf"));
        assert!(!matches("port", "photo.jpg"));
        assert!(matches("*.jpg", "Photo.JPG"));
        assert!(!matches("*.jpg", "photo.jpg.txt"));
        assert!(matches("img_??.png", "IMG_01.png"));
        assert!(matches("caf\u{e9}", "CAF\u{c9} menu.txt"));
    }
}
