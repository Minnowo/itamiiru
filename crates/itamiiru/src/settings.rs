//! What the app remembers between runs, and the optional features,
//! mapped to and from the config file's keys.

use itamiiru_core::config::Config;
use itamiiru_core::sort::{SortColumn, SortOrder};

use crate::toolbar::ViewMode;

/// A Details view column besides Name (always first) and Folder (search
/// results only). `details_columns=modified,type,size` lists the shown
/// ones in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DetailColumn {
    Modified,
    Created,
    Type,
    Size,
}

impl DetailColumn {
    pub const ALL: [DetailColumn; 4] = [
        DetailColumn::Modified,
        DetailColumn::Created,
        DetailColumn::Type,
        DetailColumn::Size,
    ];

    /// The name used in the config file.
    pub fn key(self) -> &'static str {
        return match self {
            DetailColumn::Modified => "modified",
            DetailColumn::Created => "created",
            DetailColumn::Type => "type",
            DetailColumn::Size => "size",
        };
    }

    pub fn from_key(key: &str) -> Option<DetailColumn> {
        for column in DetailColumn::ALL {
            if column.key() == key {
                return Some(column);
            }
        }
        return None;
    }

    /// The header, and the name on the settings page.
    pub fn label(self) -> &'static str {
        return match self {
            DetailColumn::Modified => "Date modified",
            DetailColumn::Created => "Date created",
            DetailColumn::Type => "Type",
            DetailColumn::Size => "Size",
        };
    }

    pub fn sort_column(self) -> SortColumn {
        return match self {
            DetailColumn::Modified => SortColumn::Modified,
            DetailColumn::Created => SortColumn::Created,
            DetailColumn::Type => SortColumn::Type,
            DetailColumn::Size => SortColumn::Size,
        };
    }
}

/// Reads a comma-separated column list. Unknown names and repeats are
/// skipped.
fn parse_columns(value: &str) -> Vec<DetailColumn> {
    let mut columns = Vec::new();
    for part in value.split(',') {
        if let Some(column) = DetailColumn::from_key(part.trim())
            && !columns.contains(&column)
        {
            columns.push(column);
        }
    }
    return columns;
}

fn columns_value(columns: &[DetailColumn]) -> String {
    let mut keys = Vec::new();
    for column in columns {
        keys.push(column.key());
    }
    return keys.join(",");
}

/// What the status bar says about a single selected item, in the order
/// listed in `status_fields` (e.g. `status_fields=type,size,modified`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StatusField {
    Name,
    Type,
    /// Files only; a folder's size isn't known without walking it.
    Size,
    Modified,
    Created,
    /// The full path.
    Path,
}

impl StatusField {
    pub const ALL: [StatusField; 6] = [
        StatusField::Name,
        StatusField::Type,
        StatusField::Size,
        StatusField::Modified,
        StatusField::Created,
        StatusField::Path,
    ];

    /// The name used in the config file.
    pub fn key(self) -> &'static str {
        return match self {
            StatusField::Name => "name",
            StatusField::Type => "type",
            StatusField::Size => "size",
            StatusField::Modified => "modified",
            StatusField::Created => "created",
            StatusField::Path => "path",
        };
    }

    pub fn from_key(key: &str) -> Option<StatusField> {
        for field in StatusField::ALL {
            if field.key() == key {
                return Some(field);
            }
        }
        return None;
    }
}

/// Reads a comma-separated field list. Unknown names and repeats are
/// skipped; an empty value means no details, just "1 selected".
fn parse_status_fields(value: &str) -> Vec<StatusField> {
    let mut fields = Vec::new();
    for part in value.split(',') {
        if let Some(field) = StatusField::from_key(part.trim())
            && !fields.contains(&field)
        {
            fields.push(field);
        }
    }
    return fields;
}

