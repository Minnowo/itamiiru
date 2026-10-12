//! The only place for OS-specific code. Each OS file exposes the same functions.

/// A font file and the face to use in it (collections hold several).
pub type FontFile = (std::path::PathBuf, u32);

/// The system's own text fonts, when it has ones egui can use.
pub struct MainFonts {
    pub sans: Option<FontFile>,
    pub mono: Option<FontFile>,
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
