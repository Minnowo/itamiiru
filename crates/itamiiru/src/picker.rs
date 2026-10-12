//! Picker mode (`itami --pick`): the request read
//! from stdin, the footer with the file name field, filters and choices,
//! and the answer written to stdout.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};

use eframe::egui::{self, Key};
use itamiiru_core::entry::Entry;
use itamiiru_core::glob::Glob;
use itamiiru_core::mime::MimeDb;
use itamiiru_core::probe::{self, PathKind, ProbeResult};
use itamiiru_core::protocol::{Mode, Pattern, Request, Response};

use crate::theme::{self, Accent, Theme};
use crate::toolbar::dropdown;

/// Exit codes of the picker protocol.
pub const EXIT_OK: i32 = 0;
pub const EXIT_CANCELLED: i32 = 1;
pub const EXIT_ERROR: i32 = 2;

/// What a typed name is waiting to be checked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Pending {
    Open,
    Save,
    Folder,
}

/// What the footer asks the app to do.
pub enum FooterAction {
    Accept,
    Cancel,
}

/// What a checked name turned out to mean.
pub enum ProbeOutcome {
    Finish(Vec<PathBuf>),
    Navigate(PathBuf),
    ConfirmOverwrite(PathBuf),
    Error(String),
}

pub struct Picker {
    pub request: Request,
    /// The globs of each filter, with mime: types expanded.
    filter_globs: Vec<Vec<Glob>>,
    pub active_filter: Option<usize>,
    /// A glob typed into the name field, used instead of the active filter.
    typed_glob: Option<(String, Glob)>,
    /// The filter changed; the list needs filtering again.
    filter_changed: bool,
    pub name: String,
    /// Focus the name field (and select the stem, in save mode) next frame.
    name_needs_focus: bool,
    choice_values: Vec<String>,
    pub error: Option<String>,
    pub confirm_overwrite: Option<PathBuf>,
    /// (id, what for) of the probe being waited on.
    pending: Option<(u64, Pending)>,
    next_probe_id: u64,
    probe_tx: Sender<ProbeResult>,
    probe_rx: Receiver<ProbeResult>,
    /// The selection the name field last followed: (count, cursor).
    followed_selection: (usize, Option<PathBuf>),
    /// The name field holds a name copied from the selection, not typed.
    /// Such a name is cleared when the selection goes away, so it can't
    /// be picked by mistake (or looked for inside a folder just opened).
    name_from_selection: bool,
    exit_code: Arc<AtomicI32>,
}

impl Picker {
    /// `mime` is only needed when a filter has mime: patterns.
    pub fn new(request: Request, mime: Option<&MimeDb>, exit_code: Arc<AtomicI32>) -> Picker {
        let mut filter_globs = Vec::new();
        for filter in &request.filters {
            let mut globs = Vec::new();
            for pattern in &filter.patterns {
                match pattern {
                    Pattern::Glob(glob) => globs.push(Glob::new(glob)),
                    Pattern::Mime(mime_type) => {
                        if let Some(db) = mime {
                            globs.extend(db.globs_for(mime_type));
                        }
                    }
                }
            }
            filter_globs.push(globs);
        }
        let active_filter = match request.current_filter {
            Some(index) => Some(index),
            None if !request.filters.is_empty() => Some(0),
            None => None,
        };
        let mut choice_values = Vec::new();
        for choice in &request.choices {
            choice_values.push(choice.default.clone());
        }
        // Save mode starts with the suggested name, or the existing file's.
        let mut name = String::new();
        if request.mode == Mode::Save {
            if let Some(suggested) = &request.name {
                name = suggested.clone();
            } else if let Some(file) = &request.file
                && let Some(file_name) = file.file_name()
            {
                name = file_name.to_string_lossy().into_owned();
            }
        }
        let name_needs_focus = request.mode == Mode::Save;
        let (probe_tx, probe_rx) = mpsc::channel();
        return Picker {
            request,
            filter_globs,
            active_filter,
            typed_glob: None,
            filter_changed: false,
            name,
            name_needs_focus,
            choice_values,
            error: None,
            confirm_overwrite: None,
            pending: None,
            next_probe_id: 0,
            probe_tx,
            probe_rx,
            followed_selection: (0, None),
            name_from_selection: false,
            exit_code,
        };
    }

