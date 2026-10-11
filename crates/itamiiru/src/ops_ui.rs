//! The UI side of file operations: the progress panel, the conflict dialog,
//! the delete confirmation and the error banner.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use eframe::egui;
use itamiiru_core::fileops::{self, Answer, Op, OpHandle, OpMsg, Progress, Resolution};

use crate::theme::{self, Accent, Theme};

/// Error messages kept for the banner; older ones are dropped.
const MAX_ERRORS: usize = 100;

/// A file operation running on its worker thread.
pub struct RunningOp {
    handle: OpHandle,
    title: String,
    progress: Progress,
    /// (the item being copied or moved, the existing item in its way)
    conflict: Option<(PathBuf, PathBuf)>,
    apply_to_all: bool,
    pub is_new_folder: bool,
}

impl RunningOp {
    pub fn start(op: Op, ctx: &egui::Context) -> RunningOp {
        let title = title_for(&op);
        let is_new_folder = matches!(op, Op::NewFolder { .. });
        let ctx = ctx.clone();
        let handle = fileops::start(op, Box::new(move || ctx.request_repaint()));
        return RunningOp {
            handle,
            title,
            progress: Progress::default(),
            conflict: None,
            apply_to_all: false,
            is_new_folder,
        };
    }

    /// Takes the worker's waiting messages. Failed items go to `errors`.
    /// Returns the created paths once the operation has finished.
    pub fn poll(&mut self, errors: &mut Vec<String>) -> Option<Vec<PathBuf>> {
        while let Ok(msg) = self.handle.messages.try_recv() {
            match msg {
                OpMsg::Progress(progress) => self.progress = progress,
                OpMsg::Conflict { source, existing } => self.conflict = Some((source, existing)),
                OpMsg::ItemFailed { path, error } => {
                    let name = display_name(&path);
                    push_error(errors, format!("{name}: {error}"));
                }
                OpMsg::Finished { created, .. } => return Some(created),
            }
        }
        return None;
    }

    pub fn cancel(&mut self) {
        self.handle.cancel.store(true, Ordering::Relaxed);
        // A worker waiting on a conflict needs an answer to notice.
        if self.conflict.take().is_some() {
            self.answer(Resolution::Cancel);
        }
    }

    pub fn has_conflict(&self) -> bool {
        return self.conflict.is_some();
    }

    fn answer(&mut self, resolution: Resolution) {
        let answer = Answer {
            resolution,
            apply_to_all: self.apply_to_all,
        };
        // Fails only if the worker is gone, which is fine.
        let _ = self.handle.answers.send(answer);
        self.conflict = None;
    }

    /// A strip with the title, the current file, a progress bar and Cancel.
    pub fn progress_panel(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(&self.title);
            let current = display_name(&self.progress.current);
            if !current.is_empty() {
                ui.weak(current);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("Cancel").clicked() {
                    self.cancel();
                }
                let bar = egui::ProgressBar::new(self.progress.fraction())
                    .desired_width(200.0)
                    .show_percentage();
                ui.add(bar);
            });
        });
    }

    /// Asks what to do when an item with the same name already exists.
    pub fn conflict_dialog(&mut self, ctx: &egui::Context, theme: &Theme) {
        let Some((source, existing)) = self.conflict.clone() else {
            return;
        };
        let mut choice = None;
        let modal = egui::Modal::new(egui::Id::new("conflict")).show(ctx, |ui| {
            ui.set_max_width(420.0);
            ui.heading("Replace or skip?");
            let folder = existing.parent().map(display_path).unwrap_or_default();
            ui.label(format!(
                "\"{}\" already exists in {}.",
                display_name(&existing),
                folder
            ));
            if source.is_dir() != existing.is_dir() {
                ui.weak("One is a folder and the other is a file.");
            }
            ui.add_space(theme::GAP);
            ui.checkbox(&mut self.apply_to_all, "Do this for all conflicts");
            ui.add_space(theme::GAP);
            ui.horizontal(|ui| {
                if ui.button("Cancel").clicked() {
                    choice = Some(Resolution::Cancel);
                }
                if ui.button("Skip").clicked() {
                    choice = Some(Resolution::Skip);
                }
                if ui.button("Keep both").clicked() {
                    choice = Some(Resolution::KeepBoth);
                }
                if theme
                    .outlined_button(ui, Accent::Error, "Replace")
                    .clicked()
                {
                    choice = Some(Resolution::Replace);
                }
            });
        });
        // Escape or a click outside the dialog cancels.
        if choice.is_none() && modal.should_close() {
            choice = Some(Resolution::Cancel);
        }
        if let Some(resolution) = choice {
            if resolution == Resolution::Cancel {
                self.handle.cancel.store(true, Ordering::Relaxed);
            }
            self.answer(resolution);
        }
    }
}

