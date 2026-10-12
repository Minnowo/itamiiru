//! The only place for OS-specific code. Each OS file exposes the same functions.

/// A font file and the face to use in it (collections hold several).
pub type FontFile = (std::path::PathBuf, u32);

/// The system's own text fonts, when it has ones egui can use.
pub struct MainFonts {
    pub sans: Option<FontFile>,
    pub mono: Option<FontFile>,
}

/// Where a drag out to another app stands, as the drag thread sees it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DragState {
    /// Not over a window that takes drops.
    Nothing,
    /// Over one that said no.
    Refused,
    /// Over one that will take the drop.
    Accepted,
    /// Dropped, cancelled or given up: the drag is over.
    Ended,
}

/// The app's side of a drag out: it reads the state (for the cursor) and
/// can cancel (Escape). Cloning shares the same drag.
#[derive(Clone, Debug, Default)]
pub struct DragOut {
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
    state: std::sync::Arc<std::sync::atomic::AtomicU8>,
}

impl DragOut {
    pub fn new() -> DragOut {
        return DragOut::default();
    }

    /// Stops the drag; nothing gets dropped.
    pub fn cancel(&self) {
        self.cancel
            .store(true, std::sync::atomic::Ordering::Relaxed);
    }

    pub fn is_cancelled(&self) -> bool {
        return self.cancel.load(std::sync::atomic::Ordering::Relaxed);
    }

    pub fn state(&self) -> DragState {
        return match self.state.load(std::sync::atomic::Ordering::Relaxed) {
            1 => DragState::Refused,
            2 => DragState::Accepted,
            3 => DragState::Ended,
            _ => DragState::Nothing,
        };
    }

    pub fn set_state(&self, state: DragState) {
        let value = match state {
            DragState::Nothing => 0,
            DragState::Refused => 1,
            DragState::Accepted => 2,
            DragState::Ended => 3,
        };
        self.state
            .store(value, std::sync::atomic::Ordering::Relaxed);
    }
}

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
pub use linux::*;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
pub use windows::*;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
pub use macos::*;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_handles_share_state() {
        let app_side = DragOut::new();
        let thread_side = app_side.clone();
        assert_eq!(app_side.state(), DragState::Nothing);
        for state in [DragState::Refused, DragState::Accepted, DragState::Ended] {
            thread_side.set_state(state);
            assert_eq!(app_side.state(), state);
        }
        assert!(!thread_side.is_cancelled());
        app_side.cancel();
        assert!(thread_side.is_cancelled());
    }
}