    /// True once after the filter changed.
    pub fn take_filter_changed(&mut self) -> bool {
        let changed = self.filter_changed;
        self.filter_changed = false;
        return changed;
    }

    /// The name the request suggested (save mode), to put back after
    /// navigating with a typed folder name.
    pub fn suggested_name(&self) -> String {
        return self.request.name.clone().unwrap_or_default();
    }

    pub fn mode(&self) -> Mode {
        return self.request.mode;
    }

    /// Folder modes pick folders; the others pick files.
    pub fn picks_folders(&self) -> bool {
        return matches!(self.request.mode, Mode::Folder | Mode::SaveFiles);
    }

    pub fn allows_multiple(&self) -> bool {
        return self.request.mode == Mode::Open && self.request.multiple;
    }

    pub fn window_title(&self) -> String {
        if !self.request.title.is_empty() {
            return self.request.title.clone();
        }
        return match self.request.mode {
            Mode::Open => "Open".to_string(),
            Mode::Save => "Save As".to_string(),
            Mode::Folder | Mode::SaveFiles => "Select Folder".to_string(),
        };
    }

    fn accept_label(&self) -> String {
        if let Some(label) = &self.request.accept_label
            && !label.is_empty()
        {
            return label.clone();
        }
        return match self.request.mode {
            Mode::Open => "Open".to_string(),
            Mode::Save => "Save".to_string(),
            Mode::Folder | Mode::SaveFiles => "Select Folder".to_string(),
        };
    }

    /// Whether an entry is listed: folders always (to browse into), files
    /// only outside folder modes and only if they match the filter.
    pub fn shows(&self, entry: &Entry) -> bool {
        if entry.is_dir() {
            return true;
        }
        if self.picks_folders() {
            return false;
        }
        let name = entry.name.to_string_lossy();
        if let Some((_, glob)) = &self.typed_glob {
            return glob.matches(&name);
        }
        let Some(index) = self.active_filter else {
            return true;
        };
        let Some(globs) = self.filter_globs.get(index) else {
            return true;
        };
        for glob in globs {
            if glob.matches(&name) {
                return true;
            }
        }
        return false;
    }

    /// Keeps the name field in step with the list selection, like Windows.
    /// `selected` are the selected entries; returns true if it changed.
    pub fn follow_selection(&mut self, selected: &[&Entry], cursor: Option<&Path>) {
        let signature = (selected.len(), cursor.map(Path::to_path_buf));
        if signature == self.followed_selection {
            return;
        }
        self.followed_selection = signature;
        if let Some(text) = name_for_selection(self.request.mode, selected) {
            self.name = text;
            self.name_from_selection = true;
            self.error = None;
            return;
        }
        // Nothing to copy any more (deselected, or another folder opened):
        // drop the old copied name. Save mode keeps it; that field holds
        // the name to save under, which a click elsewhere shouldn't erase.
        if self.name_from_selection && self.request.mode != Mode::Save {
            self.name.clear();
            self.name_from_selection = false;
        }
    }

    /// Called when the Open/Save button or Enter is used. `current` is the
    /// folder on screen, `selected` the selected entries, `resolve` turns a
    /// typed name into a path. Returns a decision the app acts on now, or
    /// None while a typed path is being checked (see `receive_probe`).
    pub fn accept(
        &mut self,
        current: &Path,
        selected: &[&Entry],
        ctx: &egui::Context,
        resolve: &dyn Fn(&str) -> PathBuf,
    ) -> Option<ProbeOutcome> {
        self.error = None;
        let text = self.name.trim().to_string();
        let mode = self.request.mode;

        if text.is_empty() {
            return match mode {
                Mode::Open => outcome_for_open_selection(selected, self.allows_multiple()),
                Mode::Save => None,
                Mode::Folder | Mode::SaveFiles => {
                    // The selected folder, or the folder on screen.
                    if selected.len() == 1 && selected[0].is_dir() {
                        return Some(ProbeOutcome::Finish(vec![selected[0].path.clone()]));
                    }
                    Some(ProbeOutcome::Finish(vec![current.to_path_buf()]))
                }
            };
        }

        // A typed glob filters the list instead (open and save modes).
        if !self.picks_folders() && Glob::has_wildcards(&text) && !text.starts_with('"') {
            self.typed_glob = Some((text.clone(), Glob::new(&text)));
            self.filter_changed = true;
            return None;
        }

        match mode {
            Mode::Open => {
                let names = parse_name_list(&text);
                if names.len() > 1 && !self.allows_multiple() {
                    return Some(ProbeOutcome::Error("Pick just one file.".to_string()));
                }
                let mut paths = Vec::new();
                for name in &names {
                    paths.push(resolve(name));
                }
                self.start_probe(paths, None, Pending::Open, ctx);
            }
            Mode::Save => {
                let name = append_extension(&text, self.single_extension().as_deref());
                let path = resolve(&name);
                let folder = path.parent().map(Path::to_path_buf);
                self.start_probe(vec![path], folder, Pending::Save, ctx);
            }
            Mode::Folder | Mode::SaveFiles => {
                self.start_probe(vec![resolve(&text)], None, Pending::Folder, ctx);
            }
        }
        return None;
    }

