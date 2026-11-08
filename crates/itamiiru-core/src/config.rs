//! The config files: `config.toml`, written by hand and only ever read,
//! and `live.toml` next to it, where the app keeps what was changed while
//! it ran (zoom, window size, the settings page). A setting in
//! `live.toml` wins over the same one in `config.toml`.
//!
//! Comments, ordering and keys we don't know survive a save of
//! `live.toml`. Typed getters fall back to a default when a value is
//! missing or has the wrong type.

use std::io;
use std::path::{Path, PathBuf};

use toml_edit::{DocumentMut, Item, Table};

/// Config files bigger than this are ignored.
const MAX_CONFIG_SIZE: u64 = 64 * 1024;

pub const CONFIG_FILE: &str = "config.toml";
pub const LIVE_FILE: &str = "live.toml";

/// Put at the top of a new `live.toml`.
const LIVE_HEADER: &str = "\
# Written by itami: the settings changed in the app, on top of config.toml.
# Delete a line to go back to the value in config.toml (or the default).
";

#[derive(Clone, Debug, Default)]
pub struct Config {
    /// `config.toml`.
    base: DocumentMut,
    /// `live.toml`.
    live: DocumentMut,
    /// Why a file couldn't be used (not valid TOML), for the settings page.
    problems: Vec<String>,
}

/// A setting's value, as `Config::set` writes it.
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Bool(bool),
    Number(f64),
    Text(String),
    List(Vec<String>),
}

impl Config {
    /// Parses both files' text. A file that isn't valid TOML counts as
    /// empty; the error is logged.
    pub fn parse(base_text: &str, live_text: &str) -> Config {
        let mut problems = Vec::new();
        return Config {
            base: parse_document(CONFIG_FILE, base_text, &mut problems),
            live: parse_document(LIVE_FILE, live_text, &mut problems),
            problems,
        };
    }

    /// Files that couldn't be used, and why.
    pub fn problems(&self) -> &[String] {
        return &self.problems;
    }

    /// Reads `config.toml` and `live.toml` from `dir`. Missing, unreadable,
    /// oversized or broken files count as empty. Reads from disk: call at
    /// startup only.
    pub fn load(dir: &Path) -> Config {
        let base = read_small(&dir.join(CONFIG_FILE)).unwrap_or_default();
        let live = read_small(&dir.join(LIVE_FILE)).unwrap_or_default();
        return Config::parse(&base, &live);
    }

    /// Reads one file as `config.toml`, with nothing on top.
    pub fn load_file(path: &Path) -> Config {
        let text = read_small(path).unwrap_or_default();
        return Config::parse(&text, "");
    }

    /// The same `config.toml`, without what `live.toml` changed.
    pub fn base_only(&self) -> Config {
        return Config {
            base: self.base.clone(),
            live: DocumentMut::new(),
            problems: Vec::new(),
        };
    }

