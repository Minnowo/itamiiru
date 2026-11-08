//! The right-click menus as configured: `[menu]` (and `[picker_menu]` for
//! the file dialog) lists each menu's entries, and `[action.<id>]` tables
//! define custom actions. The built-in defaults come from a TOML file the
//! app passes in; a menu set in config.toml replaces that menu, and an
//! action there replaces the default action with the same id.
//!
//! ```text
//! [menu]
//! item = [
//!   "open", "---",
//!   { name = "7-Zip", for = ["*.zip"], items = ["extract_here"] },
//!   { id = "rename", name = "Rename..." },
//! ]
//! ```
//!
//! An entry is an id (a built-in command or an action), `"---"` (a
//! separator), `{ id, name?, for? }` (an id with its own name, or shown
//! only for some items), or `{ name, for?, items }` (a submenu).
//!
//! Ids are resolved here, once, into a `Pick`: what an entry runs. The app
//! gives built-ins their labels and behavior.

use toml_edit::Value;

use crate::actions::{self, Action, Targets};
use crate::config::Config;

/// Submenus deeper than this are left out (and reported).
pub const MAX_DEPTH: usize = 16;
/// The separator entry.
pub const SEPARATOR: &str = "---";

/// One of the configurable menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MenuKind {
    /// Files and folders in the file list.
    Item,
    /// Empty space in the file list.
    Background,
    /// A bookmark in the side pane.
    Bookmark,
    /// A place: Home, Desktop, Downloads, network places.
    Place,
    /// A drive, in the side pane or on This PC.
    Drive,
}

/// A built-in command, by its id in the menus.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BuiltIn {
    Open,
    OpenInNewTab,
    /// Pin to, or unpin from, Bookmarks.
    Pin,
    CopyPath,
    Cut,
    Copy,
    Paste,
    CopyToOtherPane,
    MoveToOtherPane,
    Rename,
    Trash,
    /// Delete permanently, after asking.
    Delete,
    NewFolder,
    /// The "Sort by" submenu.
    SortBy,
    SelectAll,
    Refresh,
    ToggleHidden,
    TogglePreview,
    ToggleSplit,
    Settings,
    Mount,
    Unmount,
    Eject,
}

/// What a menu entry runs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    BuiltIn(BuiltIn),
    /// An index into `MenuConfig::actions`.
    Action(usize),
}

#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// A built-in command or an action. `name` replaces its own;
    /// `targets` replaces the action's `for`.
    Item {
        pick: Pick,
        name: Option<String>,
        targets: Option<Targets>,
    },
    Separator,
    Submenu {
        name: String,
        targets: Option<Targets>,
        items: Vec<Entry>,
    },
}

/// Every menu, the actions they use, and what was wrong in the config.
#[derive(Debug, Default)]
pub struct MenuConfig {
    pub actions: Vec<Action>,
    /// The explorer's menus, by `MenuKind::index`.
    pub explorer: Vec<Vec<Entry>>,
    /// The file dialog's.
    pub picker: Vec<Vec<Entry>>,
    pub problems: Vec<String>,
}

impl MenuKind {
    pub const ALL: [MenuKind; 5] = [
        MenuKind::Item,
        MenuKind::Background,
        MenuKind::Bookmark,
        MenuKind::Place,
        MenuKind::Drive,
    ];

    /// The key under `[menu]`.
    pub fn key(self) -> &'static str {
        return match self {
            MenuKind::Item => "item",
            MenuKind::Background => "background",
            MenuKind::Bookmark => "bookmark",
            MenuKind::Place => "place",
            MenuKind::Drive => "drive",
        };
    }

    pub fn index(self) -> usize {
        return self as usize;
    }

    pub fn from_key(key: &str) -> Option<MenuKind> {
        for kind in MenuKind::ALL {
            if kind.key() == key {
                return Some(kind);
            }
        }
        return None;
    }
}