    fn start_probe(
        &mut self,
        paths: Vec<PathBuf>,
        writable: Option<PathBuf>,
        pending: Pending,
        ctx: &egui::Context,
    ) {
        self.next_probe_id += 1;
        self.pending = Some((self.next_probe_id, pending));
        let ctx = ctx.clone();
        probe::start(
            self.next_probe_id,
            paths,
            writable,
            self.probe_tx.clone(),
            Box::new(move || ctx.request_repaint()),
        );
    }

    /// The answer to a typed path, once the worker has checked it.
    pub fn receive_probe(&mut self) -> Option<ProbeOutcome> {
        while let Ok(result) = self.probe_rx.try_recv() {
            let Some((id, pending)) = self.pending else {
                continue;
            };
            if result.id != id {
                continue;
            }
            self.pending = None;
            return Some(decide(pending, &result));
        }
        return None;
    }

    /// The extension to add in save mode: the active filter's, if it has
    /// exactly one `*.ext` glob.
    fn single_extension(&self) -> Option<String> {
        let filter = self.request.filters.get(self.active_filter?)?;
        let mut found = None;
        for pattern in &filter.patterns {
            if let Pattern::Glob(glob) = pattern
                && let Some(ext) = glob.strip_prefix("*.")
                && !ext.is_empty()
                && !Glob::has_wildcards(ext)
            {
                if found.is_some() {
                    return None;
                }
                found = Some(ext.to_string());
            }
        }
        return found;
    }

    /// Writes the answer to stdout and records success. The app then closes.
    pub fn finish(&self, paths: &[PathBuf]) {
        let mut response = Response::default();
        for path in paths {
            response
                .uris
                .push(itamiiru_core::percent::path_to_file_uri(path));
        }
        if !self.request.filters.is_empty() {
            response.filter = self.active_filter;
        }
        for (choice, value) in self.request.choices.iter().zip(&self.choice_values) {
            response.choices.push((choice.id.clone(), value.clone()));
        }
        let text = itamiiru_core::protocol::write_response(&response);
        use std::io::Write;
        let mut out = std::io::stdout().lock();
        let written = out.write_all(text.as_bytes()).and_then(|_| out.flush());
        match written {
            Ok(()) => self.exit_code.store(EXIT_OK, Ordering::Relaxed),
            Err(err) => {
                eprintln!("itami: can't write the answer: {err}");
                self.exit_code.store(EXIT_ERROR, Ordering::Relaxed);
            }
        }
    }

    pub fn cancel(&self) {
        self.exit_code.store(EXIT_CANCELLED, Ordering::Relaxed);
    }

