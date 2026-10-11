//! Colors and style, following the ponkotsu design system
//! (https://github.com/Minnowo/ponkotsu-css).
//!
//! A theme is a set of named color roles. Text on fill `X` uses `on-X`.
//! Depth comes from `surface-container-N` fills, not borders or shadows.
//!
//! A theme file replaces some or all of the built-in colors. It accepts
//! `primary = #dcc496` lines, and also ponkotsu's CSS lines
//! (`--color-c-primary: #dcc496;`), so one generated block can be pasted in.

// This is the whole design system; some parts aren't used by any screen yet.
#![allow(dead_code)]

use std::path::Path;

use eframe::egui::{self, Color32, CornerRadius, FontId, Margin, Shadow, Stroke, TextStyle};

/// Theme files bigger than this are ignored. A full palette is about 4 KiB.
const MAX_THEME_FILE_SIZE: u64 = 64 * 1024;

/// Corner radius for inputs, buttons and chips (`rounded-sm`).
pub const RADIUS_SMALL: u8 = 4;
/// Corner radius for containers (`rounded-md`).
pub const RADIUS_MEDIUM: u8 = 8;
/// Spacing inside one control, e.g. an icon and its text (`gap-1`).
pub const GAP_SMALL: f32 = 4.0;
/// Spacing between related rows, fields or buttons (`gap-2`).
pub const GAP: f32 = 8.0;
/// Spacing between sections or panels (`gap-4`).
pub const GAP_LARGE: f32 = 16.0;
/// Padding inside a surface box (`p-3`).
pub const PADDING: i8 = 12;
/// File icons in the details view, in points (pixels at 100% zoom).
pub const ICON_SIZE: f32 = 16.0;

/// The accent roles that have filled and outlined buttons.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Accent {
    Primary,
    Secondary,
    Tertiary,
    Success,
    Error,
}

/// Every color role. Field names match ponkotsu's role names with `-` as `_`.
#[derive(Clone, Debug, PartialEq)]
pub struct Theme {
    pub primary: Color32,
    pub on_primary: Color32,
    pub primary_container: Color32,
    pub on_primary_container: Color32,
    pub secondary: Color32,
    pub on_secondary: Color32,
    pub secondary_container: Color32,
    pub on_secondary_container: Color32,
    pub tertiary: Color32,
    pub on_tertiary: Color32,
    pub tertiary_container: Color32,
    pub on_tertiary_container: Color32,
    pub error: Color32,
    pub on_error: Color32,
    pub error_container: Color32,
    pub on_error_container: Color32,
    pub success: Color32,
    pub on_success: Color32,
    pub success_container: Color32,
    pub on_success_container: Color32,
    pub surface: Color32,
    pub on_surface: Color32,
    pub on_surface_variant: Color32,
    pub surface_container_1: Color32,
    pub surface_container_2: Color32,
    pub surface_container_3: Color32,
    pub surface_container_4: Color32,
    pub surface_container_5: Color32,
    pub surface_container_6: Color32,
    pub surface_dim: Color32,
    pub surface_bright: Color32,
    pub outline: Color32,
    pub outline_variant: Color32,
    pub inverse_surface: Color32,
    pub inverse_on_surface: Color32,
    pub inverse_primary: Color32,
    pub pink: Color32,
    pub on_pink: Color32,
    pub d_pink: Color32,
    pub on_d_pink: Color32,
    pub l_pink: Color32,
    pub on_l_pink: Color32,
    pub red: Color32,
    pub on_red: Color32,
    pub d_red: Color32,
    pub on_d_red: Color32,
    pub l_red: Color32,
    pub on_l_red: Color32,
    pub yellow: Color32,
    pub on_yellow: Color32,
    pub d_yellow: Color32,
    pub on_d_yellow: Color32,
    pub l_yellow: Color32,
    pub on_l_yellow: Color32,
    pub green: Color32,
    pub on_green: Color32,
    pub d_green: Color32,
    pub on_d_green: Color32,
    pub l_green: Color32,
    pub on_l_green: Color32,
    pub blue: Color32,
    pub on_blue: Color32,
    pub d_blue: Color32,
    pub on_d_blue: Color32,
    pub l_blue: Color32,
    pub on_l_blue: Color32,
    pub primary_hover: Color32,
    pub primary_focus: Color32,
    pub secondary_hover: Color32,
    pub secondary_focus: Color32,
    pub tertiary_hover: Color32,
    pub tertiary_focus: Color32,
    pub error_hover: Color32,
    pub error_focus: Color32,
    pub success_hover: Color32,
    pub success_focus: Color32,
    pub surface_container_1_hover: Color32,
    pub surface_container_1_focus: Color32,
    pub primary_hover_tint: Color32,
    pub primary_focus_tint: Color32,
    pub secondary_hover_tint: Color32,
    pub secondary_focus_tint: Color32,
    pub tertiary_hover_tint: Color32,
    pub tertiary_focus_tint: Color32,
    pub error_hover_tint: Color32,
    pub error_focus_tint: Color32,
    pub success_hover_tint: Color32,
    pub success_focus_tint: Color32,
    pub on_surface_hover_tint: Color32,
    pub on_surface_focus_tint: Color32,
}

