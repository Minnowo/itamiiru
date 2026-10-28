//! The main window: tabs, toolbar, sidebar, the folder panes and the
//! status bar. Each tab holds one pane or two (split view); `pane.rs`
//! holds a pane's folder, listing and selection.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::Duration;

use eframe::egui::{self, Key, Modifiers, Sense};
use itamiiru_core::actions::{self, Action};
use itamiiru_core::config::Config;
use itamiiru_core::entry::Entry;
use itamiiru_core::fileops::Op;
use itamiiru_core::format::{format_local_time, format_size};
use itamiiru_core::listing;
use itamiiru_core::places::{self, DiskUsage, Places, PlacesMsg, Volume};
use itamiiru_core::platform::{DragOut, DragState};
use itamiiru_core::sort::{SortColumn, SortOrder, type_label};
use itamiiru_core::thumbcache::ThumbSize;

use crate::details::{self, TableAction, TableInput};
use crate::iconview::{self, KeySteps, ThumbSource};
use crate::menus::{Command, MenuState};
use crate::ops_ui::{self, RunningOp};
use crate::pane::{Filter, Pane};
use crate::picker::{FooterAction, Picker, ProbeOutcome};
use crate::preview::Preview;
use crate::settings::{DetailColumn, Settings, StatusField};
use crate::settings_page::{self, Options};
use crate::sidebar::{self, SidebarAction, SidebarDrop};
use crate::startup;
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
/// How long a notice ("safe to remove") stays in the status bar, in seconds.
const NOTICE_SECONDS: f64 = 8.0;

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
    /// Removable volumes being mounted or ejected (their ids).
    volumes_busy: HashSet<String>,
    /// A short message for the status bar, and when it was shown.
    notice: Option<(String, f64)>,

    /// The config file's contents, kept so unknown keys and comments survive.
    config: Config,
    config_path: Option<PathBuf>,
    /// The settings as last written, and as of this frame.
    saved_settings: Settings,
    latest_settings: Settings,
    settings_changed_at: Option<f64>,
    sidebar_width: f32,
    /// Which columns the Details view shows.
    details_columns: Vec<DetailColumn>,
    auto_refresh_folder: bool,
    auto_refresh_drives: bool,
    thumbnails_enabled: bool,
    thumbs: Thumbs,
    preview_pane: bool,
    /// The preview pane hides while there's nothing to preview.
    preview_auto_hide: bool,
    preview_width: f32,
    preview: Preview,
    /// What the status bar shows for one selected item.
    status_fields: Vec<StatusField>,
    /// The settings page is open.
    settings_open: bool,
    /// Delete moves to the trash (setting `use_trash`).
    use_trash: bool,
    /// Files can be dragged out to other apps (setting `drag_and_drop`).
    drag_and_drop: bool,
    /// Dragging from empty space selects with a box (setting `drag_select`).
    drag_select: bool,
    /// Dragged files can be dropped onto folders and the other pane
    /// (setting `drag_within`).
    drag_within: bool,
    /// Custom right-click actions, from the `actions` file next to the
    /// config file (read at startup).
    actions: Vec<Action>,
    /// Items being dragged. Let go over a folder or the other pane, they
    /// move or copy there; once the pointer leaves the window with the
    /// button still down, they're dragged out to other apps.
    drag_out: Option<Vec<PathBuf>>,
    /// The folder the dragged items would be dropped into, as of the last
    /// frame.
    drop_target: Option<PathBuf>,
    /// Files another app is dragging over the window this frame, or just
    /// dropped (`drag_in_dropped`). Set in `raw_input_hook`.
    drag_in: Option<Vec<PathBuf>>,
    drag_in_dropped: bool,
    /// The platform said where the pointer is, so the drop goes where it
    /// points; otherwise into the active pane's folder.
    drag_in_pointer: bool,
    /// A drag has been over this tab since this time (egui seconds); held
    /// there long enough, the tab opens so the items can be dropped in it.
    drag_over_tab: Option<(usize, f64)>,
    /// Drags over the side pane's places and drives.
    sidebar_drop: SidebarDrop,
    /// Starts that drag. A field so tests can watch it instead of
    /// dragging on the real screen.
    start_drag_out: fn(Vec<PathBuf>) -> Result<DragOut, String>,
    /// Where the pointer is on the screen, while another app drags files
    /// over the window. A field so tests don't read the real pointer.
    pointer_position: fn() -> Option<(f32, f32)>,
    /// The drag out in progress, for its cursor and Escape.
    dragging_out: Option<DragOut>,

    /// Set in picker mode (`itami --pick`). The picker has one tab and no
    /// split view.
    picker: Option<Picker>,
    /// Startup timing stages already logged: first frame started, its
    /// drawing done, and it being on screen.
    startup_logged: [bool; 3],
    /// The first folder's listing finished (startup timing).
    first_listing_logged: bool,
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
        let actions = load_actions(config_path.as_deref());
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
            volumes_busy: HashSet::new(),
            notice: None,
            config,
            config_path,
            saved_settings: settings.clone(),
            latest_settings: settings.clone(),
            settings_changed_at: None,
            sidebar_width: settings.sidebar_width,
            details_columns: settings.columns.clone(),
            auto_refresh_folder: settings.auto_refresh_folder,
            auto_refresh_drives: settings.auto_refresh_drives,
            thumbnails_enabled: settings.thumbnails,
            thumbs,
            preview_pane: settings.preview_pane,
            preview_auto_hide: settings.preview_auto_hide,
            preview_width: settings.preview_width,
            preview: Preview::new(),
            status_fields: settings.status_fields.clone(),
            settings_open: false,
            use_trash: settings.use_trash,
            drag_and_drop: settings.drag_and_drop,
            drag_select: settings.drag_select,
            drag_within: settings.drag_within,
            actions,
            drag_out: None,
            drop_target: None,
            drag_in: None,
            drag_in_dropped: false,
            drag_in_pointer: false,
            drag_over_tab: None,
            sidebar_drop: SidebarDrop::default(),
            start_drag_out: itamiiru_core::platform::start_drag_out,
            pointer_position: itamiiru_core::platform::pointer_position,
            dragging_out: None,
            picker,
            startup_logged: [false; 3],
            first_listing_logged: false,
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

    /// A drag started on a row: like Explorer, an unselected row becomes
    /// the selection; the selection is what gets dragged.
    fn drag_start(&mut self, row: usize) {
        if !self.drag_within && !self.drag_and_drop {
            return;
        }
        self.pane_mut().select_row_if_unselected(row);
        let paths = self.pane().selected_paths();
        if !paths.is_empty() {
            self.drag_out = Some(paths);
        }
    }

    /// Hands a drag over to the system once the pointer leaves the window
    /// with the button still down, so it can be dropped in another app.
    /// While it goes on, the cursor shows whether the window under it
    /// takes the drop, and Escape cancels. Runs before the other keys, so
    /// that Escape does nothing else.
    fn check_drag_out(&mut self, ctx: &egui::Context) {
        if let Some(drag) = &self.dragging_out {
            let state = drag.state();
            if state == DragState::Ended {
                self.dragging_out = None;
                return;
            }
            if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
                drag.cancel();
            }
            let icon = match state {
                DragState::Accepted => egui::CursorIcon::Copy,
                _ => egui::CursorIcon::NoDrop,
            };
            ctx.set_cursor_icon(icon);
            // The state changes on the drag thread; keep looking.
            ctx.request_repaint_after(Duration::from_millis(30));
            return;
        }
        if self.drag_out.is_none() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::Escape)) {
            self.drag_out = None;
            return;
        }
        let modifiers = ctx.input(|i| i.modifiers);
        let (down, pos) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.latest_pos()));
        if !down {
            // Let go inside the window: dropped on a folder or the other
            // pane, or nowhere.
            let sources = self.drag_out.take();
            if let (Some(sources), Some(dest_dir)) = (sources, self.drop_target.take()) {
                self.drop_into(sources, dest_dir, modifiers);
            }
            return;
        }
        let icon = match (&self.drag_out, &self.drop_target) {
            (Some(sources), Some(dest_dir)) if self.drop_copies(sources, dest_dir, modifiers) => {
                egui::CursorIcon::Copy
            }
            _ => egui::CursorIcon::Grabbing,
        };
        ctx.set_cursor_icon(icon);
        let inside = pos.is_some_and(|pos| ctx.content_rect().contains(pos));
        if inside || !self.drag_and_drop {
            return;
        }
        let Some(paths) = self.drag_out.take() else {
            return;
        };
        match (self.start_drag_out)(paths) {
            Ok(drag) => self.dragging_out = Some(drag),
            Err(err) => itamiiru_core::debug!("drag out: {err}"),
        }
    }

    /// Notices files another app drags over the window or drops on it.
    /// While it drags, the pointer is the other app's, so its position is
    /// asked of the platform and fed to egui as a pointer move: then the
    /// views find the folder under it as for our own drags.
    fn notice_drag_in(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.drag_in = None;
        self.drag_in_dropped = false;
        self.drag_in_pointer = false;
        if !self.drag_and_drop {
            return;
        }
        let mut paths = Vec::new();
        let dropped = !raw.dropped_files.is_empty();
        if dropped {
            for file in &raw.dropped_files {
                let path = file.path();
                if !path.as_os_str().is_empty() {
                    paths.push(path.to_path_buf());
                }
            }
        } else {
            for file in &raw.hovered_files {
                if let Some(path) = &file.path {
                    paths.push(path.clone());
                }
            }
        }
        if paths.is_empty() {
            return;
        }
        let inner_rect = match raw.viewports.get(&raw.viewport_id) {
            Some(info) if info.inner_rect.is_some() => info.inner_rect,
            _ => ctx.input(|i| i.viewport().inner_rect),
        };
        if let Some((x, y)) = (self.pointer_position)()
            && let Some(inner_rect) = inner_rect
        {
            let ppp = ctx.pixels_per_point();
            let pos = egui::pos2(x / ppp, y / ppp) - inner_rect.min.to_vec2();
            raw.events.push(egui::Event::PointerMoved(pos));
            self.drag_in_pointer = true;
        }
        if !dropped {
            // Nothing else wakes us while it's dragged around.
            ctx.request_repaint_after(Duration::from_millis(30));
        }
        self.drag_in = Some(paths);
        self.drag_in_dropped = dropped;
    }

    /// Like Explorer: dropped items move within a drive and are copied to
    /// another one; Ctrl copies and Shift moves either way.
    fn drop_copies(&self, sources: &[PathBuf], dest_dir: &Path, modifiers: Modifiers) -> bool {
        if modifiers.command {
            return true;
        }
        if modifiers.shift {
            return false;
        }
        for source in sources {
            if !places::same_drive(&self.places.drives, source, dest_dir) {
                return true;
            }
        }
        return false;
    }

    fn drop_into(&mut self, sources: Vec<PathBuf>, dest_dir: PathBuf, modifiers: Modifiers) {
        if self.drop_copies(&sources, &dest_dir, modifiers) {
            self.start_op(Op::Copy { sources, dest_dir });
        } else {
            self.start_op(Op::Move { sources, dest_dir });
        }
    }

    /// The items being dragged over the window: ours (with `drag_within`)
    /// or another app's.
    fn dragged(&self) -> Option<&Vec<PathBuf>> {
        if self.drag_within
            && let Some(paths) = &self.drag_out
        {
            return Some(paths);
        }
        return self.drag_in.as_ref();
    }

    /// A drag held over another tab for a moment opens that tab, so the
    /// items can be dropped in it. `over` is the tab under the pointer.
    fn drag_over_tab(&mut self, ctx: &egui::Context, over: Option<usize>) {
        const HOLD: f64 = 0.5;
        let Some(index) = over else {
            self.drag_over_tab = None;
            return;
        };
        let now = ctx.input(|i| i.time);
        let since = match self.drag_over_tab {
            Some((tab, since)) if tab == index => since,
            _ => now,
        };
        if now - since >= HOLD && index < self.tabs.len() {
            self.active_tab = index;
            self.drag_over_tab = None;
            return;
        }
        self.drag_over_tab = Some((index, since));
        // The pointer may not move again; look back when it's time.
        ctx.request_repaint_after(Duration::from_secs_f64(HOLD - (now - since)));
    }

    /// Without the pointer's position, files from another app go into the
    /// active pane's folder.
    fn drag_in_fallback(&self) -> Option<PathBuf> {
        let sources = self.drag_in.as_ref()?;
        let pane = self.pane();
        if pane.this_pc || !can_drop(sources, &pane.current) {
            return None;
        }
        return Some(pane.current.clone());
    }

    /// Where the dragged items would go with the pointer over pane
    /// `index`: into the folder of the address bar crumb it's on, or of
    /// row `row` if that's a folder, else into the pane's folder. None
    /// where they can't go: into themselves, or back where they all are.
    fn drop_target_in(&self, index: usize, row: Option<usize>) -> Option<PathBuf> {
        let sources = self.dragged()?;
        let pane = &self.tabs[self.active_tab].panes[index];
        let mut dest_dir = pane.current.clone();
        if let Some(crumb) = &pane.address.drop_hover {
            dest_dir = crumb.clone();
        } else if pane.this_pc {
            return None;
        } else if let Some(entry) = row.and_then(|row| pane.entry_at(row))
            && entry.is_dir()
        {
            dest_dir = entry.path.clone();
        }
        if !can_drop(sources, &dest_dir) {
            return None;
        }
        return Some(dest_dir);
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
                if trash_supported() && self.use_trash {
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
            Command::OpenSettings => self.settings_open = true,
            Command::SetSort(sort) => self.set_sort_order(sort),
            Command::ActionOnSelection(index) => self.run_action(index, false),
            Command::ActionOnFolder(index) => self.run_action(index, true),
            Command::CopyToOtherPane => self.send_to_other_pane(false),
            Command::MoveToOtherPane => self.send_to_other_pane(true),
            Command::ToggleBookmark => self.toggle_bookmark(),
        }
    }

    /// Toggles bookmark on the selected folder, or the current folder.
    fn toggle_bookmark(&mut self) {
        let target = self.selected_folder().or_else(|| {
            if !self.pane().this_pc {
                Some(self.pane().current.clone())
            } else {
                None
            }
        });
        if let Some(path) = target {
            if self.places.is_bookmarked(&path) {
                let _ = itamiiru_core::places::remove_bookmark(&path);
            } else {
                let _ = itamiiru_core::places::add_bookmark(&path, None);
            }
            self.scan_places();
        }
    }

    /// Runs custom action `index` on the selection, or on the folder on
    /// screen (`on_folder`, from a right-click on empty space).
    fn run_action(&mut self, index: usize, on_folder: bool) {
        let Some(action) = self.actions.get(index) else {
            return;
        };
        let pane = self.tabs[self.active_tab].pane();
        let selected = if on_folder {
            Vec::new()
        } else {
            pane.selected_entries()
        };
        let mut items = Vec::new();
        for entry in &selected {
            items.push(actions::Item {
                path: &entry.path,
                is_dir: entry.is_dir(),
            });
        }
        let mut errors = Vec::new();
        for line in action.command_lines(&items, &pane.current) {
            if let Err(err) = actions::run(&line, &pane.current) {
                let program = line.first().map(|p| p.to_string_lossy().into_owned());
                let program = program.unwrap_or_default();
                errors.push(format!("{}: can't run {program}: {err}", action.name));
            }
        }
        for error in errors {
            ops_ui::push_error(&mut self.op_errors, error);
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
                PlacesMsg::Mounted { volume, result } => {
                    self.volumes_busy.remove(&volume.id);
                    match result {
                        Ok(point) => self.pane_mut().navigate(point),
                        Err(error) => {
                            let text = format!("{}: can't mount: {error}", volume.name);
                            ops_ui::push_error(&mut self.op_errors, text);
                        }
                    }
                    self.places_scan_running = false;
                    self.scan_places();
                }
                PlacesMsg::Ejected {
                    volume,
                    power_off,
                    result,
                } => {
                    self.volumes_busy.remove(&volume.id);
                    let verb = if power_off { "eject" } else { "unmount" };
                    match result {
                        Ok(()) if power_off => {
                            let text = format!("{} can be safely removed", volume.name);
                            self.show_notice(text);
                        }
                        Ok(()) => {
                            let text =
                                format!("{} unmounted; click it to mount it again", volume.name);
                            self.show_notice(text);
                        }
                        Err(error) => {
                            let text = format!("{}: can't {verb}: {error}", volume.name);
                            ops_ui::push_error(&mut self.op_errors, text);
                        }
                    }
                    self.places_scan_running = false;
                    self.scan_places();
                }
            }
        }
    }

    /// Mounts a plugged-in drive on a worker, then opens it in the active
    /// pane. One already mounted just opens.
    fn mount_volume(&mut self, volume: Volume) {
        if let Some(point) = volume.mount_point.clone() {
            self.pane_mut().navigate(point);
            return;
        }
        if !self.volumes_busy.insert(volume.id.clone()) {
            return;
        }
        let ctx = self.ctx.clone();
        let wake: listing::Wake = Box::new(move || ctx.request_repaint());
        places::start_mount(volume, self.places_tx.clone(), wake);
    }

    /// Unmounts a removable drive's filesystem on a worker (`power_off`
    /// false), or ejects the drive: unmounts everything on it and powers
    /// it off. Panes showing a folder on it move to This PC first, so
    /// nothing here is reading from it.
    fn eject_volume(&mut self, volume: Volume, power_off: bool) {
        if !self.volumes_busy.insert(volume.id.clone()) {
            return;
        }
        // Ejecting unmounts every volume on the same drive, not just this one.
        let mut points = Vec::new();
        for other in &self.places.volumes {
            let same_drive =
                power_off && !volume.drive_id.is_empty() && other.drive_id == volume.drive_id;
            if (same_drive || other.id == volume.id)
                && let Some(point) = &other.mount_point
            {
                points.push(point.clone());
            }
        }
        for tab in &mut self.tabs {
            for pane in &mut tab.panes {
                let inside = points.iter().any(|point| pane.current.starts_with(point));
                if !pane.this_pc && inside {
                    pane.show_this_pc();
                }
            }
        }
        let ctx = self.ctx.clone();
        let wake: listing::Wake = Box::new(move || ctx.request_repaint());
        places::start_eject(volume, power_off, self.places_tx.clone(), wake);
    }

    fn show_notice(&mut self, text: String) {
        let now = self.ctx.input(|i| i.time);
        self.notice = Some((text, now));
        self.ctx
            .request_repaint_after(Duration::from_secs_f64(NOTICE_SECONDS));
    }

    fn handle_keys(&mut self, ctx: &egui::Context) {
        // Leave keys alone while the user is typing in a text field, or a
        // dialog is waiting for an answer.
        let dialog_open = self.confirm_delete.is_some()
            || self.settings_open
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
        let mut go_home = false;

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
            if input.consume_key(Modifiers::COMMAND, Key::D) {
                commands.push(Command::ToggleBookmark);
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
            if input.consume_key(Modifiers::COMMAND, Key::Comma) {
                commands.push(Command::OpenSettings);
            }
            toggle_hidden = input.consume_key(Modifiers::COMMAND, Key::H);
            refresh = input.consume_key(Modifiers::NONE, Key::F5)
                || input.consume_key(Modifiers::COMMAND, Key::R);
            go_home = input.consume_key(Modifiers::ALT, Key::Home);
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

        if go_home && let Some(home) = itamiiru_core::platform::home_dir() {
            self.pane_mut().navigate(home);
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
        // Zoom controls first, from the right edge, so a long selection
        // summary is cut short instead of pushing them off the window.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            zoom::controls_right_to_left(ui);
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                let now = ui.input(|i| i.time);
                if let Some((text, shown_at)) = &self.notice
                    && now - shown_at < NOTICE_SECONDS
                {
                    ui.strong(text);
                    ui.separator();
                }
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
                if pane.loading {
                    ui.spinner();
                }
                if pane.skipped > 0 {
                    ui.weak(format!("({} could not be read)", pane.skipped));
                }
                if pane.selection.count() > 0
                    && let Some(summary) =
                        selection_summary(&pane.selected_entries(), &self.status_fields)
                {
                    ui.separator();
                    ui.add(egui::Label::new(summary).truncate());
                }
            });
        });
    }

    /// The central area: the operation error banner, then one pane, or
    /// two side by side with a drag handle between them.
    fn panes(&mut self, ui: &mut egui::Ui) {
        let dragging = self.dragged().is_some();
        for pane in &mut self.tabs[self.active_tab].panes {
            pane.address.dragging = dragging;
        }
        ops_ui::error_banner(ui, &self.theme, &mut self.op_errors);
        let full = ui.available_rect_before_wrap();
        let mut rects = vec![full];
        if self.tabs[self.active_tab].is_split() {
            let (left, handle, right) = self.split_handle(ui, full);
            rects = vec![left, right];
            ui.allocate_rect(handle, Sense::hover());
        }
        let mut outputs = Vec::new();
        let pane_rects = rects.clone();
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

        // During a drag: the pane under the pointer and the row in it.
        self.drop_target = None;
        let mut pointer = ui.input(|i| i.pointer.latest_pos());
        if self.drag_in.is_some() && !self.drag_in_pointer {
            // From before the drag came in; it says nothing.
            pointer = None;
        }
        for (index, output) in outputs.iter_mut().enumerate() {
            let mut row = None;
            if let Some(TableAction::DropHover(hovered)) = output.action {
                row = Some(hovered);
                output.action = None;
            }
            let rect = pane_rects[index];
            if self.dragged().is_none() || !pointer.is_some_and(|p| rect.contains(p)) {
                continue;
            }
            self.drop_target = self.drop_target_in(index, row);
            let pane = &self.tabs[self.active_tab].panes[index];
            let on_crumb = pane.address.drop_hover.is_some();
            if !on_crumb && self.drop_target.as_ref() == Some(&pane.current) {
                // Into the pane's own folder: tint the whole pane.
                ui.painter().rect_filled(
                    rect,
                    theme::RADIUS_MEDIUM,
                    self.theme.on_surface_hover_tint,
                );
            }
        }
        // Over a place or drive in the side pane (drawn before the panes).
        let pointer_known = self.drag_in.is_none() || self.drag_in_pointer;
        if pointer_known
            && let Some(place) = &self.sidebar_drop.hover
            && let Some(sources) = self.dragged()
            && can_drop(sources, place)
        {
            self.drop_target = Some(place.clone());
        }
        if self.drag_in.is_some() && !self.drag_in_pointer {
            self.drop_target = self.drag_in_fallback();
            let active = self.tabs[self.active_tab].active;
            if self.drop_target.is_some()
                && let Some(rect) = pane_rects.get(active)
            {
                ui.painter().rect_filled(
                    *rect,
                    theme::RADIUS_MEDIUM,
                    self.theme.on_surface_hover_tint,
                );
            }
        }
        for pane in &mut self.tabs[self.active_tab].panes {
            pane.address.drop_target = self.drop_target.clone();
        }
        self.sidebar_drop.target = self.drop_target.clone();
        // Files from another app are copied: the app may still need them.
        if self.drag_in_dropped
            && let Some(sources) = self.drag_in.take()
            && let Some(dest_dir) = self.drop_target.take()
        {
            self.start_op(Op::Copy { sources, dest_dir });
        }

        for (index, output) in outputs.into_iter().enumerate() {
            let used = output.activated || output.action.is_some() || output.tile.is_some();
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
        let single_select = self.single_select();
        if ui.input(|i| i.pointer.any_pressed()) && ui.rect_contains_pointer(ui.max_rect()) {
            out.activated = true;
        }
        let dragging = self.dragged().is_some();
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
                &self.volumes_busy,
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
        // Which custom actions apply is only worked out while a menu may
        // show (a right-click, or one already open), not every frame.
        let mut actions_on_selection = Vec::new();
        let mut actions_on_folder = Vec::new();
        let menu_may_show = ui
            .input(|i| i.pointer.secondary_down() || i.pointer.secondary_released())
            || egui::Popup::is_any_open(ui.ctx());
        if menu_may_show && !self.actions.is_empty() {
            let selected = pane.selected_entries();
            let mut items = Vec::new();
            for entry in &selected {
                items.push(actions::Item {
                    path: &entry.path,
                    is_dir: entry.is_dir(),
                });
            }
            for (index, action) in self.actions.iter().enumerate() {
                if !items.is_empty() && action.applies_to(&items) {
                    actions_on_selection.push((index, action.name.clone()));
                }
                if action.applies_to(&[]) {
                    actions_on_folder.push((index, action.name.clone()));
                }
            }
        }
        let selected_folder = selected_one_folder(pane);
        let target_bookmark_path = selected_folder.clone().or_else(|| {
            if !pane.this_pc {
                Some(pane.current.clone())
            } else {
                None
            }
        });
        let is_bookmarked = match &target_bookmark_path {
            Some(p) => self.places.is_bookmarked(p),
            None => false,
        };
        let menu = MenuState {
            selected: pane.selection.count(),
            can_paste: self.clipboard.is_some(),
            show_hidden: self.show_hidden,
            trash_supported: trash_supported() && self.use_trash,
            open_in_new_tab: self.picker.is_none() && selected_folder.is_some(),
            other_pane: other_name,
            other_pane_same: other_same,
            actions_on_selection,
            actions_on_folder,
            sort: self.sort,
            selected_folder: selected_folder.is_some(),
            is_bookmarked,
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
            columns: &self.details_columns,
            widest_type: &pane.widest_type,
            // A picker that returns one file has nothing to select a box of.
            drag_select: self.drag_select && !single_select,
            dragging,
            drop_target: self.drop_target.as_deref(),
        };
        let scroll_to = pane.scroll_to.take();
        out.action = match self.view {
            ViewMode::Details => {
                pane.key_steps = KeySteps {
                    vertical: 1,
                    horizontal: 0,
                };
                details::show(ui, &input, &mut pane.rename, scroll_to)
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
            Some(TileAction::Mount(volume)) => self.mount_volume(volume),
            Some(TileAction::Unmount(volume)) => self.eject_volume(volume, false),
            Some(TileAction::Eject(volume)) => self.eject_volume(volume, true),
            None => {}
        }
        match output.action {
            Some(TableAction::Sort(column)) => self.set_sort(column),
            Some(TableAction::Click { row, ctrl, shift }) => {
                let single = self.single_select();
                self.pane_mut().click_row(row, ctrl, shift, single);
            }
            Some(TableAction::Open(row)) => self.open_row(row),
            Some(TableAction::RightClick(row)) => self.pane_mut().select_row_if_unselected(row),
            Some(TableAction::MiddleClick(row)) => self.middle_click_row(row),
            Some(TableAction::DragStart(row)) => self.drag_start(row),
            Some(TableAction::Command(command)) => self.run_command(command),
            Some(TableAction::RenameDone { commit }) => self.finish_rename(commit),
            Some(TableAction::ClearSelection) => self.pane_mut().selection.clear(),
            Some(TableAction::BandStart { add }) => self.pane_mut().band_start(add),
            Some(TableAction::Band(rows)) => self.pane_mut().band(&rows),
            // Taken out in `panes`.
            Some(TableAction::DropHover(_)) => {}
            None => {}
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
            columns: self.details_columns.clone(),
            auto_refresh_folder: self.auto_refresh_folder,
            auto_refresh_drives: self.auto_refresh_drives,
            thumbnails: self.thumbnails_enabled,
            preview_pane: self.preview_pane,
            preview_auto_hide: self.preview_auto_hide,
            preview_width: self.preview_width,
            status_fields: self.status_fields.clone(),
            use_trash: self.use_trash,
            drag_and_drop: self.drag_and_drop,
            drag_select: self.drag_select,
            drag_within: self.drag_within,
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

impl App {
    /// A last save on the way out, if something changed since the last one.
    /// Writing here, on the UI thread, is fine: the window is closing.
    fn save_on_exit(&mut self) {
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
}

impl eframe::App for App {
    #[cfg(feature = "glow")]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.save_on_exit();
    }

    // eframe's on_exit has no GL context without glow.
    #[cfg(not(feature = "glow"))]
    fn on_exit(&mut self) {
        self.save_on_exit();
    }

    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.click_fix.fix(ctx.zoom_factor(), raw_input);
        self.notice_drag_in(ctx, raw_input);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // egui counts frames shown, and passes within a frame (a frame can
        // be laid out twice before it's shown).
        let frame_nr = ui.ctx().cumulative_frame_nr();
        let pass = ui.ctx().current_pass_index();
        if frame_nr == 0 && !self.startup_logged[0] {
            self.startup_logged[0] = true;
            startup::mark("first frame started");
            // Which GPU and backend (Vulkan or OpenGL) wgpu picked.
            #[cfg(feature = "wgpu")]
            if let Some(state) = _frame.wgpu_render_state() {
                let info = state.adapter.get_info();
                itamiiru_core::debug!(
                    "wgpu: {:?} on {} ({})",
                    info.backend,
                    info.name,
                    info.driver
                );
            }
        }
        if frame_nr == 0 && pass > 0 {
            startup::mark("first frame laid out again (extra pass)");
        }
        if frame_nr >= 1 && !self.startup_logged[2] {
            self.startup_logged[2] = true;
            startup::mark("first frame on screen (second frame started)");
        }
        if !self.first_listing_logged && !self.pane().loading && frame_nr >= 1 {
            self.first_listing_logged = true;
            startup::mark("folder listed");
        }
        self.handle_navigation();
        self.receive_panes();
        self.receive_op();
        self.receive_places();
        self.thumbs.receive(ui.ctx());
        self.picker_frame(ui.ctx());
        // Before the keys: Escape during a drag only cancels the drag.
        self.check_drag_out(ui.ctx());
        self.handle_keys(ui.ctx());
        self.check_for_changes(ui.ctx());
        self.check_places(ui.ctx());
        // Uses the sidebar and column widths measured last frame.
        self.persist_settings(ui.ctx());
        self.dialogs(ui.ctx());
        self.settings_page(ui.ctx());

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
                    let dragging = self.dragged().is_some();
                    tab_action =
                        tabs::tab_bar(ui, &self.tabs, self.active_tab, dragging, &self.theme);
                });
        }
        egui::Panel::top("toolbar")
            .frame(bar_frame)
            .show(ui, |ui| self.command_row(ui));
        let mut over_tab = None;
        match tab_action {
            Some(TabAction::Select(index)) => self.active_tab = index,
            Some(TabAction::Close(index)) => self.close_tab(index),
            Some(TabAction::New) => self.new_tab(None, true),
            Some(TabAction::DragOver(index)) => over_tab = Some(index),
            None => {}
        }
        self.drag_over_tab(ui.ctx(), over_tab);
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
        self.sidebar_drop.dragging = self.dragged().is_some();
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
                    &self.volumes_busy,
                    current,
                    self.show_hidden,
                    self.picker.is_none(),
                    &mut self.sidebar_drop,
                    &self.theme,
                );
            });
        self.sidebar_width = sidebar.response.rect.width();
        if let Some(action) = sidebar_action {
            self.apply_sidebar_action(action);
        }

        // While a drag-to-select box is drawn the preview stays as it is:
        // following each item it touches would load them all, and with
        // auto-hide the pane would open and close under the pointer.
        let pointer_down = ui.input(|i| i.pointer.primary_down());
        let mut banding = false;
        for pane in &mut self.tabs[self.active_tab].panes {
            if !pointer_down {
                pane.banding = false;
            }
            banding = banding || pane.banding;
        }
        if self.preview_pane && !self.pane().this_pc {
            if !banding {
                let pane = self.tabs[self.active_tab].pane();
                self.preview.prepare(
                    ui.ctx(),
                    &pane.entries,
                    pane.previewed(),
                    &mut self.thumbs,
                    self.preview_width,
                );
            }
            let open = if !self.preview_auto_hide {
                true
            } else if banding {
                self.preview.was_open()
            } else {
                self.preview.auto_open(&self.thumbs)
            };
            if open {
                let panel = egui::Panel::right("preview")
                    .resizable(true)
                    .default_size(self.preview_width)
                    .frame(bar_frame)
                    .show(ui, |ui| self.preview.show(ui, &self.thumbs, &self.theme));
                self.preview_width = panel.response.rect.width();
            }
        }

        egui::CentralPanel::default().show(ui, |ui| self.panes(ui));
        if frame_nr == 0 && !self.startup_logged[1] {
            self.startup_logged[1] = true;
            startup::mark("first frame drawn by us (egui paints it next)");
        }
    }
}