impl BuiltIn {
    pub const ALL: [BuiltIn; 23] = [
        BuiltIn::Open,
        BuiltIn::OpenInNewTab,
        BuiltIn::Pin,
        BuiltIn::CopyPath,
        BuiltIn::Cut,
        BuiltIn::Copy,
        BuiltIn::Paste,
        BuiltIn::CopyToOtherPane,
        BuiltIn::MoveToOtherPane,
        BuiltIn::Rename,
        BuiltIn::Trash,
        BuiltIn::Delete,
        BuiltIn::NewFolder,
        BuiltIn::SortBy,
        BuiltIn::SelectAll,
        BuiltIn::Refresh,
        BuiltIn::ToggleHidden,
        BuiltIn::TogglePreview,
        BuiltIn::ToggleSplit,
        BuiltIn::Settings,
        BuiltIn::Mount,
        BuiltIn::Unmount,
        BuiltIn::Eject,
    ];

    /// The id used in the menus.
    pub fn id(self) -> &'static str {
        return match self {
            BuiltIn::Open => "open",
            BuiltIn::OpenInNewTab => "open_in_new_tab",
            BuiltIn::Pin => "pin",
            BuiltIn::CopyPath => "copy_path",
            BuiltIn::Cut => "cut",
            BuiltIn::Copy => "copy",
            BuiltIn::Paste => "paste",
            BuiltIn::CopyToOtherPane => "copy_to_other_pane",
            BuiltIn::MoveToOtherPane => "move_to_other_pane",
            BuiltIn::Rename => "rename",
            BuiltIn::Trash => "trash",
            BuiltIn::Delete => "delete",
            BuiltIn::NewFolder => "new_folder",
            BuiltIn::SortBy => "sort_by",
            BuiltIn::SelectAll => "select_all",
            BuiltIn::Refresh => "refresh",
            BuiltIn::ToggleHidden => "toggle_hidden",
            BuiltIn::TogglePreview => "toggle_preview",
            BuiltIn::ToggleSplit => "toggle_split",
            BuiltIn::Settings => "settings",
            BuiltIn::Mount => "mount",
            BuiltIn::Unmount => "unmount",
            BuiltIn::Eject => "eject",
        };
    }

    pub fn from_id(id: &str) -> Option<BuiltIn> {
        for builtin in BuiltIn::ALL {
            if builtin.id() == id {
                return Some(builtin);
            }
        }
        return None;
    }

    /// Whether it can do anything in this menu.
    pub fn works_in(self, kind: MenuKind) -> bool {
        use MenuKind::{Background, Bookmark, Drive, Item, Place};
        let kinds: &[MenuKind] = match self {
            BuiltIn::Open | BuiltIn::OpenInNewTab => &[Item, Bookmark, Place, Drive],
            BuiltIn::Pin | BuiltIn::CopyPath => &[Item, Background, Bookmark, Place, Drive],
            BuiltIn::Paste => &[Item, Background],
            BuiltIn::Cut
            | BuiltIn::Copy
            | BuiltIn::CopyToOtherPane
            | BuiltIn::MoveToOtherPane
            | BuiltIn::Rename
            | BuiltIn::Trash
            | BuiltIn::Delete => &[Item],
            BuiltIn::NewFolder
            | BuiltIn::SortBy
            | BuiltIn::SelectAll
            | BuiltIn::Refresh
            | BuiltIn::ToggleHidden => &[Background],
            BuiltIn::TogglePreview | BuiltIn::ToggleSplit | BuiltIn::Settings => {
                &[Item, Background]
            }
            BuiltIn::Mount | BuiltIn::Unmount | BuiltIn::Eject => &[Drive],
        };
        return kinds.contains(&kind);
    }
}

impl MenuConfig {
    /// One menu, for the explorer or the file dialog (`picker`).
    pub fn menu(&self, kind: MenuKind, picker: bool) -> &[Entry] {
        let menus = if picker { &self.picker } else { &self.explorer };
        return match menus.get(kind.index()) {
            Some(entries) => entries,
            None => &[],
        };
    }

    /// The action with this id.
    pub fn action(&self, id: &str) -> Option<&Action> {
        for action in &self.actions {
            if action.id == id {
                return Some(action);
            }
        }
        return None;
    }
}