/// "Permanently delete 3 items?" Returns Some(true) to delete,
/// Some(false) to cancel, None while still open.
pub fn confirm_delete_dialog(
    ctx: &egui::Context,
    theme: &Theme,
    paths: &[PathBuf],
) -> Option<bool> {
    let mut choice = None;
    let modal = egui::Modal::new(egui::Id::new("confirm-delete")).show(ctx, |ui| {
        ui.set_max_width(420.0);
        let what = if paths.len() == 1 {
            format!("\"{}\"", display_name(&paths[0]))
        } else {
            format!("these {} items", paths.len())
        };
        ui.heading("Delete permanently?");
        ui.label(format!("Permanently delete {what}? This can't be undone."));
        ui.add_space(theme::GAP);
        ui.horizontal(|ui| {
            if ui.button("Cancel").clicked() {
                choice = Some(false);
            }
            if theme.filled_button(ui, Accent::Error, "Delete").clicked() {
                choice = Some(true);
            }
        });
    });
    if choice.is_none() && modal.should_close() {
        choice = Some(false);
    }
    return choice;
}

/// Lists failed items at the top of the file list, with a Dismiss button.
pub fn error_banner(ui: &mut egui::Ui, theme: &Theme, errors: &mut Vec<String>) {
    if errors.is_empty() {
        return;
    }
    let mut dismiss = false;
    egui::Frame::new()
        .fill(theme.error_container)
        .corner_radius(theme::RADIUS_MEDIUM)
        .inner_margin(theme::PADDING)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.colored_label(theme.on_error_container, "Some items failed:");
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Dismiss").clicked() {
                        dismiss = true;
                    }
                });
            });
            let shown = errors.len().min(5);
            for error in &errors[errors.len() - shown..] {
                ui.colored_label(theme.on_error_container, error);
            }
            if errors.len() > shown {
                let more = errors.len() - shown;
                ui.colored_label(theme.on_error_container, format!("...and {more} more"));
            }
        });
    ui.add_space(theme::GAP);
    if dismiss {
        errors.clear();
    }
}

pub fn push_error(errors: &mut Vec<String>, error: String) {
    if errors.len() >= MAX_ERRORS {
        errors.remove(0);
    }
    errors.push(error);
}

fn title_for(op: &Op) -> String {
    return match op {
        Op::Copy { sources, .. } => format!("Copying {}", count_text(sources.len())),
        Op::Move { sources, .. } => format!("Moving {}", count_text(sources.len())),
        Op::Trash { paths } => format!("Moving {} to the trash", count_text(paths.len())),
        Op::Delete { paths } => format!("Deleting {}", count_text(paths.len())),
        Op::Rename { .. } => "Renaming".to_string(),
        Op::NewFolder { .. } => "Creating a folder".to_string(),
    };
}

fn count_text(n: usize) -> String {
    if n == 1 {
        return "1 item".to_string();
    }
    return format!("{n} items");
}

fn display_name(path: &Path) -> String {
    return match path.file_name() {
        Some(name) => name.to_string_lossy().into_owned(),
        None => path.to_string_lossy().into_owned(),
    };
}

fn display_path(path: &Path) -> String {
    return path.to_string_lossy().into_owned();
}