impl App {
    fn apply_sidebar_action(&mut self, action: SidebarAction) {
        match action {
            SidebarAction::GoTo(path) => self.pane_mut().navigate(path),
            SidebarAction::ThisPc => self.pane_mut().show_this_pc(),
            SidebarAction::Mount(volume) => self.mount_volume(volume),
            SidebarAction::Unmount(volume) => self.eject_volume(volume, false),
            SidebarAction::Eject(volume) => self.eject_volume(volume, true),
            SidebarAction::OpenInNewTab(path) => self.new_tab(Some(path), true),
            SidebarAction::CopyPath(path) => {
                self.ctx.copy_text(path.to_string_lossy().into_owned());
            }
            SidebarAction::RemoveBookmark(path) => {
                let _ = itamiiru_core::places::remove_bookmark(&path);
                self.scan_places();
            }
            SidebarAction::Refresh => self.scan_places(),
            SidebarAction::Settings => self.settings_open = true,
        }
    }

    /// The options the settings page shows, as they are now.
    fn options(&self) -> Options {
        return Options {
            show_hidden: self.show_hidden,
            auto_refresh_folder: self.auto_refresh_folder,
            auto_refresh_drives: self.auto_refresh_drives,
            thumbnails: self.thumbnails_enabled,
            preview_pane: self.preview_pane,
            preview_auto_hide: self.preview_auto_hide,
            status_fields: self.status_fields.clone(),
            columns: self.details_columns.clone(),
            use_trash: self.use_trash,
            drag_and_drop: self.drag_and_drop,
            drag_select: self.drag_select,
            drag_within: self.drag_within,
        };
    }