impl Default for Theme {
    /// ponkotsu's default dark palette.
    fn default() -> Self {
        return Theme {
            primary: Color32::from_rgb(0xdc, 0xc4, 0x96),
            on_primary: Color32::from_rgb(0x40, 0x2d, 0x00),
            primary_container: Color32::from_rgb(0x6e, 0x5c, 0x36),
            on_primary_container: Color32::from_rgb(0xff, 0xff, 0xff),
            secondary: Color32::from_rgb(0xa7, 0xca, 0xec),
            on_secondary: Color32::from_rgb(0x00, 0x33, 0x51),
            secondary_container: Color32::from_rgb(0x3f, 0x62, 0x7f),
            on_secondary_container: Color32::from_rgb(0xff, 0xff, 0xff),
            tertiary: Color32::from_rgb(0xe1, 0xb6, 0xfd),
            on_tertiary: Color32::from_rgb(0x4c, 0x00, 0x7b),
            tertiary_container: Color32::from_rgb(0x73, 0x4f, 0x8e),
            on_tertiary_container: Color32::from_rgb(0xff, 0xff, 0xff),
            error: Color32::from_rgb(0xed, 0xbc, 0xa4),
            on_error: Color32::from_rgb(0x52, 0x23, 0x05),
            error_container: Color32::from_rgb(0x7b, 0x56, 0x42),
            on_error_container: Color32::from_rgb(0xff, 0xff, 0xff),
            success: Color32::from_rgb(0xb7, 0xcd, 0xb0),
            on_success: Color32::from_rgb(0x15, 0x37, 0x14),
            success_container: Color32::from_rgb(0x51, 0x63, 0x4c),
            on_success_container: Color32::from_rgb(0xff, 0xff, 0xff),
            surface: Color32::from_rgb(0x2a, 0x27, 0x24),
            on_surface: Color32::from_rgb(0xe8, 0xe1, 0xdb),
            on_surface_variant: Color32::from_rgb(0xce, 0xc5, 0xb9),
            surface_container_1: Color32::from_rgb(0x35, 0x32, 0x2e),
            surface_container_2: Color32::from_rgb(0x40, 0x3d, 0x39),
            surface_container_3: Color32::from_rgb(0x4c, 0x49, 0x44),
            surface_container_4: Color32::from_rgb(0x58, 0x54, 0x50),
            surface_container_5: Color32::from_rgb(0x64, 0x60, 0x5b),
            surface_container_6: Color32::from_rgb(0x70, 0x6c, 0x67),
            surface_dim: Color32::from_rgb(0x2a, 0x27, 0x24),
            surface_bright: Color32::from_rgb(0x53, 0x4f, 0x4b),
            outline: Color32::from_rgb(0xb2, 0xaa, 0x9f),
            outline_variant: Color32::from_rgb(0x64, 0x5e, 0x54),
            inverse_surface: Color32::from_rgb(0xff, 0xff, 0xff),
            inverse_on_surface: Color32::from_rgb(0x4a, 0x46, 0x42),
            inverse_primary: Color32::from_rgb(0x91, 0x73, 0x2c),
            pink: Color32::from_rgb(0xd8, 0x9d, 0xbd),
            on_pink: Color32::from_rgb(0x33, 0x0c, 0x26),
            d_pink: Color32::from_rgb(0xba, 0x84, 0xb8),
            on_d_pink: Color32::from_rgb(0x32, 0x08, 0x36),
            l_pink: Color32::from_rgb(0xe8, 0xba, 0xd1),
            on_l_pink: Color32::from_rgb(0x2e, 0x12, 0x23),
            red: Color32::from_rgb(0xe2, 0x9e, 0x7b),
            on_red: Color32::from_rgb(0x34, 0x11, 0x00),
            d_red: Color32::from_rgb(0xd2, 0x80, 0x6f),
            on_d_red: Color32::from_rgb(0x3a, 0x0a, 0x03),
            l_red: Color32::from_rgb(0xf1, 0xbb, 0x9f),
            on_l_red: Color32::from_rgb(0x30, 0x14, 0x04),
            yellow: Color32::from_rgb(0xc2, 0xad, 0x67),
            on_yellow: Color32::from_rgb(0x23, 0x1b, 0x00),
            d_yellow: Color32::from_rgb(0xbb, 0x8e, 0x41),
            on_d_yellow: Color32::from_rgb(0x28, 0x19, 0x00),
            l_yellow: Color32::from_rgb(0xd7, 0xc6, 0x90),
            on_l_yellow: Color32::from_rgb(0x23, 0x1b, 0x00),
            green: Color32::from_rgb(0x91, 0xb8, 0x89),
            on_green: Color32::from_rgb(0x01, 0x22, 0x03),
            d_green: Color32::from_rgb(0x85, 0x9f, 0x5a),
            on_d_green: Color32::from_rgb(0x12, 0x1f, 0x00),
            l_green: Color32::from_rgb(0xb2, 0xce, 0xaa),
            on_l_green: Color32::from_rgb(0x0a, 0x20, 0x09),
            blue: Color32::from_rgb(0x84, 0xb4, 0xd2),
            on_blue: Color32::from_rgb(0x00, 0x1e, 0x2d),
            d_blue: Color32::from_rgb(0x52, 0xa0, 0xb7),
            on_d_blue: Color32::from_rgb(0x00, 0x1f, 0x27),
            l_blue: Color32::from_rgb(0xab, 0xcb, 0xe1),
            on_l_blue: Color32::from_rgb(0x00, 0x1e, 0x2d),
            primary_hover: Color32::from_rgb(0xc8, 0xb0, 0x83),
            primary_focus: Color32::from_rgb(0xb4, 0x9d, 0x71),
            secondary_hover: Color32::from_rgb(0x92, 0xb6, 0xd8),
            secondary_focus: Color32::from_rgb(0x7e, 0xa3, 0xc5),
            tertiary_hover: Color32::from_rgb(0xce, 0xa2, 0xed),
            tertiary_focus: Color32::from_rgb(0xba, 0x8d, 0xdd),
            error_hover: Color32::from_rgb(0xd9, 0xa8, 0x90),
            error_focus: Color32::from_rgb(0xc6, 0x95, 0x7c),
            success_hover: Color32::from_rgb(0xa2, 0xba, 0x9b),
            success_focus: Color32::from_rgb(0x8d, 0xa6, 0x87),
            surface_container_1_hover: Color32::from_rgb(0x48, 0x44, 0x40),
            surface_container_1_focus: Color32::from_rgb(0x5c, 0x58, 0x53),
            primary_hover_tint: Color32::from_rgba_unmultiplied(0xdc, 0xc4, 0x96, 0x1f),
            primary_focus_tint: Color32::from_rgba_unmultiplied(0xdc, 0xc4, 0x96, 0x3d),
            secondary_hover_tint: Color32::from_rgba_unmultiplied(0xa7, 0xca, 0xec, 0x1f),
            secondary_focus_tint: Color32::from_rgba_unmultiplied(0xa7, 0xca, 0xec, 0x3d),
            tertiary_hover_tint: Color32::from_rgba_unmultiplied(0xe1, 0xb6, 0xfd, 0x1f),
            tertiary_focus_tint: Color32::from_rgba_unmultiplied(0xe1, 0xb6, 0xfd, 0x3d),
            error_hover_tint: Color32::from_rgba_unmultiplied(0xed, 0xbc, 0xa4, 0x1f),
            error_focus_tint: Color32::from_rgba_unmultiplied(0xed, 0xbc, 0xa4, 0x3d),
            success_hover_tint: Color32::from_rgba_unmultiplied(0xb7, 0xcd, 0xb0, 0x1f),
            success_focus_tint: Color32::from_rgba_unmultiplied(0xb7, 0xcd, 0xb0, 0x3d),
            on_surface_hover_tint: Color32::from_rgba_unmultiplied(0xe8, 0xe1, 0xdb, 0x1f),
            on_surface_focus_tint: Color32::from_rgba_unmultiplied(0xe8, 0xe1, 0xdb, 0x3d),
        };
    }
}

