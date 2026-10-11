//! What the app remembers between runs (P4.10), and the optional features
//! from PLAN.md section 6, mapped to and from the config file's keys.

use itamiiru_core::config::Config;
use itamiiru_core::sort::{SortColumn, SortOrder};

use crate::toolbar::ViewMode;

/// Widths of the Details columns, in points. Size takes what's left.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ColumnWidths {
    pub name: f32,
    /// Only shown for search results.
    pub folder: f32,
    pub modified: f32,
    pub kind: f32,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub view: ViewMode,
    pub sort: SortOrder,
    pub show_hidden: bool,
    pub zoom: f32,
    pub sidebar_width: f32,
    /// The window's size when not maximized, in points at 100% zoom.
    pub window_size: [f32; 2],
    pub maximized: bool,
    pub columns: ColumnWidths,
    /// Optional features (PLAN.md section 6).
    pub auto_refresh_folder: bool,
    pub auto_refresh_drives: bool,
}

impl Default for Settings {
    fn default() -> Self {
        return Settings {
            view: ViewMode::Details,
            sort: SortOrder {
                column: SortColumn::Name,
                ascending: true,
            },
            show_hidden: false,
            zoom: 1.0,
            sidebar_width: 220.0,
            window_size: [900.0, 600.0],
            maximized: false,
            columns: ColumnWidths {
                name: 320.0,
                folder: 260.0,
                modified: 140.0,
                kind: 110.0,
            },
            auto_refresh_folder: true,
            auto_refresh_drives: true,
        };
    }
}

impl Settings {
    /// Reads settings, keeping defaults for anything missing or out of range.
    pub fn from_config(config: &Config) -> Settings {
        let d = Settings::default();
        let view = match config.get("view").and_then(ViewMode::from_key) {
            Some(view) => view,
            None => d.view,
        };
        let sort_column = match config.get("sort_column").and_then(sort_column_from_key) {
            Some(column) => column,
            None => d.sort.column,
        };
        return Settings {
            view,
            sort: SortOrder {
                column: sort_column,
                ascending: config.get_bool("sort_ascending", d.sort.ascending),
            },
            show_hidden: config.get_bool("show_hidden", d.show_hidden),
            zoom: clamp(config.get_f32("zoom", d.zoom), 0.2, 5.0),
            sidebar_width: clamp(
                config.get_f32("sidebar_width", d.sidebar_width),
                80.0,
                2000.0,
            ),
            window_size: [
                clamp(
                    config.get_f32("window_width", d.window_size[0]),
                    300.0,
                    20000.0,
                ),
                clamp(
                    config.get_f32("window_height", d.window_size[1]),
                    200.0,
                    20000.0,
                ),
            ],
            maximized: config.get_bool("window_maximized", d.maximized),
            columns: ColumnWidths {
                name: column_width(config, "column_name", d.columns.name),
                folder: column_width(config, "column_folder", d.columns.folder),
                modified: column_width(config, "column_modified", d.columns.modified),
                kind: column_width(config, "column_type", d.columns.kind),
            },
            auto_refresh_folder: config.get_bool("auto_refresh_folder", d.auto_refresh_folder),
            auto_refresh_drives: config.get_bool("auto_refresh_drives", d.auto_refresh_drives),
        };
    }

    /// Writes every setting into `config`, leaving its other lines alone.
    pub fn write_to(&self, config: &mut Config) {
        config.set("view", self.view.key());
        config.set("sort_column", sort_column_key(self.sort.column));
        config.set_bool("sort_ascending", self.sort.ascending);
        config.set_bool("show_hidden", self.show_hidden);
        config.set_f32("zoom", self.zoom);
        config.set_f32("sidebar_width", self.sidebar_width.round());
        config.set_f32("window_width", self.window_size[0].round());
        config.set_f32("window_height", self.window_size[1].round());
        config.set_bool("window_maximized", self.maximized);
        config.set_f32("column_name", self.columns.name.round());
        config.set_f32("column_folder", self.columns.folder.round());
        config.set_f32("column_modified", self.columns.modified.round());
        config.set_f32("column_type", self.columns.kind.round());
        config.set_bool("auto_refresh_folder", self.auto_refresh_folder);
        config.set_bool("auto_refresh_drives", self.auto_refresh_drives);
    }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    return value.clamp(min, max);
}

fn column_width(config: &Config, key: &str, default: f32) -> f32 {
    return clamp(config.get_f32(key, default), 40.0, 4000.0);
}

fn sort_column_key(column: SortColumn) -> &'static str {
    return match column {
        SortColumn::Name => "name",
        SortColumn::Modified => "modified",
        SortColumn::Type => "type",
        SortColumn::Size => "size",
    };
}

fn sort_column_from_key(key: &str) -> Option<SortColumn> {
    return match key {
        "name" => Some(SortColumn::Name),
        "modified" => Some(SortColumn::Modified),
        "type" => Some(SortColumn::Type),
        "size" => Some(SortColumn::Size),
        _ => None,
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut columns = Settings::default().columns;
        columns.name = 400.0;
        let settings = Settings {
            view: ViewMode::LargeIcons,
            sort: SortOrder {
                column: SortColumn::Size,
                ascending: false,
            },
            show_hidden: true,
            zoom: 1.25,
            window_size: [1280.0, 800.0],
            columns,
            auto_refresh_drives: false,
            // The rest keep their defaults.
            ..Settings::default()
        };

        let mut config = Config::parse("# keep me\n");
        settings.write_to(&mut config);
        let text = config.to_text();
        assert!(text.starts_with("# keep me\n"));
        assert_eq!(Settings::from_config(&Config::parse(&text)), settings);
    }

    #[test]
    fn bad_values_fall_back() {
        let config = Config::parse("view=huge\nsort_column=color\nzoom=1000\nsidebar_width=-5\n");
        let settings = Settings::from_config(&config);
        assert_eq!(settings.view, ViewMode::Details);
        assert_eq!(settings.sort.column, SortColumn::Name);
        assert_eq!(settings.zoom, 5.0);
        assert_eq!(settings.sidebar_width, 80.0);
        assert_eq!(
            Settings::from_config(&Config::default()),
            Settings::default()
        );
    }

    #[test]
    fn every_view_has_a_key() {
        for view in ViewMode::ALL {
            assert_eq!(ViewMode::from_key(view.key()), Some(view));
        }
    }
}