    /// Applies what the settings page changed. The next frame saves it.
    fn apply_options(&mut self, options: Options) {
        if options.show_hidden != self.show_hidden {
            self.toggle_hidden();
        }
        if options.thumbnails != self.thumbnails_enabled {
            // Dropping the old pool stops its workers; textures go with it.
            self.thumbnails_enabled = options.thumbnails;
            self.thumbs = Thumbs::new(&self.ctx, options.thumbnails);
        }
        self.auto_refresh_folder = options.auto_refresh_folder;
        self.auto_refresh_drives = options.auto_refresh_drives;
        self.preview_pane = options.preview_pane;
        self.preview_auto_hide = options.preview_auto_hide;
        self.status_fields = options.status_fields;
        self.details_columns = options.columns;
        self.use_trash = options.use_trash;
        self.drag_and_drop = options.drag_and_drop;
        self.drag_select = options.drag_select;
        self.drag_within = options.drag_within;
    }

    fn settings_page(&mut self, ctx: &egui::Context) {
        if !self.settings_open {
            return;
        }
        let before = self.options();
        let mut options = before.clone();
        self.settings_open =
            settings_page::show(ctx, &self.theme, &mut options, self.config_path.as_deref());
        if options != before {
            self.apply_options(options);
        }
    }

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

/// The status bar's words about the selection. One item: the details in
/// `fields` (the `status_fields` setting), like a row of the Details view.
/// Several: how many, and the total size of the files among them (folder
/// sizes aren't known without walking them).
pub fn selection_summary(selected: &[&Entry], fields: &[StatusField]) -> Option<String> {
    if selected.len() == 1 {
        let entry = selected[0];
        let mut parts = Vec::new();
        for field in fields {
            match field {
                StatusField::Name => parts.push(entry.name.to_string_lossy().into_owned()),
                StatusField::Type => parts.push(type_label(entry)),
                StatusField::Size => {
                    if !entry.is_dir() {
                        parts.push(format_size(entry.size));
                    }
                }
                StatusField::Modified => {
                    if let Some(mtime) = entry.mtime {
                        parts.push(format!("modified {}", format_local_time(mtime)));
                    }
                }
                StatusField::Created => {
                    if let Some(created) = entry.created {
                        parts.push(format!("created {}", format_local_time(created)));
                    }
                }
                StatusField::Path => parts.push(entry.path.to_string_lossy().into_owned()),
            }
        }
        if parts.is_empty() {
            return Some("1 selected".to_string());
        }
        return Some(parts.join(", "));
    }
    if selected.is_empty() {
        return None;
    }
    let mut files = 0;
    let mut total: u64 = 0;
    for entry in selected {
        if !entry.is_dir() {
            files += 1;
            total = total.saturating_add(entry.size);
        }
    }
    let count = selected.len();
    if files == 0 {
        return Some(format!("{count} selected"));
    }
    if files == count {
        return Some(format!("{count} selected, {}", format_size(total)));
    }
    let noun = if files == 1 { "file" } else { "files" };
    return Some(format!(
        "{count} selected, {} in {files} {noun}",
        format_size(total)
    ));
}

/// The custom actions from the `actions` file next to the config file.
fn load_actions(config_path: Option<&Path>) -> Vec<Action> {
    let Some(dir) = config_path.and_then(Path::parent) else {
        return Vec::new();
    };
    return actions::load(&dir.join("actions"));
}

/// The path of the one selected folder, if exactly one folder is selected.
fn selected_one_folder(pane: &Pane) -> Option<PathBuf> {
    let selected = pane.selected_entries();
    if selected.len() == 1 && selected[0].is_dir() {
        return Some(selected[0].path.clone());
    }
    return None;
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

/// Only Linux has a trash implementation so far.
fn trash_supported() -> bool {
    return cfg!(target_os = "linux");
}

/// One path per line, for the system clipboard.
/// Whether `sources` can be dropped into `dest_dir`: not into one of
/// them or a folder inside one, and not back into the folder they're all
/// in already.
fn can_drop(sources: &[PathBuf], dest_dir: &Path) -> bool {
    let mut all_there = true;
    for source in sources {
        if dest_dir.starts_with(source) {
            return false;
        }
        if source.parent() != Some(dest_dir) {
            all_there = false;
        }
    }
    return !all_there;
}

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

/// An inline message in place of the list, never a popup.
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
    use std::sync::{Arc, Mutex};

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

    fn entry(name: &str, kind: itamiiru_core::entry::EntryKind, size: u64) -> Entry {
        return Entry {
            name: name.into(),
            path: PathBuf::from("/x").join(name),
            kind,
            size,
            mtime: None,
            created: None,
            hidden: false,
            executable: false,
        };
    }

    #[test]
    fn selection_summary_text() {
        use itamiiru_core::entry::EntryKind;
        let photo = entry("photo.jpg", EntryKind::File, 3 * 1024 * 1024);
        let notes = entry("notes.txt", EntryKind::File, 2048);
        let src = entry("src", EntryKind::Dir, 0);
        let default = Settings::default().status_fields;
        let summary = |selected: &[&Entry]| selection_summary(selected, &default);
        assert_eq!(summary(&[]), None);
        // The default leaves the name out.
        assert_eq!(summary(&[&photo]).as_deref(), Some("JPG File, 3.0 MB"));
        assert_eq!(summary(&[&src]).as_deref(), Some("Folder"));
        assert_eq!(
            summary(&[&photo, &notes]).as_deref(),
            Some("2 selected, 3.0 MB")
        );
        assert_eq!(
            summary(&[&photo, &src]).as_deref(),
            Some("2 selected, 3.0 MB in 1 file")
        );
        assert_eq!(summary(&[&src, &src]).as_deref(), Some("2 selected"));

        // Fields in the order given.
        let fields = [StatusField::Size, StatusField::Name, StatusField::Path];
        assert_eq!(
            selection_summary(&[&photo], &fields).as_deref(),
            Some("3.0 MB, photo.jpg, /x/photo.jpg")
        );
        assert_eq!(
            selection_summary(&[&photo], &[]).as_deref(),
            Some("1 selected")
        );
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
    fn settings_page_opens_applies_and_closes() {
        let folder = Folder::new("settings");
        std::fs::write(folder.0.join(".hidden"), b"").expect("hidden file");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        assert_eq!(app.pane().visible.len(), 3);

        frame(&mut app, &ctx, key(Key::Comma, Modifiers::COMMAND));
        assert!(app.settings_open);
        frame(&mut app, &ctx, Vec::new());
        // Keys don't reach the list while it's open.
        frame(&mut app, &ctx, key(Key::A, Modifiers::COMMAND));
        assert_eq!(app.pane().selection.count(), 0);

        let mut options = app.options();
        options.show_hidden = true;
        options.thumbnails = true;
        options.status_fields = vec![StatusField::Name];
        app.apply_options(options.clone());
        assert_eq!(app.options(), options);
        // Hidden files show up at once.
        assert_eq!(app.pane().visible.len(), 4);
        // Restore defaults gives what a fresh install has.
        app.apply_options(Options::defaults());
        assert_eq!(app.options(), Options::defaults());
        assert_eq!(app.pane().visible.len(), 3);

        frame(&mut app, &ctx, key(Key::Escape, Modifiers::NONE));
        frame(&mut app, &ctx, Vec::new());
        assert!(!app.settings_open);
    }

    #[test]
    fn backspace_selects_the_folder_we_came_from() {
        let folder = Folder::new("up");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        app.pane_mut().navigate(folder.0.join("beta"));
        settle(&mut app, &ctx);

        frame(&mut app, &ctx, key(Key::Backspace, Modifiers::NONE));
        settle(&mut app, &ctx);
        let pane = app.pane();
        assert_eq!(pane.current, folder.0);
        let beta = folder.0.join("beta");
        assert!(pane.selection.is_selected(&beta));
        assert_eq!(pane.selection.count(), 1);
        assert_eq!(pane.selection.cursor(), Some(beta.as_path()));

        // The arrow keys carry on from it: alpha is just above beta.
        frame(&mut app, &ctx, key(Key::ArrowUp, Modifiers::NONE));
        assert!(app.pane().selection.is_selected(&folder.0.join("alpha")));
    }

    #[test]
    fn details_columns_can_be_switched_off() {
        let folder = Folder::new("columns");
        std::fs::write(folder.0.join("picture.jpeg"), b"x").expect("jpeg");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        // "JPEG File" is longer than "Folder" and "TXT File".
        assert_eq!(app.pane().widest_type, "JPEG File");

        // Every subset of the columns, in order and reversed.
        for bits in 0..16 {
            let mut columns = Vec::new();
            for (i, column) in DetailColumn::ALL.iter().enumerate() {
                if bits & (1 << i) != 0 {
                    columns.push(*column);
                }
            }
            app.details_columns = columns.clone();
            frame(&mut app, &ctx, Vec::new());
            columns.reverse();
            app.details_columns = columns;
            frame(&mut app, &ctx, Vec::new());
        }
        // And with search results' Folder column.
        app.pane_mut().open_search();
        // One frame for the box to take focus, then type into it.
        frame(&mut app, &ctx, Vec::new());
        frame(&mut app, &ctx, vec![Event::Text("pic".to_string())]);
        std::thread::sleep(Duration::from_millis(400));
        for _ in 0..50 {
            frame(&mut app, &ctx, Vec::new());
            if app.pane().search.as_ref().is_some_and(|s| !s.running) {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(app.pane().visible.len(), 1);
        assert_eq!(app.pane().widest_type, "JPEG File");
        frame(&mut app, &ctx, Vec::new());
    }

    /// Runs frames until no mount or eject is in progress.
    fn settle_volumes(app: &mut App, ctx: &egui::Context) {
        for _ in 0..500 {
            frame(app, ctx, Vec::new());
            if app.volumes_busy.is_empty() {
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("mount or eject didn't finish");
    }

    /// A volume UDisks doesn't have, so nothing real is touched.
    fn fake_volume(mount_point: Option<PathBuf>) -> Volume {
        return Volume {
            id: "/org/freedesktop/UDisks2/block_devices/itamiiru_test_nonexistent".to_string(),
            drive_id: String::new(),
            name: "TESTSTICK".to_string(),
            device: PathBuf::from("/dev/itamiiru-test"),
            size: 1024,
            fs_type: "vfat".to_string(),
            mount_point,
            can_power_off: false,
        };
    }

    #[test]
    fn ejecting_moves_panes_off_the_drive_and_reports_back() {
        let folder = Folder::new("eject");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        // Pretend the test folder is a mounted stick, open in a second tab.
        app.pane_mut().navigate(folder.0.join("alpha"));
        settle(&mut app, &ctx);
        app.new_tab(Some(folder.0.join("beta")), false);
        let volume = fake_volume(Some(folder.0.clone()));
        app.places.volumes = vec![volume.clone()];

        app.eject_volume(volume.clone(), true);
        assert!(app.volumes_busy.contains(&volume.id));
        // Both panes inside it left for This PC before anything ran.
        assert!(app.tabs[0].pane().this_pc);
        assert!(app.tabs[1].pane().this_pc);
        // A second click while it's busy does nothing.
        app.eject_volume(volume.clone(), true);

        settle_volumes(&mut app, &ctx);
        // UDisks has nothing to unmount for this id, so it "succeeds"; on
        // a machine without UDisks it's an error. Either way it's reported.
        let noticed = app
            .notice
            .as_ref()
            .is_some_and(|(t, _)| t.contains("TESTSTICK"));
        let failed = app.op_errors.iter().any(|e| e.contains("TESTSTICK"));
        assert!(
            noticed || failed,
            "notice {:?} errors {:?}",
            app.notice,
            app.op_errors
        );
    }

    /// Runs one frame and returns where each piece of text was drawn.
    fn frame_texts(app: &mut App, ctx: &egui::Context, events: Vec<Event>) -> Vec<(String, Rect)> {
        let input = egui::RawInput {
            screen_rect: Some(SCREEN),
            events,
            ..Default::default()
        };
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(input, |ui| eframe::App::ui(app, ui, &mut frame));
        output.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in &output.shapes {
            collect_texts(&clipped.shape, &mut texts);
        }
        return texts;
    }

    fn collect_texts(shape: &egui::Shape, out: &mut Vec<(String, Rect)>) {
        match shape {
            egui::Shape::Text(text) => {
                out.push((text.galley.text().to_string(), text.visual_bounding_rect()));
            }
            egui::Shape::Vec(shapes) => {
                for shape in shapes {
                    collect_texts(shape, out);
                }
            }
            _ => {}
        }
    }

    #[test]
    fn sidebar_drive_menu_ejects() {
        let folder = Folder::new("sidebarmenu");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        // No rescans replacing the pretend drive below.
        app.auto_refresh_drives = false;
        frame(&mut app, &ctx, Vec::new());
        let volume = fake_volume(Some(folder.0.clone()));
        app.places = Places {
            drives: vec![itamiiru_core::places::Drive {
                name: "TESTSTICK".to_string(),
                mount_point: folder.0.clone(),
                fs_type: "vfat".to_string(),
                kind: itamiiru_core::places::DriveKind::Removable,
                hidden: false,
            }],
            volumes: vec![volume.clone()],
            ..Places::default()
        };
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let row = texts
            .iter()
            .find(|(text, _)| text == "TESTSTICK")
            .expect("the drive is in the sidebar")
            .1;

        // Right-click it: the menu has Eject.
        let press = |pos, button, pressed| Event::PointerButton {
            pos,
            button,
            pressed,
            modifiers: Modifiers::NONE,
        };
        let at = row.center();
        frame(&mut app, &ctx, vec![Event::PointerMoved(at)]);
        frame(
            &mut app,
            &ctx,
            vec![press(at, PointerButton::Secondary, true)],
        );
        frame(
            &mut app,
            &ctx,
            vec![press(at, PointerButton::Secondary, false)],
        );
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        for item in ["Open", "Open in new tab", "Copy path", "Unmount"] {
            assert!(texts.iter().any(|(t, _)| t == item), "{item} in the menu");
        }
        let eject = texts
            .iter()
            .find(|(text, _)| text == "Eject")
            .expect("Eject in the menu")
            .1;

        click(&mut app, &ctx, eject.center());
        assert!(
            app.volumes_busy.contains(&volume.id)
                || app.notice.is_some()
                || !app.op_errors.is_empty(),
            "the eject started"
        );
        settle_volumes(&mut app, &ctx);
    }

    /// Right-clicks at `at`, then clicks the menu item called `item`.
    fn menu_click(app: &mut App, ctx: &egui::Context, at: Pos2, item: &str) {
        let press = |pressed| Event::PointerButton {
            pos: at,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(app, ctx, vec![Event::PointerMoved(at)]);
        frame(app, ctx, vec![press(true)]);
        frame(app, ctx, vec![press(false)]);
        let texts = frame_texts(app, ctx, Vec::new());
        let found = texts.iter().find(|(text, _)| text == item);
        let names: Vec<&String> = texts.iter().map(|(t, _)| t).collect();
        let rect = found
            .unwrap_or_else(|| panic!("{item:?} in the menu: {names:?}"))
            .1;
        click(app, ctx, rect.center());
    }

    fn wait_for(path: &Path) -> bool {
        for _ in 0..200 {
            if path.exists() {
                return true;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        return false;
    }

    #[test]
    fn custom_actions_in_the_right_click_menus() {
        let folder = Folder::new("actions");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let (actions, problems) = actions::parse(
            "[Mark here]\ncommand = touch %d/marker\nfor = folders, background\n\
             [Only for jpg]\ncommand = true\nfor = *.jpg\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        app.actions = actions;

        // On a folder: %d is that folder.
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let alpha = texts
            .iter()
            .find(|(text, _)| text == "alpha")
            .expect("alpha is listed")
            .1;
        menu_click(&mut app, &ctx, alpha.center(), "Mark here");
        assert!(wait_for(&folder.0.join("alpha/marker")), "ran in alpha");

        // The jpg action isn't offered for a folder.
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        assert!(!texts.iter().any(|(t, _)| t == "Only for jpg"));

        // On empty space: %d is the folder on screen.
        let empty = Pos2::new(900.0, 700.0);
        menu_click(&mut app, &ctx, empty, "Mark here");
        assert!(wait_for(&folder.0.join("marker")), "ran in the folder");
    }

    #[test]
    fn a_missing_program_is_reported() {
        let folder = Folder::new("badaction");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let (actions, _) =
            actions::parse("[Nope]\ncommand = itamiiru-no-such-program %d\nfor = background\n");
        app.actions = actions;
        app.run_command(Command::ActionOnFolder(0));
        assert!(
            app.op_errors
                .iter()
                .any(|e| e.starts_with("Nope: can't run itamiiru-no-such-program")),
            "{:?}",
            app.op_errors
        );
    }

    #[test]
    fn settings_moved_to_the_sidebar() {
        let folder = Folder::new("sidebarbuttons");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        assert!(
            !texts.iter().any(|(t, _)| t == "Settings"),
            "no Settings text button"
        );

        // The icons sit in a row at the bottom of the side pane, just above
        // the status bar: click along it until the settings page opens.
        let status = texts
            .iter()
            .find(|(t, _)| t == "3 items")
            .expect("status bar")
            .1;
        let mut opened = false;
        'search: for dy in (12..80).step_by(4) {
            for x in (10..120).step_by(6) {
                let y = status.top() - dy as f32;
                click(&mut app, &ctx, Pos2::new(x as f32, y));
                if app.settings_open {
                    opened = true;
                    break 'search;
                }
            }
        }
        assert!(opened, "the gear opens the settings page");
    }

    #[test]
    fn the_refresh_button_lists_the_folder_again() {
        let folder = Folder::new("refreshbutton");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        app.auto_refresh_folder = false;
        settle(&mut app, &ctx);
        assert_eq!(app.pane().visible.len(), 3);
        std::fs::write(folder.0.join("new.txt"), b"x").expect("new file");

        // The buttons are at the right end of the navigation bar, on the
        // same row as the folder's name in the address bar.
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let name = folder.0.file_name().and_then(|n| n.to_str()).expect("name");
        let row = texts
            .iter()
            .find(|(t, _)| t == name)
            .expect("the address bar shows the folder")
            .1
            .center()
            .y;
        let mut refreshed = false;
        for x in (900..1195).rev().step_by(3) {
            click(&mut app, &ctx, Pos2::new(x as f32, row));
            // Undo the neighbors: search and filter open a row.
            if app.pane().search_open {
                app.pane_mut().close_search();
            }
            if app.pane().filter_open {
                let (pane, filter) = app.pane_and_filter();
                pane.close_filter(&filter);
            }
            if app.pane().loading {
                refreshed = true;
                break;
            }
        }
        assert!(refreshed, "found the refresh button");
        settle(&mut app, &ctx);
        assert_eq!(app.pane().visible.len(), 4, "the new file shows");
    }

    #[test]
    fn dialog_buttons_sit_bottom_right() {
        let folder = Folder::new("dialogbuttons");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        app.confirm_delete = Some(vec![folder.0.join("file.txt")]);
        frame(&mut app, &ctx, Vec::new());
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let message = texts
            .iter()
            .find(|(t, _)| t.starts_with("Permanently delete"))
            .expect("message")
            .1;
        // The dialog's buttons, not the toolbar's "Delete" at the top.
        let find = |wanted: &str| {
            let found = texts
                .iter()
                .find(|(t, rect)| t == wanted && rect.top() > message.bottom());
            return found
                .unwrap_or_else(|| panic!("{wanted:?} in the dialog"))
                .1;
        };
        let cancel = find("Cancel");
        let delete = find("Delete");
        // Cancel, then Delete, both under the right half of the message.
        assert!(
            cancel.right() < delete.left(),
            "cancel {cancel:?} delete {delete:?}"
        );
        assert!(
            cancel.left() > message.center().x,
            "cancel {cancel:?} message {message:?}"
        );
        assert!(
            delete.right() > message.right() - 40.0,
            "delete {delete:?} message {message:?}"
        );
    }

    /// What the stand-in for the real drag received, and the handle it
    /// gave back (so the test can play the drag thread).
    static DRAGGED: std::sync::Mutex<Vec<Vec<PathBuf>>> = std::sync::Mutex::new(Vec::new());
    static LAST_DRAG: std::sync::Mutex<Option<DragOut>> = std::sync::Mutex::new(None);

    fn record_drag(paths: Vec<PathBuf>) -> Result<DragOut, String> {
        let drag = DragOut::new();
        if let Ok(mut dragged) = DRAGGED.lock() {
            dragged.push(paths);
        }
        if let Ok(mut last) = LAST_DRAG.lock() {
            *last = Some(drag.clone());
        }
        return Ok(drag);
    }

    fn button(pos: Pos2, pressed: bool) -> Event {
        return Event::PointerButton {
            pos,
            button: PointerButton::Primary,
            pressed,
            modifiers: Modifiers::NONE,
        };
    }

    /// Presses on `start` and drags `steps` times by `step`; returns the
    /// cursor the last frame asked for.
    fn drag_from(
        app: &mut App,
        ctx: &egui::Context,
        start: Pos2,
        step: egui::Vec2,
        steps: usize,
    ) -> egui::CursorIcon {
        frame(app, ctx, vec![Event::PointerMoved(start)]);
        frame(app, ctx, vec![button(start, true)]);
        let mut cursor = egui::CursorIcon::Default;
        for i in 1..=steps {
            let pos = start + step * i as f32;
            cursor = frame(app, ctx, vec![Event::PointerMoved(pos)]);
        }
        return cursor;
    }

    #[test]
    fn dragging_a_file_out_of_the_window() {
        let folder = Folder::new("dragout");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        app.start_drag_out = record_drag;
        settle(&mut app, &ctx);
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let start = texts
            .iter()
            .find(|(text, _)| text == "file.txt")
            .expect("file.txt is listed")
            .1
            .center();

        // Let go inside the window: nothing is handed over. While it's
        // inside, the cursor is a grabbing hand.
        let cursor = drag_from(&mut app, &ctx, start, egui::vec2(0.0, 10.0), 5);
        assert!(app.drag_out.is_some(), "the drag started");
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(
            frame(&mut app, &ctx, Vec::new()),
            egui::CursorIcon::Grabbing,
            "{cursor:?}"
        );
        frame(
            &mut app,
            &ctx,
            vec![button(start + egui::vec2(0.0, 50.0), false)],
        );
        frame(&mut app, &ctx, Vec::new());
        assert!(app.drag_out.is_none());

        // Escape before leaving the window: the drag is dropped.
        drag_from(&mut app, &ctx, start, egui::vec2(0.0, 10.0), 5);
        frame(&mut app, &ctx, key(Key::Escape, Modifiers::NONE));
        assert!(app.drag_out.is_none(), "Escape dropped the drag");
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(Pos2::new(-40.0, start.y))],
        );
        frame(
            &mut app,
            &ctx,
            vec![button(Pos2::new(-40.0, start.y), false)],
        );
        assert!(DRAGGED.lock().expect("lock").is_empty());

        // Past the window's edge with the button down: handed over, with
        // the dragged file (selected on the way).
        drag_from(&mut app, &ctx, start, egui::vec2(-20.0, 0.0), 5);
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(Pos2::new(-40.0, start.y))],
        );
        frame(&mut app, &ctx, Vec::new());
        let dragged = DRAGGED.lock().expect("lock").clone();
        assert_eq!(dragged, vec![vec![folder.0.join("file.txt")]]);
        assert!(app.drag_out.is_none(), "handed over once");
        let drag = LAST_DRAG.lock().expect("lock").clone().expect("a handle");

        // The cursor follows the drag thread's state.
        drag.set_state(DragState::Accepted);
        assert_eq!(frame(&mut app, &ctx, Vec::new()), egui::CursorIcon::Copy);
        drag.set_state(DragState::Refused);
        assert_eq!(frame(&mut app, &ctx, Vec::new()), egui::CursorIcon::NoDrop);

        // Escape cancels it, and does nothing else (the search row, say).
        app.pane_mut().search_open = true;
        frame(&mut app, &ctx, key(Key::Escape, Modifiers::NONE));
        assert!(drag.is_cancelled());
        assert!(app.pane().search_open, "Escape only cancelled the drag");

        // Once the thread says it's over, the app lets go of it.
        drag.set_state(DragState::Ended);
        frame(&mut app, &ctx, Vec::new());
        assert!(app.dragging_out.is_none());
    }

    /// Drags from `start` to `end` and lets go there.
    fn drag_and_drop(app: &mut App, ctx: &egui::Context, start: Pos2, end: Pos2) {
        drag_from(app, ctx, start, (end - start) / 5.0, 5);
        frame(app, ctx, Vec::new());
        frame(app, ctx, vec![button(end, false)]);
        settle_op(app, ctx);
    }

    fn text_center(app: &mut App, ctx: &egui::Context, text: &str) -> Pos2 {
        let texts = frame_texts(app, ctx, Vec::new());
        let found = texts.iter().find(|(t, _)| t == text);
        return found.expect("the text is on screen").1.center();
    }

    #[test]
    fn dropping_on_a_folder_and_on_the_other_pane() {
        let folder = Folder::new("dropping");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);

        // A file dropped on a folder on the same drive moves into it.
        let file = text_center(&mut app, &ctx, "file.txt");
        let alpha = text_center(&mut app, &ctx, "alpha");
        drag_and_drop(&mut app, &ctx, file, alpha);
        assert!(folder.0.join("alpha/file.txt").exists());
        assert!(!folder.0.join("file.txt").exists());

        // Let go on empty space in its own folder: nothing happens.
        let beta = text_center(&mut app, &ctx, "beta");
        drag_and_drop(&mut app, &ctx, beta, Pos2::new(400.0, 600.0));
        assert!(folder.0.join("beta").exists());

        // Split view, the right pane in alpha: beta dropped on the right
        // pane's empty space moves into alpha.
        app.run_command(Command::ToggleSplit);
        settle(&mut app, &ctx);
        app.tabs[app.active_tab].panes[1].navigate(folder.0.join("alpha"));
        settle(&mut app, &ctx);
        let beta = text_center(&mut app, &ctx, "beta");
        drag_and_drop(&mut app, &ctx, beta, Pos2::new(1100.0, 600.0));
        assert!(folder.0.join("alpha/beta").exists());
        assert!(!folder.0.join("beta").exists());
        assert!(app.drag_out.is_none() && app.drop_target.is_none());

        // One pane, in alpha: file.txt dropped on the crumb of the folder
        // above goes back there.
        app.run_command(Command::ToggleSplit);
        app.pane_mut().navigate(folder.0.join("alpha"));
        settle(&mut app, &ctx);
        let name = folder.0.file_name().expect("a name").to_string_lossy();
        let crumb = text_center(&mut app, &ctx, &name);
        let file = text_center(&mut app, &ctx, "file.txt");
        drag_and_drop(&mut app, &ctx, file, crumb);
        assert!(folder.0.join("file.txt").exists());
        assert!(!folder.0.join("alpha/file.txt").exists());
    }

    #[derive(Debug)]
    struct TestDrop(PathBuf);

    impl egui::DroppedFile for TestDrop {
        fn path(&self) -> &Path {
            return &self.0;
        }

        fn bytes(&self) -> Result<Vec<u8>, String> {
            return std::fs::read(&self.0).map_err(|e| e.to_string());
        }
    }

    /// Where the window's inside is on the (pretend) screen.
    const WINDOW_AT: egui::Vec2 = egui::vec2(100.0, 50.0);

    static POINTER: Mutex<Option<(f32, f32)>> = Mutex::new(None);

    fn test_pointer() -> Option<(f32, f32)> {
        return *POINTER.lock().expect("lock");
    }

    /// A frame where another app drags `paths` over the window, or drops
    /// them, with the pointer at `at` in the window (None: unknown).
    fn drag_in_frame(
        app: &mut App,
        ctx: &egui::Context,
        paths: &[PathBuf],
        drop: bool,
        at: Option<Pos2>,
    ) {
        *POINTER.lock().expect("lock") = at.map(|p| (p.x + WINDOW_AT.x, p.y + WINDOW_AT.y));
        app.pointer_position = test_pointer;
        let mut input = egui::RawInput {
            screen_rect: Some(SCREEN),
            ..Default::default()
        };
        let info = egui::ViewportInfo {
            inner_rect: Some(SCREEN.translate(WINDOW_AT)),
            ..Default::default()
        };
        input.viewports.insert(egui::ViewportId::ROOT, info);
        for path in paths {
            if drop {
                input.dropped_files.push(Arc::new(TestDrop(path.clone())));
            } else {
                input.hovered_files.push(egui::HoveredFile {
                    path: Some(path.clone()),
                    mime: String::new(),
                });
            }
        }
        app.notice_drag_in(ctx, &mut input);
        let mut frame = eframe::Frame::_new_kittest();
        let mut output = ctx.run_ui(input, |ui| eframe::App::ui(app, ui, &mut frame));
        output.textures_delta.clear();
    }

    #[test]
    fn files_dropped_in_from_another_app() {
        let folder = Folder::new("dropin");
        let outside = Folder::new("dropin-outside");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let file = outside.0.join("file.txt");

        // Dropped on a folder: copied into it, the original stays.
        let alpha = text_center(&mut app, &ctx, "alpha");
        drag_in_frame(
            &mut app,
            &ctx,
            std::slice::from_ref(&file),
            false,
            Some(alpha),
        );
        drag_in_frame(
            &mut app,
            &ctx,
            std::slice::from_ref(&file),
            false,
            Some(alpha),
        );
        assert_eq!(app.drop_target, Some(folder.0.join("alpha")));
        drag_in_frame(
            &mut app,
            &ctx,
            std::slice::from_ref(&file),
            true,
            Some(alpha),
        );
        settle_op(&mut app, &ctx);
        assert!(folder.0.join("alpha/file.txt").exists());
        assert!(file.exists());

        // Where the pointer is isn't known: into the open folder.
        let new_file = outside.0.join("new.txt");
        std::fs::write(&new_file, b"new").expect("write new.txt");
        drag_in_frame(&mut app, &ctx, std::slice::from_ref(&new_file), false, None);
        drag_in_frame(&mut app, &ctx, std::slice::from_ref(&new_file), true, None);
        settle_op(&mut app, &ctx);
        assert!(folder.0.join("new.txt").exists());

        // Switched off: nothing happens.
        let mut options = app.options();
        options.drag_and_drop = false;
        app.apply_options(options);
        let alpha_dir = outside.0.join("alpha");
        drag_in_frame(
            &mut app,
            &ctx,
            std::slice::from_ref(&alpha_dir),
            true,
            Some(alpha),
        );
        settle_op(&mut app, &ctx);
        assert!(!folder.0.join("alpha/alpha").exists());
    }

    #[test]
    fn a_drag_held_over_a_tab_opens_it() {
        let folder = Folder::new("dragtab");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        app.new_tab(Some(folder.0.join("alpha")), false);
        settle(&mut app, &ctx);
        assert_eq!(app.active_tab, 0);

        // The second tab is named alpha; the topmost "alpha" is the tab.
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let mut tab = None;
        for (text, rect) in &texts {
            if text == "alpha" && tab.is_none_or(|t: Rect| rect.top() < t.top()) {
                tab = Some(*rect);
            }
        }
        let tab = tab.expect("the alpha tab").center();
        let file = text_center(&mut app, &ctx, "file.txt");
        drag_from(&mut app, &ctx, file, (tab - file) / 5.0, 5);
        assert_eq!(app.active_tab, 0, "not straight away");
        for _ in 0..40 {
            frame(&mut app, &ctx, Vec::new());
        }
        assert_eq!(app.active_tab, 1, "held there, the tab opened");

        // Dropped in the open tab's folder.
        let empty = Pos2::new(600.0, 600.0);
        for i in 1..=5 {
            let pos = tab + (empty - tab) * (i as f32 / 5.0);
            frame(&mut app, &ctx, vec![Event::PointerMoved(pos)]);
        }
        frame(&mut app, &ctx, Vec::new());
        frame(&mut app, &ctx, vec![button(empty, false)]);
        settle_op(&mut app, &ctx);
        assert!(folder.0.join("alpha/file.txt").exists());
        assert!(!folder.0.join("file.txt").exists());
    }

    #[test]
    fn dropping_on_a_sidebar_place() {
        let folder = Folder::new("dropsidebar");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        // No rescans replacing the pretend place below.
        app.auto_refresh_drives = false;
        frame(&mut app, &ctx, Vec::new());
        app.places = Places {
            quick: vec![itamiiru_core::places::Place {
                name: "DropHere".to_string(),
                path: folder.0.join("beta"),
                kind: itamiiru_core::places::PlaceKind::Folder,
            }],
            ..Places::default()
        };
        let place = text_center(&mut app, &ctx, "DropHere");
        let file = text_center(&mut app, &ctx, "file.txt");
        drag_and_drop(&mut app, &ctx, file, place);
        assert!(folder.0.join("beta/file.txt").exists());
        assert!(!folder.0.join("file.txt").exists());
    }

    #[test]
    fn dropping_can_be_switched_off() {
        let folder = Folder::new("nodrop");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let mut options = app.options();
        options.drag_within = false;
        app.apply_options(options);

        let file = text_center(&mut app, &ctx, "file.txt");
        let alpha = text_center(&mut app, &ctx, "alpha");
        drag_and_drop(&mut app, &ctx, file, alpha);
        assert!(folder.0.join("file.txt").exists());
        assert!(!folder.0.join("alpha/file.txt").exists());
    }

    #[test]
    fn where_items_can_be_dropped() {
        let sources = [PathBuf::from("/a/x"), PathBuf::from("/a/dir")];
        assert!(can_drop(&sources, Path::new("/b")));
        assert!(can_drop(&sources, Path::new("/")));
        // Not where they all are, nor into themselves.
        assert!(!can_drop(&sources, Path::new("/a")));
        assert!(!can_drop(&sources, Path::new("/a/dir")));
        assert!(!can_drop(&sources, Path::new("/a/dir/sub")));
        // Search results from several folders can be gathered into one.
        let mixed = [PathBuf::from("/a/x"), PathBuf::from("/b/y")];
        assert!(can_drop(&mixed, Path::new("/a")));
    }

    #[test]
    fn trash_and_drag_can_be_switched_off() {
        let folder = Folder::new("switches");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        app.start_drag_out = record_drag;
        settle(&mut app, &ctx);
        let mut options = app.options();
        options.use_trash = false;
        options.drag_and_drop = false;
        app.apply_options(options);
        assert!(!app.use_trash && !app.drag_and_drop);

        // Delete asks to delete permanently instead of using the trash.
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let start = texts
            .iter()
            .find(|(text, _)| text == "file.txt")
            .expect("file.txt is listed")
            .1
            .center();
        click(&mut app, &ctx, start);
        frame(&mut app, &ctx, key(Key::Delete, Modifiers::NONE));
        assert!(app.confirm_delete.is_some(), "asks first");
        assert!(app.running.is_none(), "nothing went to the trash");
        app.confirm_delete = None;

        // The right-click menu has no "Move to trash" (looked at while
        // it's open), only "Delete permanently".
        let right = |pressed| Event::PointerButton {
            pos: start,
            button: PointerButton::Secondary,
            pressed,
            modifiers: Modifiers::NONE,
        };
        frame(&mut app, &ctx, vec![Event::PointerMoved(start)]);
        frame(&mut app, &ctx, vec![right(true)]);
        frame(&mut app, &ctx, vec![right(false)]);
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        assert!(
            texts.iter().any(|(t, _)| t == "Delete permanently"),
            "the menu is open"
        );
        assert!(!texts.iter().any(|(t, _)| t == "Move to trash"));
        frame(&mut app, &ctx, key(Key::Escape, Modifiers::NONE));
        frame(&mut app, &ctx, Vec::new());

        // Dragging doesn't hand the items to other apps (dropping them
        // inside the window still works).
        drag_from(&mut app, &ctx, start, egui::vec2(-20.0, 0.0), 5);
        frame(
            &mut app,
            &ctx,
            vec![Event::PointerMoved(Pos2::new(-40.0, start.y))],
        );
        frame(
            &mut app,
            &ctx,
            vec![button(Pos2::new(-40.0, start.y), false)],
        );
        assert!(app.dragging_out.is_none());
        assert!(folder.0.join("file.txt").exists());
    }

    #[test]
    fn sorting_from_the_right_click_menu() {
        let folder = Folder::new("sortmenu");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        assert!(
            texts.iter().any(|(t, _)| t.starts_with("Sort:")),
            "toolbar sort dropdown is present"
        );

        // The icon views have no headers: sort from the empty-space menu.
        app.view = ViewMode::LargeIcons;
        frame(&mut app, &ctx, Vec::new());
        assert_eq!(app.sort.column, SortColumn::Name);
        menu_click(&mut app, &ctx, Pos2::new(900.0, 700.0), "Sort by");
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let size = texts
            .iter()
            .find(|(t, _)| t == "Size")
            .expect("the Sort by submenu is open")
            .1;
        click(&mut app, &ctx, size.center());
        assert_eq!(app.sort.column, SortColumn::Size);

        // In Details the headers still sort.
        app.view = ViewMode::Details;
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let header = texts
            .iter()
            .find(|(t, _)| t == "Name")
            .expect("Name header")
            .1;
        click(&mut app, &ctx, header.center());
        assert_eq!(app.sort.column, SortColumn::Name);
    }

    #[test]
    fn bookmarking_via_command() {
        let folder = Folder::new("bookmark_test");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);

        let alpha_path = folder.0.join("alpha");
        select_named(&mut app, "alpha");
        assert_eq!(app.selected_folder(), Some(alpha_path));

        app.run_command(Command::ToggleBookmark);
        assert!(app.places_scan_running);
    }

    /// Startup work that doesn't need a window, as the app does it.
    /// `cargo test --release -p itamiiru startup_timing -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn startup_timing() {
        let ms = |since: std::time::Instant| since.elapsed().as_secs_f64() * 1000.0;
        let home = itamiiru_core::platform::home_dir().expect("home");
        let t = std::time::Instant::now();
        let path = itamiiru_core::config::default_path();
        let config = match &path {
            Some(path) => Config::load(path),
            None => Config::default(),
        };
        let theme = Theme::load();
        let _ = itamiiru_core::format::format_local_time(std::time::SystemTime::now());
        println!("{:7.1} ms  config, theme, time zone", ms(t));
        let ctx = egui::Context::default();
        let t = std::time::Instant::now();
        theme.install(&ctx);
        let mut app = App::new(ctx.clone(), theme, home, Vec::new(), config, None, None);
        println!("{:7.1} ms  App::new", ms(t));
        let t = std::time::Instant::now();
        frame(&mut app, &ctx, Vec::new());
        println!("{:7.1} ms  first frame (egui's fonts, no GPU)", ms(t));
        let t = std::time::Instant::now();
        frame(&mut app, &ctx, Vec::new());
        println!("{:7.1} ms  second frame", ms(t));
        let t = std::time::Instant::now();
        settle(&mut app, &ctx);
        println!(
            "{:7.1} ms  home folder listed ({} items)",
            ms(t),
            app.pane().entries.len()
        );
        let t = std::time::Instant::now();
        for _ in 0..10 {
            frame(&mut app, &ctx, Vec::new());
        }
        println!("{:7.1} ms  per frame after that", ms(t) / 10.0);
    }

    #[test]
    fn dot_dot_opens_the_parent_with_a_proper_breadcrumb() {
        let folder = Folder::new("dotdot");
        let ctx = egui::Context::default();
        // As `itami ..` from inside alpha would give it, before the fix.
        let mut app = new_app(&ctx, &folder.0.join("alpha").join(".."));
        settle(&mut app, &ctx);
        assert_eq!(app.pane().current, folder.0);
        assert_eq!(app.pane().visible.len(), 3);
        // The last crumb is the folder's name, not the whole path again.
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let name = folder.0.file_name().and_then(|n| n.to_str()).expect("name");
        assert!(
            texts.iter().any(|(t, _)| t == name),
            "the folder's name is a crumb"
        );
        let full = folder.0.to_string_lossy().into_owned();
        assert!(
            !texts.iter().any(|(t, _)| *t == full),
            "no crumb with the full path"
        );

        // Typed into the address bar: ".." and "./alpha" too.
        assert_eq!(
            app.pane().parse_address(".."),
            folder.0.parent().expect("parent")
        );
        assert_eq!(
            app.pane().parse_address("./alpha/."),
            folder.0.join("alpha")
        );
        app.pane_mut().navigate(folder.0.join("beta").join("."));
        assert_eq!(app.pane().current, folder.0.join("beta"));
    }

    #[test]
    fn the_eject_mark_is_not_a_button() {
        let folder = Folder::new("ejectmark");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);
        app.auto_refresh_drives = false;
        app.pane_mut().navigate(folder.0.join("alpha"));
        settle(&mut app, &ctx);
        app.places = Places {
            drives: vec![itamiiru_core::places::Drive {
                name: "TESTSTICK".to_string(),
                mount_point: folder.0.clone(),
                fs_type: "vfat".to_string(),
                kind: itamiiru_core::places::DriveKind::Removable,
                hidden: false,
            }],
            volumes: vec![fake_volume(Some(folder.0.clone()))],
            ..Places::default()
        };
        let texts = frame_texts(&mut app, &ctx, Vec::new());
        let row = texts
            .iter()
            .find(|(text, _)| text == "TESTSTICK")
            .expect("the drive is in the sidebar")
            .1;
        // The mark sits at the right end of the row, inside the sidebar.
        let mark = Pos2::new(app.sidebar_width - theme::GAP_LARGE, row.center().y + 4.0);
        click(&mut app, &ctx, mark);
        settle(&mut app, &ctx);
        assert!(app.volumes_busy.is_empty(), "nothing was ejected");
        assert!(app.notice.is_none());
        // It's just part of the row: the click opened the drive.
        assert_eq!(app.pane().current, folder.0);
    }

    #[test]
    fn mounting_an_unknown_volume_shows_an_error() {
        let folder = Folder::new("mount");
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, &folder.0);
        settle(&mut app, &ctx);

        app.mount_volume(fake_volume(None));
        settle_volumes(&mut app, &ctx);
        assert!(
            app.op_errors
                .iter()
                .any(|e| e.contains("TESTSTICK: can't mount")),
            "{:?}",
            app.op_errors
        );
        assert_eq!(app.pane().current, folder.0);

        // An already mounted volume just opens.
        app.mount_volume(fake_volume(Some(folder.0.join("beta"))));
        assert!(app.volumes_busy.is_empty());
        assert_eq!(app.pane().current, folder.0.join("beta"));
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