    /// A top-level string.
    pub fn get_str(&self, key: &str) -> Option<&str> {
        return self.get(key)?.as_str();
    }

    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        return match self.get(key).and_then(Item::as_bool) {
            Some(value) => value,
            None => default,
        };
    }

    /// A finite number (`1` and `1.0` both work), or `default`.
    pub fn get_f32(&self, key: &str, default: f32) -> f32 {
        let Some(item) = self.get(key) else {
            return default;
        };
        let number = if let Some(float) = item.as_float() {
            float as f32
        } else if let Some(integer) = item.as_integer() {
            integer as f32
        } else {
            return default;
        };
        if !number.is_finite() {
            return default;
        }
        return number;
    }

    /// An array's strings; anything in it that isn't a string is skipped.
    /// None if the key is missing or isn't an array.
    pub fn get_list(&self, key: &str) -> Option<Vec<String>> {
        let array = self.get(key)?.as_array()?;
        let mut list = Vec::new();
        for value in array.iter() {
            if let Some(text) = value.as_str() {
                list.push(text.to_string());
            }
        }
        return Some(list);
    }

    /// A string in a table, e.g. `[last_folder]`.
    pub fn get_in(&self, table: &str, key: &str) -> Option<&str> {
        if let Some(value) = string_in(&self.live, table, key) {
            return Some(value);
        }
        return string_in(&self.base, table, key);
    }

    /// The string pairs of a table in `config.toml` only, in file order,
    /// e.g. `[colors]`. Pairs whose value isn't a string are logged and
    /// skipped.
    pub fn base_strings(&self, table: &str) -> Vec<(String, String)> {
        let mut pairs = Vec::new();
        let Some(table_like) = self.base.get(table).and_then(Item::as_table_like) else {
            return pairs;
        };
        for (key, item) in table_like.iter() {
            match item.as_str() {
                Some(value) => pairs.push((key.to_string(), value.to_string())),
                None => crate::debug!("config: [{table}] {key} is not a string"),
            }
        }
        return pairs;
    }

    /// The `[name.<key>]` tables in `config.toml`, as (key, table) in
    /// file order, e.g. the actions. Entries that aren't tables are
    /// returned as problems.
    pub fn base_subtables(&self, name: &str) -> (Vec<(String, &Table)>, Vec<String>) {
        let mut tables = Vec::new();
        let mut problems = Vec::new();
        let Some(item) = self.base.get(name) else {
            return (tables, problems);
        };
        let Some(parent) = item.as_table() else {
            problems.push(format!("{name} must be a table like [{name}.some_id]"));
            return (tables, problems);
        };
        for (key, item) in parent.iter() {
            match item.as_table() {
                Some(table) => tables.push((key.to_string(), table)),
                None => problems.push(format!("{name}.{key} must be a table")),
            }
        }
        return (tables, problems);
    }

    /// A value in a table of `config.toml`, e.g. `[menu]` `item`.
    pub fn base_value_in(&self, table: &str, key: &str) -> Option<&toml_edit::Value> {
        return self.base.get(table)?.as_table_like()?.get(key)?.as_value();
    }

    /// The keys of a table in `config.toml`, in file order.
    pub fn base_keys_in(&self, table: &str) -> Vec<String> {
        let mut keys = Vec::new();
        if let Some(table_like) = self.base.get(table).and_then(Item::as_table_like) {
            for (key, _) in table_like.iter() {
                keys.push(key.to_string());
            }
        }
        return keys;
    }

    /// Sets a top-level key in `live.toml`, in place if it's there.
    pub fn set(&mut self, key: &str, value: &Value) {
        let new = toml_value(value);
        // Assigning to the value (not the item) keeps the line's comments.
        if let Some(item) = self.live.get_mut(key)
            && let Some(old) = item.as_value_mut()
        {
            let decor = old.decor().clone();
            *old = new;
            *old.decor_mut() = decor;
            return;
        }
        self.live.insert(key, Item::Value(new));
    }

    /// Removes a top-level key from `live.toml`, so `config.toml` decides.
    pub fn unset(&mut self, key: &str) {
        self.live.remove(key);
    }

    /// Sets a string in a table of `live.toml`, making the table if needed.
    pub fn set_in(&mut self, table: &str, key: &str, value: &str) {
        if self.live.get(table).and_then(Item::as_table).is_none() {
            self.live.insert(table, Item::Table(Table::new()));
        }
        let Some(table) = self.live.get_mut(table).and_then(Item::as_table_mut) else {
            return;
        };
        table.insert(key, toml_edit::value(value));
    }

    /// What `live.toml` holds, as text.
    pub fn live_text(&self) -> String {
        let text = self.live.to_string();
        if text.starts_with('#') {
            return text;
        }
        return format!("{LIVE_HEADER}\n{text}");
    }

    /// Writes `live.toml` in `dir` atomically: a temporary file next to
    /// it, then a rename, so a crash never leaves half a file.
    pub fn save_live(&self, dir: &Path) -> io::Result<()> {
        return write_atomic(&dir.join(LIVE_FILE), &self.live_text());
    }

    /// A top-level value: `live.toml`'s, else `config.toml`'s.
    fn get(&self, key: &str) -> Option<&Item> {
        if let Some(item) = self.live.get(key)
            && item.is_value()
        {
            return Some(item);
        }
        return self.base.get(key).filter(|item| item.is_value());
    }
}

/// The folder the config files are in.
pub fn default_dir() -> Option<PathBuf> {
    return crate::platform::config_dir();
}

/// Writes `text` to `path` through a temporary file and a rename.
pub fn write_atomic(path: &Path, text: &str) -> io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let mut temp_name = path.as_os_str().to_os_string();
    temp_name.push(".tmp");
    let temp = PathBuf::from(temp_name);
    std::fs::write(&temp, text)?;
    return std::fs::rename(&temp, path);
}

/// Reads a small text file. None if it's missing, too big or unreadable;
/// all but missing are logged.
pub fn read_small(path: &Path) -> Option<String> {
    let size = match std::fs::metadata(path) {
        Ok(meta) => meta.len(),
        Err(_) => return None,
    };
    if size > MAX_CONFIG_SIZE {
        crate::debug!("config: {} is too big, ignoring it", path.display());
        return None;
    }
    return match std::fs::read_to_string(path) {
        Ok(text) => Some(text),
        Err(err) => {
            crate::debug!("config: can't read {}: {err}", path.display());
            None
        }
    };
}

fn parse_document(name: &str, text: &str, problems: &mut Vec<String>) -> DocumentMut {
    return match text.parse::<DocumentMut>() {
        Ok(document) => document,
        Err(err) => {
            crate::debug!("config: {name} is not valid TOML, ignoring it: {err}");
            problems.push(format!(
                "{name} is not valid TOML, so none of it is used: {err}"
            ));
            DocumentMut::new()
        }
    };
}

fn string_in<'a>(document: &'a DocumentMut, table: &str, key: &str) -> Option<&'a str> {
    return document.get(table)?.as_table_like()?.get(key)?.as_str();
}