impl Theme {
    /// The built-in theme, with `$ITAMI_THEME` (a file path) or
    /// `<config dir>/theme` applied on top if either exists.
    /// Problems are logged and the built-in colors are kept.
    pub fn load() -> Theme {
        let mut theme = Theme::default();
        let path = match std::env::var_os("ITAMI_THEME") {
            Some(path) => std::path::PathBuf::from(path),
            None => match itamiiru_core::platform::config_dir() {
                Some(dir) => dir.join("theme"),
                None => return theme,
            },
        };
        if let Err(err) = theme.apply_file(&path) {
            itamiiru_core::debug!("theme: not using {}: {}", path.display(), err);
        }
        return theme;
    }

    /// Reads a theme file and applies its colors to `self`.
    pub fn apply_file(&mut self, path: &Path) -> Result<(), String> {
        let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if size > MAX_THEME_FILE_SIZE {
            return Err(format!(
                "file is {size} bytes, the limit is {MAX_THEME_FILE_SIZE}"
            ));
        }
        let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        self.apply_text(&text);
        itamiiru_core::debug!("theme: loaded {}", path.display());
        return Ok(());
    }

    /// Applies every `role = #hex` or `--color-c-role: #hex;` line in `text`.
    /// Anything else (comments, braces, unknown roles, bad colors) is skipped.
    pub fn apply_text(&mut self, text: &str) {
        for (index, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with("/*") {
                continue;
            }
            let Some(sep) = line.find([':', '=']) else {
                continue;
            };
            let key = line[..sep].trim();
            let key = key.strip_prefix("--color-c-").unwrap_or(key);
            let value = line[sep + 1..].trim().trim_end_matches(';').trim();

            let Some(color) = parse_hex_color(value) else {
                itamiiru_core::debug!("theme: line {}: bad color {:?}", index + 1, value);
                continue;
            };
            if !self.set(key, color) {
                itamiiru_core::debug!("theme: line {}: unknown role {:?}", index + 1, key);
            }
        }
    }