fn status_fields_value(fields: &[StatusField]) -> String {
    let mut keys = Vec::new();
    for field in fields {
        keys.push(field.key());
    }
    return keys.join(",");
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
    /// The Details view's columns after Name, in order.
    pub columns: Vec<DetailColumn>,
    /// Optional features.
    pub auto_refresh_folder: bool,
    pub auto_refresh_drives: bool,
    /// Image previews in the icon views.
    pub thumbnails: bool,
    /// The preview pane on the right, and its width in points.
    pub preview_pane: bool,
    /// The preview pane only shows when there's a picture or text to show.
    pub preview_auto_hide: bool,
    pub preview_width: f32,
    /// The details the status bar shows for one selected item.
    pub status_fields: Vec<StatusField>,
    /// Delete moves to the trash; off, it deletes permanently (after asking).
    pub use_trash: bool,
    /// Files can be dragged out to other apps.
    pub drag_and_drop: bool,
    /// Dragging from empty space selects with a box.
    pub drag_select: bool,
    /// Files can be dragged onto folders and between panes.
    pub drag_within: bool,
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
            columns: vec![
                DetailColumn::Modified,
                DetailColumn::Type,
                DetailColumn::Size,
            ],
            auto_refresh_folder: true,
            auto_refresh_drives: true,
            thumbnails: true,
            preview_pane: false,
            preview_auto_hide: false,
            preview_width: 300.0,
            status_fields: vec![StatusField::Type, StatusField::Size, StatusField::Modified],
            use_trash: true,
            drag_and_drop: true,
            drag_select: true,
            drag_within: true,
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
            columns: match config.get("details_columns") {
                Some(value) => parse_columns(value),
                None => d.columns,
            },
            auto_refresh_folder: config.get_bool("auto_refresh_folder", d.auto_refresh_folder),
            auto_refresh_drives: config.get_bool("auto_refresh_drives", d.auto_refresh_drives),
            thumbnails: config.get_bool("thumbnails", d.thumbnails),
            preview_pane: config.get_bool("preview_pane", d.preview_pane),
            preview_auto_hide: config.get_bool("preview_auto_hide", d.preview_auto_hide),
            preview_width: clamp(
                config.get_f32("preview_width", d.preview_width),
                120.0,
                4000.0,
            ),
            status_fields: match config.get("status_fields") {
                Some(value) => parse_status_fields(value),
                None => d.status_fields,
            },
            use_trash: config.get_bool("use_trash", d.use_trash),
            drag_and_drop: config.get_bool("drag_and_drop", d.drag_and_drop),
            drag_select: config.get_bool("drag_select", d.drag_select),
            drag_within: config.get_bool("drag_within", d.drag_within),
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
        config.set("details_columns", &columns_value(&self.columns));
        config.set_bool("use_trash", self.use_trash);
        config.set_bool("drag_and_drop", self.drag_and_drop);
        config.set_bool("drag_select", self.drag_select);
        config.set_bool("drag_within", self.drag_within);
        config.set_bool("auto_refresh_folder", self.auto_refresh_folder);
        config.set_bool("auto_refresh_drives", self.auto_refresh_drives);
        config.set_bool("thumbnails", self.thumbnails);
        config.set_bool("preview_pane", self.preview_pane);
        config.set_bool("preview_auto_hide", self.preview_auto_hide);
        config.set_f32("preview_width", self.preview_width.round());
        config.set("status_fields", &status_fields_value(&self.status_fields));
    }
}

fn clamp(value: f32, min: f32, max: f32) -> f32 {
    return value.clamp(min, max);
}

fn sort_column_key(column: SortColumn) -> &'static str {
    return match column {
        SortColumn::Name => "name",
        SortColumn::Modified => "modified",
        SortColumn::Created => "created",
        SortColumn::Type => "type",
        SortColumn::Size => "size",
    };
}

fn sort_column_from_key(key: &str) -> Option<SortColumn> {
    return match key {
        "name" => Some(SortColumn::Name),
        "modified" => Some(SortColumn::Modified),
        "created" => Some(SortColumn::Created),
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
        let columns = vec![DetailColumn::Size, DetailColumn::Created];
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
            thumbnails: false,
            preview_pane: true,
            preview_auto_hide: true,
            preview_width: 420.0,
            status_fields: vec![StatusField::Path, StatusField::Name],
            use_trash: false,
            drag_and_drop: false,
            drag_select: false,
            drag_within: false,
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
    fn status_fields() {
        assert_eq!(
            parse_status_fields(" name , size,bogus,name,path"),
            vec![StatusField::Name, StatusField::Size, StatusField::Path]
        );
        assert_eq!(parse_status_fields(""), Vec::new());
        // An empty value is kept, not replaced by the default.
        let settings = Settings::from_config(&Config::parse("status_fields=\n"));
        assert!(settings.status_fields.is_empty());
        for field in StatusField::ALL {
            assert_eq!(StatusField::from_key(field.key()), Some(field));
        }
    }

    #[test]
    fn details_columns() {
        // In the order given; unknown names and repeats are skipped.
        let columns = parse_columns("size, bogus,created,name,size");
        assert_eq!(columns, [DetailColumn::Size, DetailColumn::Created]);
        assert_eq!(columns_value(&columns), "size,created");
        assert!(parse_columns("").is_empty());
        assert_eq!(
            columns_value(&Settings::default().columns),
            "modified,type,size"
        );
        for column in DetailColumn::ALL {
            assert_eq!(DetailColumn::from_key(column.key()), Some(column));
        }
    }

    #[test]
    fn every_view_has_a_key() {
        for view in ViewMode::ALL {
            assert_eq!(ViewMode::from_key(view.key()), Some(view));
        }
    }
}