/// Reads the actions and menus from `defaults` (normally `config::DEFAULTS`) and
/// `user` (config.toml). Entries with problems are left out and the
/// problems listed; the rest still works.
pub fn load(defaults: &Config, user: &Config) -> MenuConfig {
    let mut config = MenuConfig::default();
    let mut problems = Vec::new();

    // Actions: the defaults, each replaced by the user's with its id, then
    // the user's new ones.
    let (default_tables, default_problems) = defaults.base_subtables("action");
    let (user_tables, user_problems) = user.base_subtables("action");
    problems.extend(default_problems);
    problems.extend(user_problems);
    let mut tables = Vec::new();
    for (id, table) in &default_tables {
        let mut chosen = *table;
        for (user_id, user_table) in &user_tables {
            if user_id == id {
                chosen = *user_table;
            }
        }
        tables.push((id.clone(), chosen));
    }
    for (id, table) in &user_tables {
        if !default_tables
            .iter()
            .any(|(default_id, _)| default_id == id)
        {
            tables.push((id.clone(), *table));
        }
    }
    // An action can't take a built-in's id: the menus couldn't tell them
    // apart.
    let mut checked = Vec::new();
    for (id, table) in tables {
        if BuiltIn::from_id(&id).is_some() {
            problems.push(format!("action {id}: {id} is a built-in command's id"));
            continue;
        }
        checked.push((id, table));
    }
    let (actions, action_problems) = actions::parse_actions(&checked);
    config.actions = actions;
    problems.extend(action_problems);

    for (table, picker) in [("menu", false), ("picker_menu", true)] {
        for key in user.base_keys_in(table) {
            if MenuKind::from_key(&key).is_none() {
                problems.push(format!("{table}.{key}: no such menu"));
            }
        }
        let mut menus = Vec::new();
        for kind in MenuKind::ALL {
            let value = match user.base_value_in(table, kind.key()) {
                Some(value) => Some(value),
                None => defaults.base_value_in(table, kind.key()),
            };
            let where_ = format!("{table}.{}", kind.key());
            let mut parser = Parser {
                kind,
                actions: &config.actions,
                problems: &mut problems,
            };
            let entries = match value {
                Some(value) => parser.list(&where_, value, 1),
                None => Vec::new(),
            };
            menus.push(entries);
        }
        if picker {
            config.picker = menus;
        } else {
            config.explorer = menus;
        }
    }
    config.problems = problems;
    return config;
}

/// Reads one menu's entries, checking each id.
struct Parser<'a> {
    kind: MenuKind,
    actions: &'a [Action],
    problems: &'a mut Vec<String>,
}

impl Parser<'_> {
    /// A list of entries at `depth` (1 for the menu itself).
    fn list(&mut self, where_: &str, value: &Value, depth: usize) -> Vec<Entry> {
        let mut entries = Vec::new();
        let Some(array) = value.as_array() else {
            self.problems.push(format!(
                "{where_}: must be a list like [\"open\", \"---\", \"copy\"]"
            ));
            return entries;
        };
        for (index, value) in array.iter().enumerate() {
            let here = format!("{where_}[{}]", index + 1);
            match self.entry(&here, value, depth) {
                Ok(entry) => entries.push(entry),
                Err(err) => self.problems.push(format!("{here}: {err}")),
            }
        }
        return entries;
    }

    fn entry(&mut self, where_: &str, value: &Value, depth: usize) -> Result<Entry, String> {
        if let Some(text) = value.as_str() {
            if text == SEPARATOR {
                return Ok(Entry::Separator);
            }
            return Ok(Entry::Item {
                pick: self.pick(text)?,
                name: None,
                targets: None,
            });
        }
        let Some(table) = value.as_inline_table() else {
            return Err("must be an id, \"---\" or { ... }".to_string());
        };
        let mut id = None;
        let mut name = None;
        let mut targets = None;
        let mut items = None;
        for (key, value) in table.iter() {
            match key {
                "id" => id = Some(string(key, value)?),
                "name" => name = Some(string(key, value)?),
                "for" => targets = Some(Targets::parse(&toml_edit::Item::Value(value.clone()))?),
                "items" => items = Some(value),
                other => return Err(format!("unknown key {other:?}")),
            }
        }
        if let Some(name) = &name {
            actions::check_name(name)?;
        }
        if let Some(items) = items {
            if id.is_some() {
                return Err("a submenu has items, not an id".to_string());
            }
            let Some(name) = name else {
                return Err("a submenu needs a name".to_string());
            };
            if depth >= MAX_DEPTH {
                return Err(format!("submenus go at most {MAX_DEPTH} deep"));
            }
            let items = self.list(where_, items, depth + 1);
            return Ok(Entry::Submenu {
                name,
                targets,
                items,
            });
        }
        let Some(id) = id else {
            return Err("needs an id, or a name and items".to_string());
        };
        return Ok(Entry::Item {
            pick: self.pick(&id)?,
            name,
            targets,
        });
    }

    /// What an id stands for, if it works in this menu.
    fn pick(&self, id: &str) -> Result<Pick, String> {
        for (index, action) in self.actions.iter().enumerate() {
            if action.id == id {
                return Ok(Pick::Action(index));
            }
        }
        let Some(builtin) = BuiltIn::from_id(id) else {
            return Err(format!("no built-in command or action is called {id:?}"));
        };
        if !builtin.works_in(self.kind) {
            return Err(format!("{id} doesn't work in the {} menu", self.kind.key()));
        }
        return Ok(Pick::BuiltIn(builtin));
    }
}