fn toml_value(value: &Value) -> toml_edit::Value {
    return match value {
        Value::Bool(flag) => toml_edit::Value::from(*flag),
        Value::Number(number) => toml_edit::Value::from(*number),
        Value::Text(text) => toml_edit::Value::from(text.as_str()),
        Value::List(list) => {
            let mut array = toml_edit::Array::new();
            for text in list {
                array.push(text.as_str());
            }
            toml_edit::Value::Array(array)
        }
    };
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn live_wins_over_base() {
        let config = Config::parse(
            "view = \"list\"\nzoom = 1.5\n[last_folder]\na = \"/base\"\n",
            "zoom = 2\n[last_folder]\nb = \"/live\"\n",
        );
        assert_eq!(config.get_str("view"), Some("list"));
        assert_eq!(config.get_f32("zoom", 0.0), 2.0, "integers work too");
        assert_eq!(config.get_in("last_folder", "a"), Some("/base"));
        assert_eq!(config.get_in("last_folder", "b"), Some("/live"));
        assert_eq!(config.base_only().get_f32("zoom", 0.0), 1.5);
    }

    #[test]
    fn typed_values() {
        let config = Config::parse(
            "a = true\nb = \"yes\"\nx = nan\nz = \"abc\"\nl = [\"one\", 2, \"three\"]\n",
            "",
        );
        assert!(config.get_bool("a", false));
        assert!(!config.get_bool("b", false), "strings aren't booleans");
        assert!(!config.get_bool("missing", false));
        assert_eq!(config.get_f32("x", 7.0), 7.0, "NaN is rejected");
        assert_eq!(config.get_f32("z", 7.0), 7.0);
        assert_eq!(
            config.get_list("l"),
            Some(vec!["one".into(), "three".into()])
        );
        assert_eq!(config.get_list("z"), None);
        assert_eq!(config.get_str("l"), None);
    }

    #[test]
    fn broken_files_count_as_empty() {
        let config = Config::parse("view = \"list", "zoom = [");
        assert_eq!(config.get_str("view"), None);
        assert_eq!(config.get_f32("zoom", 1.0), 1.0);
        assert_eq!(config.problems().len(), 2, "{:?}", config.problems());
        assert!(config.problems()[0].starts_with("config.toml is not valid TOML"));
        assert!(Config::parse("", "").problems().is_empty());
    }

    #[test]
    fn saving_keeps_comments_and_unknown_keys() {
        let mut config = Config::parse(
            "view = \"list\"\n",
            "# mine\nzoom = 1.5 # not too big\nfuture = \"x\"\n",
        );
        config.set("zoom", &Value::Number(2.0));
        config.set("view", &Value::Text("details".into()));
        config.set("cols", &Value::List(vec!["a".into(), "b".into()]));
        config.unset("future");
        config.set_in("last_folder", "org.example \"odd\"", "/x");
        assert_eq!(
            config.live_text(),
            "# mine\nzoom = 2.0 # not too big\nview = \"details\"\ncols = [\"a\", \"b\"]\n\
             \n[last_folder]\n'org.example \"odd\"' = \"/x\"\n"
        );
        let again = Config::parse("", &config.live_text());
        assert_eq!(
            again.get_in("last_folder", "org.example \"odd\""),
            Some("/x")
        );
    }

    #[test]
    fn base_tables_and_strings() {
        let config = Config::parse(
            "[colors]\nsurface = \"#000000\"\nbad = 3\n\n[action.a]\nname = \"A\"\n\n[action.b]\nname = \"B\"\n\n[menu]\nitem = [\"cut\"]\n",
            "[colors]\nsurface = \"#ffffff\"\n",
        );
        assert_eq!(
            config.base_strings("colors"),
            vec![("surface".to_string(), "#000000".to_string())],
            "only config.toml's, and only strings"
        );
        let (tables, problems) = config.base_subtables("action");
        assert!(problems.is_empty());
        let mut names = Vec::new();
        for (key, table) in &tables {
            names.push((key.as_str(), table.get("name").and_then(Item::as_str)));
        }
        assert_eq!(names, [("a", Some("A")), ("b", Some("B"))]);
        assert_eq!(config.base_keys_in("menu"), ["item"]);
        assert!(config.base_value_in("menu", "item").is_some());
        assert!(config.base_value_in("menu", "drive").is_none());
        let flat = Config::parse("action = 1\n[menu]\n", "");
        assert_eq!(flat.base_subtables("action").1.len(), 1);
    }

    #[test]
    fn save_and_load() {
        let dir = TestDir::new("config");
        let sub = dir.path().join("sub");
        let empty = Config::load(&sub);
        assert_eq!(empty.get_str("view"), None, "missing files");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join(CONFIG_FILE), "view = \"list\"\nzoom = 3\n").unwrap();
        let mut config = Config::load(&sub);
        config.set("zoom", &Value::Number(1.25));
        config.save_live(&sub).unwrap();
        let loaded = Config::load(&sub);
        assert_eq!(loaded.get_str("view"), Some("list"));
        assert_eq!(loaded.get_f32("zoom", 0.0), 1.25);
        assert!(loaded.live_text().starts_with(LIVE_HEADER));
        assert_eq!(
            std::fs::read_to_string(sub.join(CONFIG_FILE)).unwrap(),
            "view = \"list\"\nzoom = 3\n",
            "config.toml is never written"
        );
        assert!(!sub.join("live.toml.tmp").exists());
    }
}
