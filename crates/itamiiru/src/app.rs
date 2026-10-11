//! The main window: toolbar, file list and status bar.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, SystemTime};

use eframe::egui::{self, Key, Modifiers};
use itamiiru_core::entry::Entry;
use itamiiru_core::listing::{self, ListMsg, MtimeMsg};
use itamiiru_core::sort::{self, SortColumn, SortOrder};

use crate::details::{self, TableAction};
use crate::icons::{Arrow, arrow_button};
use crate::selection::{Selection, TypeAhead};
use crate::theme::{self, Theme};
use crate::zoom;

/// How often to check whether the open folder changed (PLAN.md D9).
const CHANGE_CHECK_INTERVAL: Duration = Duration::from_secs(2);
/// How many rows Page Up / Page Down move.
const PAGE_ROWS: isize = 20;

pub struct App {
    theme: Theme,

    /// The folder being shown.
    current: PathBuf,
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// The address bar's text while it's being edited.
    address: String,

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
}

impl App {
    pub fn new(ctx: egui::Context, theme: Theme, start: PathBuf) -> App {
        let (list_tx, list_rx) = mpsc::channel();
        let (mtime_tx, mtime_rx) = mpsc::channel();
        let mut app = App {
            theme,
            current: start,
            back: Vec::new(),
            forward: Vec::new(),
            address: String::new(),
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
            dir_mtime: None,
            mtime_check_running: false,
            last_mtime_check: 0.0,
            list_tx,
            list_rx,
            mtime_tx,
            mtime_rx,
            ctx,
            click_fix: zoom::ClickPositionFix::default(),
        };
        app.load();
        return app;
    }

    /// Goes to `path`, remembering the current folder for Back.
    fn navigate(&mut self, path: PathBuf) {
        if path == self.current {
            return;
        }
        let previous = std::mem::replace(&mut self.current, path);
        self.back.push(previous);
        self.forward.clear();
        self.load();
    }

    fn go_back(&mut self) {
        if let Some(path) = self.back.pop() {
            let previous = std::mem::replace(&mut self.current, path);
            self.forward.push(previous);
            self.load();
        }
    }

    fn go_forward(&mut self) {
        if let Some(path) = self.forward.pop() {
            let previous = std::mem::replace(&mut self.current, path);
            self.back.push(previous);
            self.load();
        }
    }

    fn go_up(&mut self) {
        if let Some(parent) = self.current.parent() {
            self.navigate(parent.to_path_buf());
        }
    }

    /// Starts listing the current folder from scratch. Any older listing
    /// still running is left alone; its results carry an old generation.
    fn load(&mut self) {
        self.entries.clear();
        self.visible.clear();
        self.selection.clear();
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

    fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.sort_and_filter();
    }

    /// Opens the entry at a visible row. Files do nothing yet (picker mode is Phase 2).
    fn open_row(&mut self, row: usize) {
        let Some(&index) = self.visible.get(row) else {
            return;
        };
        let entry = &self.entries[index];
        if entry.is_dir() {
            self.navigate(entry.path.clone());
        }
    }

    /// Enter: opens the row the keyboard cursor is on.
    fn open_cursor(&mut self) {
        let Some(cursor) = self.selection.cursor() else {
            return;
        };
        let paths = self.visible_paths();
        let mut row = None;
        for (i, path) in paths.iter().enumerate() {
            if *path == cursor {
                row = Some(i);
                break;
            }
        }
        if let Some(row) = row {
            self.open_row(row);
        }
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
        // Leave keys alone while the user is typing in a text field.
        if ctx.egui_wants_keyboard_input() {
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

        ctx.input_mut(|input| {
            // egui ignores an extra Shift or Alt when matching a key, so the
            // more specific combination must be checked first.
            back = input.consume_key(Modifiers::ALT, Key::ArrowLeft);
            forward = input.consume_key(Modifiers::ALT, Key::ArrowRight);
            up = input.consume_key(Modifiers::ALT, Key::ArrowUp)
                || input.consume_key(Modifiers::NONE, Key::Backspace);
            open = input.consume_key(Modifiers::NONE, Key::Enter);
            select_all = input.consume_key(Modifiers::COMMAND, Key::A);
            toggle_hidden = input.consume_key(Modifiers::COMMAND, Key::H);
            refresh = input.consume_key(Modifiers::NONE, Key::F5);

            let moves = [
                (Key::ArrowDown, 1),
                (Key::ArrowUp, -1),
                (Key::PageDown, PAGE_ROWS),
                (Key::PageUp, -PAGE_ROWS),
                (Key::End, all_rows),
                (Key::Home, -all_rows),
            ];
            for (key, delta) in moves {
                if input.consume_key(Modifiers::SHIFT, key) {
                    cursor_move = Some((delta, true));
                } else if input.consume_key(Modifiers::NONE, key) {
                    cursor_move = Some((delta, false));
                }
            }

            // Letters typed into the list, for type-ahead. egui doesn't send
            // text events for Ctrl or Alt shortcuts.
            for event in &input.events {
                if let egui::Event::Text(text) = event {
                    typed.push_str(text);
                }
            }
            now = input.time;
        });

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
            self.open_cursor();
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
        if !focused || self.loading || self.error.is_some() {
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
            let can_go_up = self.current.parent().is_some();
            let up = ui.add_enabled_ui(can_go_up, |ui| arrow_button(ui, Arrow::Up));
            if up.inner.on_hover_text("Up (Alt+Up)").clicked() {
                self.go_up();
            }

            let edit = egui::TextEdit::singleline(&mut self.address)
                .desired_width(f32::INFINITY)
                .hint_text("Address");
            let response = ui.add(edit);
            if response.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                let path = self.parse_address(&self.address);
                self.navigate(path);
            }
            if !response.has_focus() {
                // Not being edited: show where we are. This also undoes
                // unfinished edits when the field loses focus or Escape is hit.
                self.address = self.current.to_string_lossy().into_owned();
            }
        });
    }

    fn status_bar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let count = self.visible.len();
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
                ui.add_space(theme::GAP_LARGE);
                let mut show_hidden = self.show_hidden;
                let checkbox = ui.checkbox(&mut show_hidden, "Hidden files");
                if checkbox.on_hover_text("Ctrl+H").changed() {
                    self.toggle_hidden();
                }
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
        self.handle_keys(ui.ctx());
        self.check_for_changes(ui.ctx());

        let bar_frame = egui::Frame::new()
            .fill(self.theme.surface_container_1)
            .inner_margin(theme::GAP as i8);

        egui::Panel::top("toolbar")
            .frame(bar_frame)
            .show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("status")
            .frame(bar_frame)
            .show(ui, |ui| self.status_bar(ui));

        egui::CentralPanel::default().show(ui, |ui| {
            if let Some(error) = &self.error {
                show_error(ui, &self.theme, &self.current, error);
                return;
            }
            let action = details::show(
                ui,
                &self.entries,
                &self.visible,
                &self.selection,
                self.sort,
                &self.theme,
                self.scroll_to.take(),
            );
            match action {
                Some(TableAction::Sort(column)) => self.set_sort(column),
                Some(TableAction::Click { row, ctrl, shift }) => self.click_row(row, ctrl, shift),
                Some(TableAction::Open(row)) => self.open_row(row),
                None => {}
            }
        });
    }
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