    /// Sets one role by its ponkotsu name. Returns false for an unknown name.
    pub fn set(&mut self, role: &str, color: Color32) -> bool {
        match role {
            "primary" => self.primary = color,
            "on-primary" => self.on_primary = color,
            "primary-container" => self.primary_container = color,
            "on-primary-container" => self.on_primary_container = color,
            "secondary" => self.secondary = color,
            "on-secondary" => self.on_secondary = color,
            "secondary-container" => self.secondary_container = color,
            "on-secondary-container" => self.on_secondary_container = color,
            "tertiary" => self.tertiary = color,
            "on-tertiary" => self.on_tertiary = color,
            "tertiary-container" => self.tertiary_container = color,
            "on-tertiary-container" => self.on_tertiary_container = color,
            "error" => self.error = color,
            "on-error" => self.on_error = color,
            "error-container" => self.error_container = color,
            "on-error-container" => self.on_error_container = color,
            "success" => self.success = color,
            "on-success" => self.on_success = color,
            "success-container" => self.success_container = color,
            "on-success-container" => self.on_success_container = color,
            "surface" => self.surface = color,
            "on-surface" => self.on_surface = color,
            "on-surface-variant" => self.on_surface_variant = color,
            "surface-container-1" => self.surface_container_1 = color,
            "surface-container-2" => self.surface_container_2 = color,
            "surface-container-3" => self.surface_container_3 = color,
            "surface-container-4" => self.surface_container_4 = color,
            "surface-container-5" => self.surface_container_5 = color,
            "surface-container-6" => self.surface_container_6 = color,
            "surface-dim" => self.surface_dim = color,
            "surface-bright" => self.surface_bright = color,
            "outline" => self.outline = color,
            "outline-variant" => self.outline_variant = color,
            "inverse-surface" => self.inverse_surface = color,
            "inverse-on-surface" => self.inverse_on_surface = color,
            "inverse-primary" => self.inverse_primary = color,
            "pink" => self.pink = color,
            "on-pink" => self.on_pink = color,
            "d-pink" => self.d_pink = color,
            "on-d-pink" => self.on_d_pink = color,
            "l-pink" => self.l_pink = color,
            "on-l-pink" => self.on_l_pink = color,
            "red" => self.red = color,
            "on-red" => self.on_red = color,
            "d-red" => self.d_red = color,
            "on-d-red" => self.on_d_red = color,
            "l-red" => self.l_red = color,
            "on-l-red" => self.on_l_red = color,
            "yellow" => self.yellow = color,
            "on-yellow" => self.on_yellow = color,
            "d-yellow" => self.d_yellow = color,
            "on-d-yellow" => self.on_d_yellow = color,
            "l-yellow" => self.l_yellow = color,
            "on-l-yellow" => self.on_l_yellow = color,
            "green" => self.green = color,
            "on-green" => self.on_green = color,
            "d-green" => self.d_green = color,
            "on-d-green" => self.on_d_green = color,
            "l-green" => self.l_green = color,
            "on-l-green" => self.on_l_green = color,
            "blue" => self.blue = color,
            "on-blue" => self.on_blue = color,
            "d-blue" => self.d_blue = color,
            "on-d-blue" => self.on_d_blue = color,
            "l-blue" => self.l_blue = color,
            "on-l-blue" => self.on_l_blue = color,
            "primary-hover" => self.primary_hover = color,
            "primary-focus" => self.primary_focus = color,
            "secondary-hover" => self.secondary_hover = color,
            "secondary-focus" => self.secondary_focus = color,
            "tertiary-hover" => self.tertiary_hover = color,
            "tertiary-focus" => self.tertiary_focus = color,
            "error-hover" => self.error_hover = color,
            "error-focus" => self.error_focus = color,
            "success-hover" => self.success_hover = color,
            "success-focus" => self.success_focus = color,
            "surface-container-1-hover" => self.surface_container_1_hover = color,
            "surface-container-1-focus" => self.surface_container_1_focus = color,
            "primary-hover-tint" => self.primary_hover_tint = color,
            "primary-focus-tint" => self.primary_focus_tint = color,
            "secondary-hover-tint" => self.secondary_hover_tint = color,
            "secondary-focus-tint" => self.secondary_focus_tint = color,
            "tertiary-hover-tint" => self.tertiary_hover_tint = color,
            "tertiary-focus-tint" => self.tertiary_focus_tint = color,
            "error-hover-tint" => self.error_hover_tint = color,
            "error-focus-tint" => self.error_focus_tint = color,
            "success-hover-tint" => self.success_hover_tint = color,
            "success-focus-tint" => self.success_focus_tint = color,
            "on-surface-hover-tint" => self.on_surface_hover_tint = color,
            "on-surface-focus-tint" => self.on_surface_focus_tint = color,
            _ => return false,
        }
        return true;
    }