    /// The footer: name field and filter, then choices and the buttons.
    pub fn footer(&mut self, ui: &mut egui::Ui, theme: &Theme) -> Option<FooterAction> {
        let mut action = None;
        let label = if self.picks_folders() {
            "Folder:"
        } else {
            "File name:"
        };

        ui.horizontal(|ui| {
            ui.label(label);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                self.filter_dropdown(ui);
                let field_id = egui::Id::new("picker-name");
                let edit = egui::TextEdit::singleline(&mut self.name)
                    .id(field_id)
                    .desired_width(f32::INFINITY);
                let response = ui.add(edit);
                if self.name_needs_focus {
                    self.name_needs_focus = false;
                    response.request_focus();
                    select_stem(ui.ctx(), field_id, &self.name);
                }
                if response.changed() {
                    self.error = None;
                    self.name_from_selection = false;
                }
                if response.lost_focus() {
                    if ui.input(|i| i.key_pressed(Key::Enter)) {
                        action = Some(FooterAction::Accept);
                    } else if ui.input(|i| i.key_pressed(Key::Escape)) {
                        action = Some(FooterAction::Cancel);
                    }
                }
            });
        });
        ui.add_space(theme::GAP_SMALL);
        ui.horizontal(|ui| {
            self.choices(ui);
            if let Some(error) = &self.error {
                ui.colored_label(theme.error, error);
            }
            if self.pending.is_some() {
                ui.spinner();
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Right to left: the main action ends up last, after Cancel.
                if theme
                    .filled_button(ui, Accent::Primary, &self.accept_label())
                    .clicked()
                {
                    action = Some(FooterAction::Accept);
                }
                if ui.button("Cancel").clicked() {
                    action = Some(FooterAction::Cancel);
                }
            });
        });
        return action;
    }

    fn filter_dropdown(&mut self, ui: &mut egui::Ui) {
        if self.request.filters.is_empty() || self.picks_folders() {
            return;
        }
        let label = match (&self.typed_glob, self.active_filter) {
            (Some((text, _)), _) => text.clone(),
            (None, Some(index)) => self.request.filters[index].name.clone(),
            (None, None) => "All files".to_string(),
        };
        let mut chosen = None;
        dropdown(ui, &label, |ui| {
            for (index, filter) in self.request.filters.iter().enumerate() {
                let selected = self.typed_glob.is_none() && self.active_filter == Some(index);
                if ui.selectable_label(selected, &filter.name).clicked() {
                    chosen = Some(index);
                    ui.close();
                }
            }
        });
        if let Some(index) = chosen {
            self.active_filter = Some(index);
            self.typed_glob = None;
            self.filter_changed = true;
        }
    }

    /// The request's choices: checkboxes, or drop-downs of options.
    fn choices(&mut self, ui: &mut egui::Ui) {
        for (index, choice) in self.request.choices.iter().enumerate() {
            let value = &mut self.choice_values[index];
            if choice.options.is_empty() {
                let mut checked = value == "true";
                if ui.checkbox(&mut checked, &choice.label).changed() {
                    *value = if checked { "true" } else { "false" }.to_string();
                }
                continue;
            }
            ui.label(format!("{}:", choice.label));
            let mut current_label = value.clone();
            for (id, label) in &choice.options {
                if id == value {
                    current_label = label.clone();
                }
            }
            let mut picked = None;
            dropdown(ui, &current_label, |ui| {
                for (id, label) in &choice.options {
                    if ui.selectable_label(id == value, label).clicked() {
                        picked = Some(id.clone());
                        ui.close();
                    }
                }
            });
            if let Some(id) = picked {
                *value = id;
            }
            ui.add_space(theme::GAP);
        }
    }

    /// The overwrite question. Returns Some(true) to replace.
    pub fn overwrite_dialog(&mut self, ctx: &egui::Context, theme: &Theme) -> Option<bool> {
        let path = self.confirm_overwrite.clone()?;
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("confirm-overwrite")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading("Replace the file?");
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            ui.label(format!(
                "\"{name}\" already exists. Do you want to replace it?"
            ));
            ui.add_space(theme::GAP);
            theme::dialog_buttons(ui, |ui| {
                if theme
                    .filled_button(ui, Accent::Primary, "Replace")
                    .clicked()
                {
                    choice = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    choice = Some(false);
                }
            });
        });
        if choice.is_none() && modal.should_close() {
            choice = Some(false);
        }
        if choice.is_some() {
            self.confirm_overwrite = None;
        }
        return choice;
    }
}

