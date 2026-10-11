//! The config file: `key=value` lines (PLAN.md section 6 Persistence).
//!
//! Lines that aren't `key=value` (comments, blanks) and keys we don't know
//! are kept as they are, so hand edits survive a save. Typed getters fall
//! back to a default when a value is missing or doesn't parse.

use std::io;
use std::path::{Path, PathBuf};

/// Config files bigger than this are ignored.
const MAX_CONFIG_SIZE: u64 = 64 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Line {
    Pair {
        key: String,
        value: String,
    },
    /// A comment, blank or unparseable line, kept as written.
    Other(String),
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Config {
    lines: Vec<Line>,
}

impl Config {
    pub fn parse(text: &str) -> Config {
        let mut lines = Vec::new();
        for raw in text.lines() {
            let trimmed = raw.trim();
            if trimmed.starts_with('#') {
                lines.push(Line::Other(raw.to_string()));
                continue;
            }
            match trimmed.split_once('=') {
                Some((key, value)) if !key.trim().is_empty() => lines.push(Line::Pair {
                    key: key.trim().to_string(),
                    value: value.trim().to_string(),
                }),
                _ => lines.push(Line::Other(raw.to_string())),
            }
        }
        return Config { lines };
    }

    pub fn to_text(&self) -> String {
        let mut text = String::new();
        for line in &self.lines {
            match line {
                Line::Pair { key, value } => {
                    text.push_str(key);
                    text.push('=');
                    text.push_str(value);
                }
                Line::Other(raw) => text.push_str(raw),
            }
            text.push('\n');
        }
        return text;
    }

    /// The value of `key`; the last one wins if it's there twice.
    pub fn get(&self, key: &str) -> Option<&str> {
        let mut found = None;
        for line in &self.lines {
            if let Line::Pair { key: k, value } = line
                && k == key
            {
                found = Some(value.as_str());
            }
        }
        return found;
    }

    /// `true`/`false`, `1`/`0`, `on`/`off`, `yes`/`no`.
    pub fn get_bool(&self, key: &str, default: bool) -> bool {
        let Some(value) = self.get(key) else {
            return default;
        };
        return match value.to_ascii_lowercase().as_str() {
            "true" | "1" | "on" | "yes" => true,
            "false" | "0" | "off" | "no" => false,
            _ => default,
        };
    }

    /// A finite number, or `default`.
    pub fn get_f32(&self, key: &str, default: f32) -> f32 {
        let Some(value) = self.get(key) else {
            return default;
        };
        return match value.parse::<f32>() {
            Ok(number) if number.is_finite() => number,
            _ => default,
        };
    }

    /// Sets `key`, replacing its value in place, or adding it at the end.
    pub fn set(&mut self, key: &str, value: &str) {
        // Newlines would break the file format.
        let value = value.replace(['\n', '\r'], " ");
        for line in self.lines.iter_mut() {
            if let Line::Pair { key: k, value: v } = line
                && k == key
            {
                *v = value;
                return;
            }
        }
        self.lines.push(Line::Pair {
            key: key.to_string(),
            value,
        });
    }

    pub fn set_bool(&mut self, key: &str, value: bool) {
        self.set(key, if value { "true" } else { "false" });
    }

    pub fn set_f32(&mut self, key: &str, value: f32) {
        // Whole points are plenty for sizes and widths; zoom keeps two decimals.
        self.set(key, &format!("{}", (value * 100.0).round() / 100.0));
    }

    /// Reads a config file. Missing, unreadable or oversized files give an
    /// empty config (everything at its default).
    pub fn load(path: &Path) -> Config {
        let size = match std::fs::metadata(path) {
            Ok(meta) => meta.len(),
            Err(_) => return Config::default(),
        };
        if size > MAX_CONFIG_SIZE {
            crate::debug!("config: {} is too big, ignoring it", path.display());
            return Config::default();
        }
        return match std::fs::read_to_string(path) {
            Ok(text) => Config::parse(&text),
            Err(err) => {
                crate::debug!("config: can't read {}: {err}", path.display());
                Config::default()
            }
        };
    }

    /// Writes the file atomically: a temporary file next to it, then a
    /// rename, so a crash never leaves half a config.
    pub fn save(&self, path: &Path) -> io::Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let mut temp_name = path.as_os_str().to_os_string();
        temp_name.push(".tmp");
        let temp = PathBuf::from(temp_name);
        std::fs::write(&temp, self.to_text())?;
        return std::fs::rename(&temp, path);
    }
}

/// `<config dir>/config`.
pub fn default_path() -> Option<PathBuf> {
    return Some(crate::platform::config_dir()?.join("config"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_dir::TestDir;

    #[test]
    fn keeps_comments_and_unknown_keys() {
        let text = "# my settings\nview=list\n\nfuture_key = something\nnot a pair\n";
        let mut config = Config::parse(text);
        assert_eq!(config.get("view"), Some("list"));
        assert_eq!(config.get("future_key"), Some("something"));
        config.set("view", "details");
        config.set("zoom", "1.5");
        assert_eq!(
            config.to_text(),
            "# my settings\nview=details\n\nfuture_key=something\nnot a pair\nzoom=1.5\n"
        );
    }

    #[test]
    fn typed_values() {
        let config = Config::parse("a=true\nb=OFF\nc=maybe\nx=1.25\ny=NaN\nz=abc\nx=2\n");
        assert!(config.get_bool("a", false));
        assert!(!config.get_bool("b", true));
        assert!(config.get_bool("c", true), "bad values give the default");
        assert!(!config.get_bool("missing", false));
        assert_eq!(config.get_f32("x", 0.0), 2.0, "the last one wins");
        assert_eq!(config.get_f32("y", 7.0), 7.0, "NaN is rejected");
        assert_eq!(config.get_f32("z", 7.0), 7.0);
    }

    #[test]
    fn set_rounds_and_strips_newlines() {
        let mut config = Config::default();
        config.set_f32("w", 123.4567);
        config.set_bool("b", true);
        config.set("s", "two\nlines");
        assert_eq!(config.to_text(), "w=123.46\nb=true\ns=two lines\n");
    }

    #[test]
    fn save_and_load() {
        let dir = TestDir::new("config");
        let path = dir.path().join("sub").join("config");
        assert_eq!(Config::load(&path), Config::default(), "missing file");
        let mut config = Config::default();
        config.set("view", "list");
        config.save(&path).unwrap();
        assert_eq!(Config::load(&path).get("view"), Some("list"));
        assert!(!dir.path().join("sub").join("config.tmp").exists());
    }
}
