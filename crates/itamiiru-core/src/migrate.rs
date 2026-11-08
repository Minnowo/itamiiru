//! One-time move from the old config files to config.toml and live.toml.
//! The old `config` (`key=value` lines the app wrote) becomes live.toml;
//! the hand-written `actions` and `theme` files become config.toml's
//! `[action.<id>]` tables (placed in the item and background menus where
//! custom actions used to show) and `[colors]`. The old files are left
//! alone.
//!
//! Delete this file and its call in the itamiiru crate's main.rs once
//! nobody has the old files any more (added 2026-11-08).

use std::path::Path;

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, value};

use crate::config::{self, CONFIG_FILE, Config, LIVE_FILE, Value};
use crate::menu::{self, BuiltIn};

const OLD_CONFIG: &str = "config";
const OLD_ACTIONS: &str = "actions";
const OLD_THEME: &str = "theme";

const CONFIG_HEADER: &str = "\
# Moved here by itami from the old actions and theme files, which are
# no longer read. See data/default.toml for every setting and menu.
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
        convert_actions(text, &mut document);
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

/// An action from the old file: `[Name]`, `command = ...`, `for = a, b`.
struct OldAction {
    name: String,
    command: Option<String>,
    targets: Option<Vec<String>>,
}

/// Adds the old file's actions as `[action.<id>]` tables, and puts them
/// in the item menu (after `pin`) and, those that were `for` background,
/// the background menu (after `pin`, past a separator), as they showed
/// before.
fn convert_actions(text: &str, document: &mut DocumentMut) {
    let old = parse_old_actions(text);
    let mut tables = Table::new();
    tables.set_implicit(true);
    let mut on_items = Vec::new();
    let mut on_background = Vec::new();
    let mut used: Vec<String> = Vec::new();
    for action in old {
        let id = unique_id(&action.name, &used);
        used.push(id.clone());
        let mut table = Table::new();
        table.insert("name", value(action.name.as_str()));
        if let Some(command) = &action.command {
            table.insert("command", value(command.as_str()));
        }
        // "background" is now the background menu, not a target.
        let mut background = false;
        let mut rest = Vec::new();
        if let Some(targets) = &action.targets {
            for target in targets {
                if target == "background" {
                    background = true;
                } else {
                    rest.push(target.clone());
                }
            }
        }
        if !rest.is_empty() {
            table.insert("for", value(string_array(&rest)));
        }
        if action.targets.is_none() || !rest.is_empty() {
            on_items.push(toml_edit::Value::from(id.as_str()));
        }
        if background {
            // The folder on screen is what it's for now; keep it showing
            // there even when its other targets are files or patterns.
            if rest.is_empty() || rest.iter().any(|target| target == "folders") {
                on_background.push(toml_edit::Value::from(id.as_str()));
            } else {
                let mut entry = InlineTable::new();
                entry.insert("id", id.as_str().into());
                entry.insert("for", string_array(&["folders".to_string()]).into());
                on_background.push(toml_edit::Value::InlineTable(entry));
            }
        }
        tables.insert(&id, Item::Table(table));
    }
    if tables.is_empty() {
        return;
    }
    document.insert("action", Item::Table(tables));
    let mut menus = Table::new();
    if !on_items.is_empty() {
        menus.insert("item", value(default_menu_with("item", on_items, false)));
    }
    if !on_background.is_empty() {
        menus.insert(
            "background",
            value(default_menu_with("background", on_background, true)),
        );
    }
    document.insert("menu", Item::Table(menus));
}

fn parse_old_actions(text: &str) -> Vec<OldAction> {
    let mut actions = Vec::new();
    let mut current: Option<OldAction> = None;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            if let Some(action) = current.take() {
                actions.push(action);
            }
            current = Some(OldAction {
                name: line[1..line.len() - 1].trim().to_string(),
                command: None,
                targets: None,
            });
            continue;
        }
        let (Some(action), Some((key, raw))) = (&mut current, line.split_once('=')) else {
            continue;
        };
        let raw = raw.trim();
        match key.trim() {
            "command" => action.command = Some(raw.to_string()),
            "for" => {
                let mut targets = Vec::new();
                for part in raw.split(',') {
                    if !part.trim().is_empty() {
                        targets.push(part.trim().to_string());
                    }
                }
                action.targets = Some(targets);
            }
            _ => {}
        }
    }
    if let Some(action) = current.take() {
        actions.push(action);
    }
    return actions;
}

/// An id from a name: lowercase letters, digits and `_`, not a built-in's
/// id and not one in `used` (`_2`, `_3`, ... are added).
fn unique_id(name: &str, used: &[String]) -> String {
    let mut base = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() {
            base.push(c.to_ascii_lowercase());
        } else if !base.is_empty() && !base.ends_with('_') {
            base.push('_');
        }
    }
    while base.ends_with('_') {
        base.pop();
    }
    if base.is_empty() {
        base = "action".to_string();
    }
    let mut id = base.clone();
    let mut number = 2;
    while BuiltIn::from_id(&id).is_some() || used.contains(&id) {
        id = format!("{base}_{number}");
        number += 1;
    }
    return id;
}