fn string(key: &str, value: &Value) -> Result<String, String> {
    return match value.as_str() {
        Some(text) => Ok(text.to_string()),
        None => Err(format!("{key} must be a string")),
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_DEFAULTS: &str = r#"
[action.shared]
name = "Default shared"
command = "true"

[action.only_default]
name = "Only default"
command = "true"

[menu]
item = ["open", "paste", "only_default"]
background = ["paste"]
drive = ["open"]

[picker_menu]
item = ["open"]
"#;

    fn load_text(user: &str) -> MenuConfig {
        let defaults = Config::parse(TEST_DEFAULTS, "");
        return load(&defaults, &Config::parse(user, ""));
    }

    /// The entries' ids; actions as `action<index>`.
    fn ids(entries: &[Entry]) -> Vec<String> {
        let mut ids = Vec::new();
        for entry in entries {
            match entry {
                Entry::Item {
                    pick: Pick::BuiltIn(builtin),
                    ..
                } => ids.push(builtin.id().to_string()),
                Entry::Item {
                    pick: Pick::Action(index),
                    ..
                } => ids.push(format!("action{index}")),
                Entry::Separator => ids.push(SEPARATOR.to_string()),
                Entry::Submenu { name, items, .. } => {
                    ids.push(format!("{name}({})", ids_joined(items)));
                }
            }
        }
        return ids;
    }

    fn ids_joined(entries: &[Entry]) -> String {
        return ids(entries).join(" ");
    }

    #[test]
    fn defaults_alone() {
        let config = load_text("");
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        assert_eq!(
            ids(config.menu(MenuKind::Item, false)),
            ["open", "paste", "action1"]
        );
        assert_eq!(
            ids(config.menu(MenuKind::Bookmark, false)),
            Vec::<String>::new()
        );
        assert_eq!(ids(config.menu(MenuKind::Item, true)), ["open"]);
        assert_eq!(
            ids(config.menu(MenuKind::Background, true)),
            Vec::<String>::new(),
            "the picker has its own defaults"
        );
    }

    #[test]
    fn a_user_menu_replaces_that_menu_only() {
        let config = load_text("[menu]\nitem = [\"paste\", \"---\", \"open\"]\n");
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        assert_eq!(
            ids(config.menu(MenuKind::Item, false)),
            ["paste", "---", "open"]
        );
        assert_eq!(ids(config.menu(MenuKind::Background, false)), ["paste"]);
        assert_eq!(
            ids(config.menu(MenuKind::Item, true)),
            ["open"],
            "picker untouched"
        );
    }

    #[test]
    fn actions_by_id() {
        let config = load_text(
            "[action.shared]\nname = \"Mine\"\ncommand = \"true\"\n\
             [action.new_one]\nname = \"New\"\ncommand = \"true\"\n",
        );
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        let mut names = Vec::new();
        for action in &config.actions {
            names.push((action.id.as_str(), action.name.as_str()));
        }
        assert_eq!(
            names,
            [
                ("shared", "Mine"),
                ("only_default", "Only default"),
                ("new_one", "New")
            ]
        );
    }

    #[test]
    fn nested_submenus_and_overrides() {
        let config = load_text(
            r#"
[action.extract]
name = "Extract here"
command = "7z x %f"
for = ["*.zip"]

[menu]
item = [
  { name = "7-Zip", for = ["files"], items = [
      "extract",
      { name = "More", items = [{ id = "open", name = "Open it" }] },
  ]},
  { id = "extract", for = ["*.7z"] },
]
"#,
        );
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        let item = config.menu(MenuKind::Item, false);
        // extract is action 2, after the two defaults.
        assert_eq!(ids(item), ["7-Zip(action2 More(open))", "action2"]);
        let Entry::Submenu { items, targets, .. } = &item[0] else {
            panic!("a submenu");
        };
        assert!(targets.is_some());
        let Entry::Submenu { items: inner, .. } = &items[1] else {
            panic!("a nested submenu");
        };
        assert_eq!(
            inner[0],
            Entry::Item {
                pick: Pick::BuiltIn(BuiltIn::Open),
                name: Some("Open it".to_string()),
                targets: None,
            }
        );
    }

    #[test]
    fn problems_skip_entries_not_menus() {
        let config = load_text(
            r#"
[action.paste]
name = "Clash"
command = "true"

[action.broken]
name = "No command"

[menu]
item = [
  "open",
  "nope",
  "broken",
  3,
  { name = "No items key" },
  { items = ["open"] },
  { id = "open", items = ["open"] },
  { id = "open", color = "red" },
  { id = "open", for = ["everything"] },
  { id = "open", name = "" },
  "paste",
]
drive = ["paste", "open"]
tabs = ["open"]
background = "paste"
"#,
        );
        assert_eq!(ids(config.menu(MenuKind::Item, false)), ["open", "paste"]);
        assert_eq!(ids(config.menu(MenuKind::Drive, false)), ["open"]);
        assert_eq!(
            ids(config.menu(MenuKind::Background, false)),
            Vec::<String>::new()
        );
        assert!(config.action("paste").is_none(), "built-in ids are taken");
        let expected = [
            "action paste: paste is a built-in command's id",
            "action broken: no command",
            "menu.item[2]: no built-in",
            "menu.item[3]: no built-in",
            "menu.item[4]: must be",
            "menu.item[5]: needs an id",
            "menu.item[6]: a submenu needs a name",
            "menu.item[7]: a submenu has items",
            "menu.item[8]: unknown key",
            "menu.item[9]: unknown target",
            "menu.item[10]: a name must be",
            "menu.background: must be a list",
            "menu.drive[1]: paste doesn't work in the drive menu",
            "menu.tabs: no such menu",
        ];
        let mut sorted = config.problems.clone();
        sorted.sort();
        let mut want: Vec<&str> = expected.to_vec();
        want.sort();
        assert_eq!(sorted.len(), want.len(), "{:#?}", config.problems);
        for (problem, start) in sorted.iter().zip(want) {
            assert!(
                problem.starts_with(start),
                "{problem:?} should start with {start:?}"
            );
        }
    }

    #[test]
    fn depth_is_capped() {
        let mut menu = "\"open\"".to_string();
        for level in 0..MAX_DEPTH + 2 {
            menu = format!("{{ name = \"L{level}\", items = [{menu}] }}");
        }
        let config = load_text(&format!("[menu]\nitem = [{menu}]\n"));
        assert_eq!(config.problems.len(), 1, "{:?}", config.problems);
        assert!(config.problems[0].contains("at most"));
        // The levels above the cap are kept, the one that's too deep isn't.
        let mut depth = 0;
        let mut entries = config.menu(MenuKind::Item, false);
        while let Some(Entry::Submenu { items, .. }) = entries.first() {
            depth += 1;
            entries = items;
        }
        assert_eq!(depth, MAX_DEPTH - 1);
    }

    #[test]
    fn every_builtin_has_an_id_and_a_menu() {
        for builtin in BuiltIn::ALL {
            assert_eq!(BuiltIn::from_id(builtin.id()), Some(builtin));
            assert!(MenuKind::ALL.iter().any(|kind| builtin.works_in(*kind)));
        }
        for kind in MenuKind::ALL {
            assert_eq!(MenuKind::from_key(kind.key()), Some(kind));
        }
    }

    #[test]
    fn the_built_in_defaults_have_no_problems() {
        let defaults = Config::parse(crate::config::DEFAULTS, "");
        assert!(defaults.problems().is_empty());
        let config = load(&defaults, &Config::default());
        assert!(config.problems.is_empty(), "{:?}", config.problems);
        for picker in [false, true] {
            for kind in MenuKind::ALL {
                assert!(!config.menu(kind, picker).is_empty(), "{kind:?} {picker}");
            }
        }
    }
}
