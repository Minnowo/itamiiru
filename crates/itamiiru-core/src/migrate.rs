//! One-time move from the old config files to config.toml and live.toml.
//! The old `config` (`key=value` lines the app wrote) becomes live.toml;
//! the hand-written `actions` and `theme` files become config.toml's
//! `[[action]]` tables and `[colors]`. The old files are left alone.
//!
//! Delete this file and its call in the itamiiru crate's main.rs once
//! nobody has the old files any more (added 2026-11-08).

use std::path::Path;

use toml_edit::{Array, ArrayOfTables, DocumentMut, Item, Table, value};

use crate::config::{self, CONFIG_FILE, Config, LIVE_FILE, Value};

const OLD_CONFIG: &str = "config";
const OLD_ACTIONS: &str = "actions";
const OLD_THEME: &str = "theme";

const CONFIG_HEADER: &str = "\
# Moved here by itami from the old actions and theme files, which are
# no longer read. See data/config.example.toml for every setting.
";

/// Moves the old files' contents into the new ones, if there are old
/// files and neither new file exists yet. Problems are logged. Reads and
/// writes small files: call at startup only.
pub fn run(dir: &Path) {
    if dir.join(CONFIG_FILE).exists() || dir.join(LIVE_FILE).exists() {
        return;
    }
    let old_config = config::read_small(&dir.join(OLD_CONFIG));
    let old_actions = config::read_small(&dir.join(OLD_ACTIONS));
    let old_theme = config::read_small(&dir.join(OLD_THEME));
    if old_config.is_none() && old_actions.is_none() && old_theme.is_none() {
        return;
    }
    let (config_toml, live_toml) = convert(
        old_config.as_deref(),
        old_actions.as_deref(),
        old_theme.as_deref(),
    );
    // live.toml first: config.toml existing marks the move as done.
    if let Some(text) = live_toml
        && let Err(err) = config::write_atomic(&dir.join(LIVE_FILE), &text)
    {
        crate::debug!("migrate: can't write {LIVE_FILE}: {err}");
        return;
    }
    if let Err(err) = config::write_atomic(&dir.join(CONFIG_FILE), &config_toml) {
        crate::debug!("migrate: can't write {CONFIG_FILE}: {err}");
        return;
    }
    crate::debug!("migrate: moved the old config files into {}", dir.display());
}

/// The new config.toml text, and live.toml's if there was an old config.
pub fn convert(
    old_config: Option<&str>,
    old_actions: Option<&str>,
    old_theme: Option<&str>,
) -> (String, Option<String>) {
    let mut document = DocumentMut::new();
    if let Some(text) = old_theme {
        let colors = convert_theme(text);
        if !colors.is_empty() {
            document.insert("colors", Item::Table(colors));
        }
    }
    if let Some(text) = old_actions {
        let actions = convert_actions(text);
        if !actions.is_empty() {
            document.insert("action", Item::ArrayOfTables(actions));
        }
    }
    let config_toml = format!("{CONFIG_HEADER}\n{document}");
    let live_toml = old_config.map(convert_config);
    return (config_toml, live_toml);
}

/// `key=value` lines to live.toml. Lists, booleans and numbers get their
/// TOML types; `last_folder.<percent-encoded app id>` keys go into the
/// `[last_folder]` table.
fn convert_config(text: &str) -> String {
    let mut config = Config::default();
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('#') {
            continue;
        }
        let Some((key, raw)) = line.split_once('=') else {
            continue;
        };
        let (key, raw) = (key.trim(), raw.trim());
        if key.is_empty() {
            continue;
        }
        if let Some(encoded_id) = key.strip_prefix("last_folder.") {
            if let Ok(bytes) = crate::percent::decode(encoded_id)
                && let Ok(app_id) = String::from_utf8(bytes)
            {
                config.set_in("last_folder", &app_id, raw);
            }
            continue;
        }
        let converted = if key == "details_columns" || key == "status_fields" {
            let mut list = Vec::new();
            for part in raw.split(',') {
                if !part.trim().is_empty() {
                    list.push(part.trim().to_string());
                }
            }
            Value::List(list)
        } else if raw == "true" || raw == "false" {
            Value::Bool(raw == "true")
        } else if let Ok(number) = raw.parse::<f64>()
            && number.is_finite()
        {
            Value::Number(number)
        } else {
            Value::Text(raw.to_string())
        };
        config.set(key, &converted);
    }
    return config.live_text();
}