/// The default `[menu]` list `key`, with `added` after its `pin` entry
/// (after a separator too, if `separate`), one entry per line.
fn default_menu_with(key: &str, added: Vec<toml_edit::Value>, separate: bool) -> Array {
    let defaults = Config::parse(config::DEFAULTS, "");
    let mut entries = Vec::new();
    if let Some(array) = defaults
        .base_value_in("menu", key)
        .and_then(|v| v.as_array())
    {
        for entry in array.iter() {
            let mut entry = entry.clone();
            entry.decor_mut().clear();
            let is_pin = entry.as_str() == Some(BuiltIn::Pin.id());
            entries.push(entry);
            if is_pin {
                if separate {
                    entries.push(toml_edit::Value::from(menu::SEPARATOR));
                }
                entries.extend(added.iter().cloned());
            }
        }
    }
    let mut array = Array::new();
    for mut entry in entries {
        entry.decor_mut().set_prefix("\n  ");
        array.push_formatted(entry);
    }
    array.set_trailing_comma(true);
    array.set_trailing("\n");
    return array;
}

fn string_array(items: &[String]) -> Array {
    let mut array = Array::new();
    for item in items {
        array.push(item.as_str());
    }
    return array;
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
    use crate::menu::{Entry, MenuConfig, MenuKind, Pick};
    use crate::test_dir::TestDir;

    const OLD_CONFIG_TEXT: &str = "view=list\nzoom=1.25\nshow_hidden=true\n\
        details_columns=size,created\nstatus_fields=\ntheme=windows10\n\
        last_folder.org.mozilla.firefox=%2Fhome%2Fme\n";
    const OLD_ACTIONS_TEXT: &str = "# mine\n[Open in terminal]\n\
        command = xfce4-terminal --working-directory=%d\nfor = folders, background\n\
        [Copy]\ncommand = convert %f \"%f.png\"\nfor = *.jpg\n\
        [Copy!]\ncommand = true\nfor = *.jpg, background\n";
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
        let menus = menu::load(&Config::parse(config::DEFAULTS, ""), &config);
        assert!(
            menus.problems.is_empty(),
            "{:?}\n{config_toml}",
            menus.problems
        );
        let mut ids = Vec::new();
        for action in &menus.actions {
            ids.push(action.id.as_str());
        }
        assert_eq!(
            ids,
            ["open_in_terminal", "copy_2", "copy_3"],
            "copy is a built-in"
        );
        assert_eq!(menus.actions[0].name, "Open in terminal");
        let item = actions::Item {
            path: Path::new("/p/a.jpg"),
            is_dir: false,
        };
        let lines = menus.actions[1].command_lines(&[item], Path::new("/p"));
        assert_eq!(lines[0][2], "/p/a.jpg.png", "quotes survive");

        // Where they show: as before the move.
        let item_menu = menu_ids(&menus, MenuKind::Item);
        assert_eq!(
            item_menu[..6],
            ["open", "open_in_new_tab", "pin", "a0", "a1", "a2"]
        );
        let background = menu_ids(&menus, MenuKind::Background);
        assert_eq!(
            background[..6],
            ["paste", "new_folder", "pin", "---", "a0", "a2"]
        );
        let Entry::Item { targets, .. } = &menus.menu(MenuKind::Background, false)[5] else {
            panic!("an entry");
        };
        assert!(
            targets.is_some(),
            "a *.jpg action still shows on empty space"
        );
        // The menus that had no actions stay the defaults.
        assert!(!config_toml.contains("bookmark ="));
        assert!(
            config_toml.contains("\n  \"open\",\n"),
            "one entry per line:\n{config_toml}"
        );
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
        assert_eq!(config.base_subtables("action").0.len(), 3);
        assert!(dir.join(OLD_CONFIG).exists());

        // Done once: later runs leave the new files alone.
        std::fs::write(dir.join(LIVE_FILE), "view = \"details\"\n").unwrap();
        std::fs::remove_file(dir.join(CONFIG_FILE)).unwrap();
        run(dir);
        assert!(!dir.join(CONFIG_FILE).exists());
        assert_eq!(Config::load(dir).get_str("view"), Some("details"));
    }

    /// A menu's entries: built-in ids, `a<index>` for actions.
    fn menu_ids(menus: &MenuConfig, kind: MenuKind) -> Vec<String> {
        let mut ids = Vec::new();
        for entry in menus.menu(kind, false) {
            match entry {
                Entry::Item {
                    pick: Pick::BuiltIn(builtin),
                    ..
                } => ids.push(builtin.id().to_string()),
                Entry::Item {
                    pick: Pick::Action(index),
                    ..
                } => ids.push(format!("a{index}")),
                Entry::Separator => ids.push("---".to_string()),
                Entry::Submenu { name, .. } => ids.push(name.clone()),
            }
        }
        return ids;
    }
}