/// Text for the name field from a selection, or None to leave it alone.
/// Open: selected files (quoted when several), or one selected folder.
/// Save: one selected file. Folder modes: one selected folder.
pub fn name_for_selection(mode: Mode, selected: &[&Entry]) -> Option<String> {
    let mut files = Vec::new();
    let mut folders = Vec::new();
    for entry in selected {
        let name = entry.name.to_string_lossy().into_owned();
        if entry.is_dir() {
            folders.push(name);
        } else {
            files.push(name);
        }
    }
    return match mode {
        Mode::Open if files.len() == 1 => Some(files.remove(0)),
        Mode::Open if files.len() > 1 => {
            let mut quoted = Vec::new();
            for name in files {
                quoted.push(format!("\"{name}\""));
            }
            Some(quoted.join(" "))
        }
        Mode::Open if folders.len() == 1 => Some(folders.remove(0)),
        Mode::Save if files.len() == 1 => Some(files.remove(0)),
        Mode::Folder | Mode::SaveFiles if folders.len() == 1 => Some(folders.remove(0)),
        _ => None,
    };
}

/// `"a.txt" "b c.txt"` -> [a.txt, b c.txt]; anything else is one name.
pub fn parse_name_list(text: &str) -> Vec<String> {
    let text = text.trim();
    if !text.starts_with('"') {
        return vec![text.to_string()];
    }
    let mut names = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('"') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('"') else {
            // Unbalanced: take the rest as a name.
            let name = after.trim();
            if !name.is_empty() {
                names.push(name.to_string());
            }
            break;
        };
        let name = &after[..end];
        if !name.is_empty() {
            names.push(name.to_string());
        }
        rest = &after[end + 1..];
    }
    return names;
}

/// Adds `.ext` when the name has no extension of its own.
pub fn append_extension(name: &str, extension: Option<&str>) -> String {
    let Some(extension) = extension else {
        return name.to_string();
    };
    if Path::new(name).extension().is_some() {
        return name.to_string();
    }
    return format!("{name}.{extension}");
}

/// Accepting in open mode with nothing typed: the selected files, or one
/// selected folder to browse into.
fn outcome_for_open_selection(selected: &[&Entry], multiple: bool) -> Option<ProbeOutcome> {
    let mut files = Vec::new();
    for entry in selected {
        if !entry.is_dir() {
            files.push(entry.path.clone());
        }
    }
    if !files.is_empty() {
        if !multiple {
            files.truncate(1);
        }
        return Some(ProbeOutcome::Finish(files));
    }
    if selected.len() == 1 {
        return Some(ProbeOutcome::Navigate(selected[0].path.clone()));
    }
    return None;
}

/// What a checked path means for each kind of request.
fn decide(pending: Pending, result: &ProbeResult) -> ProbeOutcome {
    let name_of = |path: &Path| match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    };
    match pending {
        Pending::Open => {
            if result.kinds.len() == 1 && result.kinds[0].1 == PathKind::Folder {
                return ProbeOutcome::Navigate(result.kinds[0].0.clone());
            }
            let mut paths = Vec::new();
            for (path, kind) in &result.kinds {
                match kind {
                    PathKind::File => paths.push(path.clone()),
                    PathKind::Folder => {
                        return ProbeOutcome::Error(format!("\"{}\" is a folder.", name_of(path)));
                    }
                    PathKind::Missing => {
                        return ProbeOutcome::Error(format!("\"{}\" wasn't found.", name_of(path)));
                    }
                }
            }
            return ProbeOutcome::Finish(paths);
        }
        Pending::Save => {
            let Some((path, kind)) = result.kinds.first() else {
                return ProbeOutcome::Error("Nothing to save.".to_string());
            };
            return match kind {
                PathKind::Folder => ProbeOutcome::Navigate(path.clone()),
                PathKind::File => ProbeOutcome::ConfirmOverwrite(path.clone()),
                PathKind::Missing if result.writable == Some(false) => ProbeOutcome::Error(
                    "You can't save in this folder (it's missing or read-only).".to_string(),
                ),
                PathKind::Missing => ProbeOutcome::Finish(vec![path.clone()]),
            };
        }
        Pending::Folder => {
            let Some((path, kind)) = result.kinds.first() else {
                return ProbeOutcome::Error("No folder given.".to_string());
            };
            return match kind {
                PathKind::Folder => ProbeOutcome::Finish(vec![path.clone()]),
                PathKind::File => {
                    ProbeOutcome::Error(format!("\"{}\" is not a folder.", name_of(path)))
                }
                PathKind::Missing => {
                    ProbeOutcome::Error(format!("\"{}\" wasn't found.", name_of(path)))
                }
            };
        }
    }
}