    /// The background of a box nested `level` deep (1 to 6, clamped).
    pub fn surface_color(&self, level: u8) -> Color32 {
        return match level {
            0 | 1 => self.surface_container_1,
            2 => self.surface_container_2,
            3 => self.surface_container_3,
            4 => self.surface_container_4,
            5 => self.surface_container_5,
            _ => self.surface_container_6,
        };
    }

    /// A container box (`surface-N`): tinted fill, padding and rounding, no border.
    /// Bump `level` by one for each box nested inside another.
    pub fn surface(&self, level: u8) -> egui::Frame {
        return egui::Frame::new()
            .fill(self.surface_color(level))
            .corner_radius(RADIUS_MEDIUM)
            .inner_margin(PADDING);
    }

    /// A filled button for the one main action of a group (`btn-primary` etc.).
    pub fn filled_button(&self, ui: &mut egui::Ui, accent: Accent, text: &str) -> egui::Response {
        let (fill, hover, focus, on) = match accent {
            Accent::Primary => (
                self.primary,
                self.primary_hover,
                self.primary_focus,
                self.on_primary,
            ),
            Accent::Secondary => (
                self.secondary,
                self.secondary_hover,
                self.secondary_focus,
                self.on_secondary,
            ),
            Accent::Tertiary => (
                self.tertiary,
                self.tertiary_hover,
                self.tertiary_focus,
                self.on_tertiary,
            ),
            Accent::Success => (
                self.success,
                self.success_hover,
                self.success_focus,
                self.on_success,
            ),
            Accent::Error => (
                self.error,
                self.error_hover,
                self.error_focus,
                self.on_error,
            ),
        };
        return button_with_colors(ui, text, [fill, hover, focus], Stroke::NONE, on);
    }