/// `[Name]` sections with `command = ...` and `for = a, b` lines.
fn convert_actions(text: &str) -> ArrayOfTables {
    let mut actions = ArrayOfTables::new();
    let mut current: Option<Table> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(table) = current.take() {
                actions.push(table);
            }
            let mut table = Table::new();
            table.insert("name", value(line[1..line.len() - 1].trim()));
            current = Some(table);
            continue;
        }
        let (Some(table), Some((key, raw))) = (&mut current, line.split_once('=')) else {
            continue;
        };
        let (key, raw) = (key.trim(), raw.trim());
        if key == "for" {
            let mut list = Array::new();
            for part in raw.split(',') {
                if !part.trim().is_empty() {
                    list.push(part.trim());
                }
            }
            table.insert(key, value(list));
        } else {
            table.insert(key, value(raw));
        }
    }
    if let Some(table) = current.take() {
        actions.push(table);
    }
    return actions;
}

/// `role = #hex` and ponkotsu CSS `--color-c-role: #hex;` lines.
fn convert_theme(text: &str) -> Table {
    let mut colors = Table::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with("/*") {
            continue;
        }
        let Some(sep) = line.find([':', '=']) else {
            continue;
        };
        let role = line[..sep].trim();
        let role = role.strip_prefix("--color-c-").unwrap_or(role);
        let color = line[sep + 1..].trim().trim_end_matches(';').trim();
        if role.is_empty() || color.is_empty() {
            continue;
        }
        colors.insert(role, value(color));
    }
    return colors;
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::actions;
    use crate::test_dir::TestDir;

    const OLD_CONFIG_TEXT: &str = "view=list\nzoom=1.25\nshow_hidden=true\n\
        details_columns=size,created\nstatus_fields=\ntheme=windows10\n\
        last_folder.org.mozilla.firefox=%2Fhome%2Fme\n";
    const OLD_ACTIONS_TEXT: &str = "# mine\n[Open in terminal]\n\
        command = xfce4-terminal --working-directory=%d\nfor = folders, background\n\
        [Convert]\ncommand = convert %f \"%f.png\"\nfor = *.jpg\n";
    const OLD_THEME_TEXT: &str = "primary = #112233\n@theme static {\n\
        --color-c-surface: #ffffff;\n}\n";

    #[test]
    fn converts_all_three() {
        let (config_toml, live_toml) = convert(
            Some(OLD_CONFIG_TEXT),
            Some(OLD_ACTIONS_TEXT),
            Some(OLD_THEME_TEXT),
        );
        let live_toml = live_toml.expect("there was an old config");
        let config = Config::parse(&config_toml, &live_toml);

        assert_eq!(config.get_str("view"), Some("list"));
        assert_eq!(config.get_f32("zoom", 0.0), 1.25);
        assert!(config.get_bool("show_hidden", false));
        assert_eq!(
            config.get_list("details_columns"),
            Some(vec!["size".to_string(), "created".to_string()])
        );
        assert_eq!(config.get_list("status_fields"), Some(Vec::new()));
        assert_eq!(config.get_str("theme"), Some("windows10"));
        assert_eq!(
            config.get_in("last_folder", "org.mozilla.firefox"),
            Some("%2Fhome%2Fme")
        );

        assert_eq!(
            config.base_strings("colors"),
            vec![
                ("primary".to_string(), "#112233".to_string()),
                ("surface".to_string(), "#ffffff".to_string()),
            ]
        );
        let (actions, problems) = actions::from_config(&config);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(actions.len(), 2);
        assert_eq!(actions[0].name, "Open in terminal");
        let item = actions::Item {
            path: Path::new("/p/a.jpg"),
            is_dir: false,
        };
        let lines = actions[1].command_lines(&[item], Path::new("/p"));
        assert_eq!(lines[0][2], "/p/a.jpg.png", "quotes survive");
    }

    #[test]
    fn runs_once_and_leaves_the_old_files() {
        let test_dir = TestDir::new("migrate");
        let dir = test_dir.path();
        let dir = dir.as_path();
        run(dir);
        assert!(!dir.join(CONFIG_FILE).exists(), "nothing to move");

        std::fs::write(dir.join(OLD_CONFIG), "view=list\n").unwrap();
        std::fs::write(dir.join(OLD_ACTIONS), OLD_ACTIONS_TEXT).unwrap();
        run(dir);
        let config = Config::load(dir);
        assert_eq!(config.get_str("view"), Some("list"));
        assert_eq!(config.base_tables("action").len(), 2);
        assert!(dir.join(OLD_CONFIG).exists());

        // Done once: later runs leave the new files alone.
        std::fs::write(dir.join(LIVE_FILE), "view = \"details\"\n").unwrap();
        std::fs::remove_file(dir.join(CONFIG_FILE)).unwrap();
        run(dir);
        assert!(!dir.join(CONFIG_FILE).exists());
        assert_eq!(Config::load(dir).get_str("view"), Some("details"));
    }
}