/// Selects the name without its extension: "untitled" in "untitled.txt".
fn select_stem(ctx: &egui::Context, id: egui::Id, text: &str) {
    use egui::text::{CCursor, CCursorRange};
    let stem_chars = match text.rfind('.') {
        Some(dot) if dot > 0 => text[..dot].chars().count(),
        _ => text.chars().count(),
    };
    let mut state = egui::TextEdit::load_state(ctx, id).unwrap_or_default();
    state.cursor.set_char_range(Some(CCursorRange::two(
        CCursor::new(0),
        CCursor::new(stem_chars),
    )));
    state.store(ctx, id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use itamiiru_core::entry::EntryKind;
    use itamiiru_core::protocol::Filter;

    fn entry(name: &str, kind: EntryKind) -> Entry {
        return Entry {
            name: name.into(),
            path: PathBuf::from("/d").join(name),
            kind,
            size: 0,
            mtime: None,
            hidden: false,
            executable: false,
        };
    }

    fn picker(mode: Mode) -> Picker {
        let mut request = Request::new(mode);
        request.filters.push(Filter {
            name: "Text".to_string(),
            patterns: vec![Pattern::Glob("*.txt".to_string())],
        });
        request.filters.push(Filter {
            name: "Images".to_string(),
            patterns: vec![
                Pattern::Mime("image/png".to_string()),
                Pattern::Glob("*.jpg".to_string()),
            ],
        });
        let db = MimeDb::built_in();
        return Picker::new(request, Some(&db), Arc::new(AtomicI32::new(EXIT_CANCELLED)));
    }

    #[test]
    fn copied_name_goes_away_with_the_selection() {
        let ctx = egui::Context::default();
        let resolve = |text: &str| PathBuf::from("/d").join(text);
        let photos = entry("photos", EntryKind::Dir);

        // Folder picker: click a folder, then click off it.
        let mut folders = picker(Mode::Folder);
        folders.follow_selection(&[&photos], Some(Path::new("/d/photos")));
        assert_eq!(folders.name, "photos");
        folders.follow_selection(&[], None);
        assert_eq!(folders.name, "");
        // Accept now picks the folder on screen, not the old click.
        let outcome = folders.accept(Path::new("/d"), &[], &ctx, &resolve);
        assert!(
            matches!(outcome, Some(ProbeOutcome::Finish(paths)) if paths == vec![PathBuf::from("/d")])
        );

        // Click a folder, then open it: the new folder's empty selection
        // clears the name, so Accept picks the opened folder.
        folders.follow_selection(&[&photos], Some(Path::new("/d/photos")));
        folders.follow_selection(&[], None);
        let outcome = folders.accept(Path::new("/d/photos"), &[], &ctx, &resolve);
        assert!(
            matches!(outcome, Some(ProbeOutcome::Finish(paths)) if paths == vec![PathBuf::from("/d/photos")])
        );

        // A typed name stays when the selection changes to nothing.
        folders.name = "typed".to_string();
        folders.name_from_selection = false;
        folders.follow_selection(&[], Some(Path::new("/d/x")));
        assert_eq!(folders.name, "typed");

        // Save mode keeps the name to save under.
        let mut save = picker(Mode::Save);
        let notes = entry("notes.txt", EntryKind::File);
        save.follow_selection(&[&notes], Some(Path::new("/d/notes.txt")));
        assert_eq!(save.name, "notes.txt");
        save.follow_selection(&[], None);
        assert_eq!(save.name, "notes.txt");
    }

    #[test]
    fn filters_decide_what_shows() {
        let mut p = picker(Mode::Open);
        assert!(p.shows(&entry("a.txt", EntryKind::File)));
        assert!(!p.shows(&entry("a.png", EntryKind::File)));
        assert!(
            p.shows(&entry("dir", EntryKind::Dir)),
            "folders always show"
        );
        p.active_filter = Some(1);
        assert!(
            p.shows(&entry("a.PNG", EntryKind::File)),
            "mime globs ignore case"
        );
        assert!(p.shows(&entry("a.jpg", EntryKind::File)));
        assert!(
            !p.shows(&entry("a.JPG", EntryKind::File)),
            "plain globs are case-sensitive"
        );
        let folders = picker(Mode::Folder);
        assert!(!folders.shows(&entry("a.txt", EntryKind::File)));
        assert!(folders.shows(&entry("dir", EntryKind::SymlinkToDir)));
    }

    #[test]
    fn save_extension() {
        let p = picker(Mode::Save);
        assert_eq!(p.single_extension().as_deref(), Some("txt"));
        assert_eq!(append_extension("notes", Some("txt")), "notes.txt");
        assert_eq!(append_extension("notes.md", Some("txt")), "notes.md");
        assert_eq!(append_extension("notes", None), "notes");
        let mut images = picker(Mode::Save);
        images.active_filter = Some(1);
        assert_eq!(
            images.single_extension(),
            Some("jpg".to_string()),
            "mime patterns don't count"
        );
    }

    #[test]
    fn name_lists() {
        assert_eq!(parse_name_list("plain name.txt"), ["plain name.txt"]);
        assert_eq!(
            parse_name_list("\"a.txt\" \"b c.txt\""),
            ["a.txt", "b c.txt"]
        );
        assert_eq!(
            parse_name_list("\"a.txt\" \"unterminated"),
            ["a.txt", "unterminated"]
        );
        assert_eq!(parse_name_list("\"\" \"x\""), ["x"]);
    }

    #[test]
    fn name_follows_selection() {
        let a = entry("a.txt", EntryKind::File);
        let b = entry("b.txt", EntryKind::File);
        let d = entry("dir", EntryKind::Dir);
        assert_eq!(
            name_for_selection(Mode::Open, &[&a]).as_deref(),
            Some("a.txt")
        );
        assert_eq!(
            name_for_selection(Mode::Open, &[&a, &b]).as_deref(),
            Some("\"a.txt\" \"b.txt\"")
        );
        assert_eq!(
            name_for_selection(Mode::Open, &[&d]).as_deref(),
            Some("dir")
        );
        assert_eq!(name_for_selection(Mode::Open, &[]), None);
        assert_eq!(name_for_selection(Mode::Save, &[&d]), None);
        assert_eq!(
            name_for_selection(Mode::Folder, &[&d]).as_deref(),
            Some("dir")
        );
        assert_eq!(name_for_selection(Mode::Folder, &[&a]), None);
    }

    #[test]
    fn decisions() {
        let result = |kinds: Vec<(&str, PathKind)>, writable| ProbeResult {
            id: 1,
            kinds: kinds
                .into_iter()
                .map(|(p, k)| (PathBuf::from(p), k))
                .collect(),
            writable,
        };
        assert!(matches!(
            decide(Pending::Open, &result(vec![("/a", PathKind::File)], None)),
            ProbeOutcome::Finish(_)
        ));
        assert!(matches!(
            decide(Pending::Open, &result(vec![("/d", PathKind::Folder)], None)),
            ProbeOutcome::Navigate(_)
        ));
        assert!(matches!(
            decide(
                Pending::Open,
                &result(vec![("/x", PathKind::Missing)], None)
            ),
            ProbeOutcome::Error(_)
        ));
        assert!(matches!(
            decide(
                Pending::Save,
                &result(vec![("/a", PathKind::File)], Some(true))
            ),
            ProbeOutcome::ConfirmOverwrite(_)
        ));
        assert!(matches!(
            decide(
                Pending::Save,
                &result(vec![("/new", PathKind::Missing)], Some(false))
            ),
            ProbeOutcome::Error(_)
        ));
        assert!(matches!(
            decide(
                Pending::Save,
                &result(vec![("/new", PathKind::Missing)], Some(true))
            ),
            ProbeOutcome::Finish(_)
        ));
        assert!(matches!(
            decide(Pending::Folder, &result(vec![("/a", PathKind::File)], None)),
            ProbeOutcome::Error(_)
        ));
    }

    #[test]
    fn titles_and_labels() {
        let p = picker(Mode::Save);
        assert_eq!(p.window_title(), "Save As");
        assert_eq!(p.accept_label(), "Save");
        assert!(picker(Mode::SaveFiles).picks_folders());
    }
}