    /// An outlined button for a colored secondary action (`btn-outlined-error` etc.).
    pub fn outlined_button(&self, ui: &mut egui::Ui, accent: Accent, text: &str) -> egui::Response {
        let (color, hover, focus) = match accent {
            Accent::Primary => (
                self.primary,
                self.primary_hover_tint,
                self.primary_focus_tint,
            ),
            Accent::Secondary => (
                self.secondary,
                self.secondary_hover_tint,
                self.secondary_focus_tint,
            ),
            Accent::Tertiary => (
                self.tertiary,
                self.tertiary_hover_tint,
                self.tertiary_focus_tint,
            ),
            Accent::Success => (
                self.success,
                self.success_hover_tint,
                self.success_focus_tint,
            ),
            Accent::Error => (self.error, self.error_hover_tint, self.error_focus_tint),
        };
        let fills = [Color32::TRANSPARENT, hover, focus];
        return button_with_colors(ui, text, fills, Stroke::new(1.0, color), color);
    }

    /// True when the page background is dark.
    pub fn is_dark(&self) -> bool {
        let c = self.surface;
        // Rough perceived brightness, 0 to 255.
        let brightness = (c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114) / 1000;
        return brightness < 128;
    }

    /// Installs this theme as egui's style. Plain egui widgets then follow it:
    /// `ui.button` is ponkotsu's plain button, text edits are its inputs, and so on.
    pub fn install(&self, ctx: &egui::Context) {
        // Use these visuals whatever light/dark mode the OS asks for.
        ctx.all_styles_mut(|style| self.apply_to_style(style));
    }

    fn apply_to_style(&self, style: &mut egui::Style) {
        // Desktop sizes: ponkotsu's web sizes scaled from a 16 px to a 14 px body.
        style
            .text_styles
            .insert(TextStyle::Heading, FontId::proportional(19.0));
        style
            .text_styles
            .insert(TextStyle::Body, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Button, FontId::proportional(14.0));
        style
            .text_styles
            .insert(TextStyle::Small, FontId::proportional(12.0));
        style
            .text_styles
            .insert(TextStyle::Monospace, FontId::monospace(13.0));

        let spacing = &mut style.spacing;
        spacing.item_spacing = egui::vec2(GAP, GAP_SMALL);
        spacing.button_padding = egui::vec2(12.0, 4.0);
        spacing.window_margin = Margin::same(PADDING);
        spacing.menu_margin = Margin::same(GAP_SMALL as i8);

        let v = &mut style.visuals;
        v.dark_mode = self.is_dark();
        v.override_text_color = None;
        v.weak_text_color = Some(self.on_surface_variant);
        v.hyperlink_color = self.primary;
        v.panel_fill = self.surface;
        v.window_fill = self.surface_container_1;
        // Floating windows and menus sit over arbitrary content, so they get
        // ponkotsu's opt-in edge (`surface-border`) instead of a shadow.
        v.window_stroke = Stroke::new(1.0, self.outline_variant);
        v.window_shadow = Shadow::NONE;
        v.popup_shadow = Shadow::NONE;
        v.window_corner_radius = CornerRadius::same(RADIUS_MEDIUM);
        v.menu_corner_radius = CornerRadius::same(RADIUS_MEDIUM);
        v.faint_bg_color = self.surface_container_1;
        v.extreme_bg_color = self.surface_container_1;
        v.text_edit_bg_color = Some(self.surface_container_1);
        v.code_bg_color = self.surface_container_1;
        v.warn_fg_color = self.yellow;
        v.error_fg_color = self.error;
        v.text_cursor.stroke = Stroke::new(2.0, self.primary);
        // Selected rows and selected text: "selected but not the main action".
        v.selection.bg_fill = self.secondary_container;
        v.selection.stroke = Stroke::new(1.0, self.on_secondary_container);

        let radius = CornerRadius::same(RADIUS_SMALL);
        let w = &mut v.widgets;
        // Labels, separators, frames.
        w.noninteractive.bg_fill = self.surface;
        w.noninteractive.weak_bg_fill = self.surface;
        w.noninteractive.bg_stroke = Stroke::new(1.0, self.outline_variant);
        w.noninteractive.fg_stroke = Stroke::new(1.0, self.on_surface);
        w.noninteractive.corner_radius = radius;
        // Buttons, inputs and checkboxes at rest, hovered, pressed and open:
        // ponkotsu's `input-like`.
        let states = [
            (&mut w.inactive, self.surface_container_1),
            (&mut w.hovered, self.surface_container_1_hover),
            (&mut w.active, self.surface_container_1_focus),
            (&mut w.open, self.surface_container_1_focus),
        ];
        for (state, fill) in states {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = Stroke::new(1.0, self.outline);
            state.fg_stroke = Stroke::new(1.0, self.on_surface);
            state.corner_radius = radius;
            state.expansion = 0.0;
        }
    }
}

/// An egui button with its own fill for the rest, hover and pressed states.
fn button_with_colors(
    ui: &mut egui::Ui,
    text: &str,
    fills: [Color32; 3],
    stroke: Stroke,
    text_color: Color32,
) -> egui::Response {
    // `scope` gives a child Ui with its own copy of the style, so these
    // changes only affect this one button.
    let scoped = ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        let states = [
            (&mut w.inactive, fills[0]),
            (&mut w.hovered, fills[1]),
            (&mut w.active, fills[2]),
        ];
        for (state, fill) in states {
            state.bg_fill = fill;
            state.weak_bg_fill = fill;
            state.bg_stroke = stroke;
            state.fg_stroke = Stroke::new(1.0, text_color);
        }
        return ui.button(text);
    });
    return scoped.inner;
}

/// Parses `#rrggbb` or `#rrggbbaa`. The alpha is not premultiplied, as in CSS.
pub fn parse_hex_color(value: &str) -> Option<Color32> {
    let hex = value.strip_prefix('#')?;
    if hex.len() != 6 && hex.len() != 8 {
        return None;
    }
    // Only hex digits: from_str_radix alone would accept "+1". This also
    // means every character is one byte, so the slicing below is safe.
    for c in hex.chars() {
        if !c.is_ascii_hexdigit() {
            return None;
        }
    }
    let r = u8::from_str_radix(&hex[0..2], 16).ok()?;
    let g = u8::from_str_radix(&hex[2..4], 16).ok()?;
    let b = u8::from_str_radix(&hex[4..6], 16).ok()?;
    let a = if hex.len() == 8 {
        u8::from_str_radix(&hex[6..8], 16).ok()?
    } else {
        255
    };
    return Some(Color32::from_rgba_unmultiplied(r, g, b, a));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_colors() {
        assert_eq!(
            parse_hex_color("#dcc496"),
            Some(Color32::from_rgb(0xdc, 0xc4, 0x96))
        );
        assert_eq!(
            parse_hex_color("#DCC496"),
            Some(Color32::from_rgb(0xdc, 0xc4, 0x96))
        );
        assert_eq!(
            parse_hex_color("#dcc4961f"),
            Some(Color32::from_rgba_unmultiplied(0xdc, 0xc4, 0x96, 0x1f))
        );
        assert_eq!(parse_hex_color("dcc496"), None);
        assert_eq!(parse_hex_color("#dcc49"), None);
        assert_eq!(parse_hex_color("#dcc4zz"), None);
        assert_eq!(parse_hex_color("#+1+2+3"), None);
        assert_eq!(parse_hex_color("#\u{e9}\u{e9}34"), None);
        assert_eq!(parse_hex_color(""), None);
    }

    #[test]
    fn key_value_lines() {
        let mut theme = Theme::default();
        theme.apply_text("# my theme\nprimary = #112233\n  on-primary=#445566  \n");
        assert_eq!(theme.primary, Color32::from_rgb(0x11, 0x22, 0x33));
        assert_eq!(theme.on_primary, Color32::from_rgb(0x44, 0x55, 0x66));
        // Untouched roles keep the defaults.
        assert_eq!(theme.secondary, Theme::default().secondary);
    }

    #[test]
    fn pasted_css_block() {
        let mut theme = Theme::default();
        theme.apply_text(
            "@theme static {\n    --color-c-surface: #ffffff;\n    --color-c-primary-hover-tint: #dcc4961f;\n}\n",
        );
        assert_eq!(theme.surface, Color32::WHITE);
        assert_eq!(
            theme.primary_hover_tint,
            Color32::from_rgba_unmultiplied(0xdc, 0xc4, 0x96, 0x1f)
        );
        assert!(!theme.is_dark());
    }

    #[test]
    fn junk_is_skipped() {
        let mut theme = Theme::default();
        theme.apply_text("nonsense\nno-such-role = #000000\nprimary = red\n= #000000\n:\n");
        assert_eq!(theme, Theme::default());
    }

    #[test]
    fn every_role_name_is_settable() {
        // Every role name must be accepted and change a color.
        let names = [
            "primary",
            "on-primary",
            "primary-container",
            "on-primary-container",
            "secondary",
            "on-secondary",
            "secondary-container",
            "on-secondary-container",
            "tertiary",
            "on-tertiary",
            "tertiary-container",
            "on-tertiary-container",
            "error",
            "on-error",
            "error-container",
            "on-error-container",
            "success",
            "on-success",
            "success-container",
            "on-success-container",
            "surface",
            "on-surface",
            "on-surface-variant",
            "surface-container-1",
            "surface-container-2",
            "surface-container-3",
            "surface-container-4",
            "surface-container-5",
            "surface-container-6",
            "surface-dim",
            "surface-bright",
            "outline",
            "outline-variant",
            "inverse-surface",
            "inverse-on-surface",
            "inverse-primary",
            "pink",
            "on-pink",
            "d-pink",
            "on-d-pink",
            "l-pink",
            "on-l-pink",
            "red",
            "on-red",
            "d-red",
            "on-d-red",
            "l-red",
            "on-l-red",
            "yellow",
            "on-yellow",
            "d-yellow",
            "on-d-yellow",
            "l-yellow",
            "on-l-yellow",
            "green",
            "on-green",
            "d-green",
            "on-d-green",
            "l-green",
            "on-l-green",
            "blue",
            "on-blue",
            "d-blue",
            "on-d-blue",
            "l-blue",
            "on-l-blue",
            "primary-hover",
            "primary-focus",
            "secondary-hover",
            "secondary-focus",
            "tertiary-hover",
            "tertiary-focus",
            "error-hover",
            "error-focus",
            "success-hover",
            "success-focus",
            "surface-container-1-hover",
            "surface-container-1-focus",
            "primary-hover-tint",
            "primary-focus-tint",
            "secondary-hover-tint",
            "secondary-focus-tint",
            "tertiary-hover-tint",
            "tertiary-focus-tint",
            "error-hover-tint",
            "error-focus-tint",
            "success-hover-tint",
            "success-focus-tint",
            "on-surface-hover-tint",
            "on-surface-focus-tint",
        ];
        for name in names {
            let mut theme = Theme::default();
            assert!(theme.set(name, Color32::from_rgb(1, 2, 3)), "{name}");
            assert_ne!(theme, Theme::default(), "{name} changed nothing");
        }
    }
}
