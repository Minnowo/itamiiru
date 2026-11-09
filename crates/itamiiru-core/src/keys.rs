//! Key bindings: the `[keys]` table maps a key to a command, by the same
//! ids the menus use (built-in commands and actions, see `menu.rs`).
//!
//! ```text
//! [keys]
//! "ctrl+shift+n" = "new_folder"
//! f2 = "rename"
//! "ctrl+alt+t" = "open_terminal"   # an action
//! ```
//!
//! A key is modifiers and a key name joined by `+`, in any order and
//! case: `ctrl`, `alt`, `shift`, then one of `KEY_NAMES`. `ctrl` is Cmd on
//! macOS. Like a menu, a `[keys]` table in config.toml replaces the
//! defaults' whole: a key left out of it is not bound.

use crate::actions::Action;
use crate::config::Config;
use crate::menu::{BuiltIn, Pick};

/// The key names a binding can use: egui's key names, lowercased. Letters,
/// digits, F1-F35, and the named keys.
pub const KEY_NAMES: &[&str] = &[
    "a",
    "b",
    "c",
    "d",
    "e",
    "f",
    "g",
    "h",
    "i",
    "j",
    "k",
    "l",
    "m",
    "n",
    "o",
    "p",
    "q",
    "r",
    "s",
    "t",
    "u",
    "v",
    "w",
    "x",
    "y",
    "z",
    "0",
    "1",
    "2",
    "3",
    "4",
    "5",
    "6",
    "7",
    "8",
    "9",
    "f1",
    "f2",
    "f3",
    "f4",
    "f5",
    "f6",
    "f7",
    "f8",
    "f9",
    "f10",
    "f11",
    "f12",
    "f13",
    "f14",
    "f15",
    "f16",
    "f17",
    "f18",
    "f19",
    "f20",
    "f21",
    "f22",
    "f23",
    "f24",
    "f25",
    "f26",
    "f27",
    "f28",
    "f29",
    "f30",
    "f31",
    "f32",
    "f33",
    "f34",
    "f35",
    "up",
    "down",
    "left",
    "right",
    "pageup",
    "pagedown",
    "home",
    "end",
    "insert",
    "delete",
    "backspace",
    "enter",
    "escape",
    "tab",
    "space",
    "comma",
    "period",
    "minus",
    "plus",
    "equals",
    "semicolon",
    "colon",
    "quote",
    "backtick",
    "slash",
    "backslash",
    "pipe",
    "questionmark",
    "exclamationmark",
    "openbracket",
    "closebracket",
    "opencurlybracket",
    "closecurlybracket",
    "browserback",
];

/// A key with its modifiers: `ctrl+shift+n`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Chord {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    /// One of `KEY_NAMES`.
    pub key: &'static str,
}

/// One key and what it runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub chord: Chord,
    pub pick: Pick,
}

/// Every binding, and what was wrong in the config.
#[derive(Debug, Default)]
pub struct KeyConfig {
    /// In the table's order.
    pub bindings: Vec<Binding>,
    pub problems: Vec<String>,
}

impl Chord {
    /// `ctrl+shift+n`, `F2`, `Alt+Left`. Modifiers in any order and case;
    /// the key name last.
    pub fn parse(text: &str) -> Result<Chord, String> {
        let mut chord = Chord {
            ctrl: false,
            alt: false,
            shift: false,
            key: "",
        };
        let parts: Vec<&str> = text.split('+').collect();
        for (index, part) in parts.iter().enumerate() {
            let part = part.trim().to_ascii_lowercase();
            if index + 1 < parts.len() {
                match part.as_str() {
                    "ctrl" => chord.ctrl = true,
                    "alt" => chord.alt = true,
                    "shift" => chord.shift = true,
                    "" => return Err("write the plus key as \"plus\"".to_string()),
                    other => {
                        return Err(format!("{other:?} is not a modifier (ctrl, alt or shift)"));
                    }
                }
                continue;
            }
            if part.is_empty() {
                return Err("no key after the modifiers".to_string());
            }
            match key_name(&part) {
                Some(name) => chord.key = name,
                None => return Err(format!("{part:?} is not a key name")),
            }
        }
        return Ok(chord);
    }

    /// The canonical form: `ctrl+alt+shift+key`.
    pub fn text(&self) -> String {
        let mut text = String::new();
        if self.ctrl {
            text.push_str("ctrl+");
        }
        if self.alt {
            text.push_str("alt+");
        }
        if self.shift {
            text.push_str("shift+");
        }
        text.push_str(self.key);
        return text;
    }
}

impl KeyConfig {
    /// What `chord` runs, if it's bound.
    pub fn pick(&self, chord: &Chord) -> Option<Pick> {
        for binding in &self.bindings {
            if binding.chord == *chord {
                return Some(binding.pick);
            }
        }
        return None;
    }

    /// The keys bound to `pick`, in the table's order (the first is the
    /// one hints show).
    pub fn chords_for(&self, pick: Pick) -> Vec<&Chord> {
        let mut chords = Vec::new();
        for binding in &self.bindings {
            if binding.pick == pick {
                chords.push(&binding.chord);
            }
        }
        return chords;
    }
}

/// The name in `KEY_NAMES` equal to `name` (already lowercase).
pub fn key_name(name: &str) -> Option<&'static str> {
    for known in KEY_NAMES {
        if *known == name {
            return Some(known);
        }
    }
    return None;
}

/// Reads `[keys]` from `user` (config.toml) if it has one, else from
/// `defaults` (normally `config::DEFAULTS`); `actions` are the loaded
/// actions (`MenuConfig::actions`). Bindings with problems are left out
/// and the problems listed.
pub fn load(defaults: &Config, user: &Config, actions: &[Action]) -> KeyConfig {
    let config = if user.has_base_table("keys") {
        user
    } else {
        defaults
    };
    let mut problems = Vec::new();
    let mut seen: Vec<Chord> = Vec::new();
    let mut bindings = Vec::new();
    for key in config.base_keys_in("keys") {
        let where_ = format!("keys.{key:?}");
        let chord = match Chord::parse(&key) {
            Ok(chord) => chord,
            Err(err) => {
                problems.push(format!("{where_}: {err}"));
                continue;
            }
        };
        if seen.contains(&chord) {
            problems.push(format!("{where_}: {} is bound twice", chord.text()));
            continue;
        }
        seen.push(chord.clone());
        let Some(id) = config
            .base_value_in("keys", &key)
            .and_then(|value| value.as_str())
        else {
            problems.push(format!("{where_}: must be a command id like \"rename\""));
            continue;
        };
        match resolve(id, actions) {
            Ok(pick) => bindings.push(Binding { chord, pick }),
            Err(err) => problems.push(format!("{where_}: {err}")),
        }
    }
    return KeyConfig { bindings, problems };
}

/// What an id stands for, if a key can run it.
fn resolve(id: &str, actions: &[Action]) -> Result<Pick, String> {
    for (index, action) in actions.iter().enumerate() {
        if action.id == id {
            return Ok(Pick::Action(index));
        }
    }
    let Some(builtin) = BuiltIn::from_id(id) else {
        return Err(format!("no built-in command or action is called {id:?}"));
    };
    if !builtin.works_as_key() {
        return Err(format!("{id} can't be run by a key"));
    }
    return Ok(Pick::BuiltIn(builtin));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::menu;

    const TEST_DEFAULTS: &str = r#"
[action.terminal]
name = "Terminal"
command = "true"

[keys]
f2 = "rename"
f5 = "refresh"
"ctrl+r" = "refresh"
"ctrl+h" = "toggle_hidden"
"#;

    #[test]
    fn chords_parse_in_any_order_and_case() {
        let chord = Chord::parse("Shift+CTRL+n").expect("valid");
        assert_eq!(
            chord,
            Chord {
                ctrl: true,
                alt: false,
                shift: true,
                key: "n"
            }
        );
        assert_eq!(chord.text(), "ctrl+shift+n");
        assert_eq!(Chord::parse("ctrl + plus").expect("valid").key, "plus");
        assert_eq!(Chord::parse("F12").expect("valid").text(), "f12");

        for bad in ["", "ctrl+", "ctrl++", "super+a", "ctrl+enterr", "a+b"] {
            assert!(Chord::parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn a_user_keys_table_replaces_the_defaults_whole() {
        let keys = load_text(
            r#"
[keys]
f2 = "refresh"
"ctrl+alt+t" = "terminal"
"#,
        );
        assert!(keys.problems.is_empty(), "{:?}", keys.problems);
        assert_eq!(bound(&keys), ["f2 = refresh", "ctrl+alt+t = action0"]);
        // Left out of the user's table: not bound.
        assert_eq!(keys.pick(&chord("f5")), None);
        assert_eq!(keys.pick(&chord("ctrl+h")), None);
        assert_eq!(
            keys.pick(&chord("ctrl+alt+t")),
            Some(Pick::Action(0)),
            "an action by its id"
        );
        assert!(keys.chords_for(Pick::BuiltIn(BuiltIn::Rename)).is_empty());

        // An empty table binds nothing; no table keeps the defaults.
        assert!(load_text("[keys]\n").bindings.is_empty());
        let defaults = load_text("");
        assert_eq!(
            bound(&defaults),
            [
                "f2 = rename",
                "f5 = refresh",
                "ctrl+r = refresh",
                "ctrl+h = toggle_hidden",
            ]
        );
        let refresh = defaults.chords_for(Pick::BuiltIn(BuiltIn::Refresh));
        assert_eq!(refresh[0].text(), "f5", "the table's first key first");
    }

    #[test]
    fn the_same_key_written_differently_is_the_same_key() {
        let keys = load_text("[keys]\n\"Ctrl+R\" = \"rename\"\n");
        assert!(keys.problems.is_empty(), "{:?}", keys.problems);
        assert_eq!(
            keys.pick(&chord("ctrl+r")),
            Some(Pick::BuiltIn(BuiltIn::Rename))
        );
    }

    #[test]
    fn problems_skip_bindings_not_the_table() {
        let keys = load_text(
            r#"
[keys]
"ctrl+q" = "quit"
"hyper+a" = "rename"
"ctrl+d" = "sort_by"
"ctrl+e" = "eject"
"ctrl+g" = 3
"ctrl+shift+x" = "cut"
"Shift+Ctrl+X" = "copy"
f9 = "select_all"
"#,
        );
        let expected = [
            "keys.\"ctrl+q\": no built-in command or action is called \"quit\"",
            "keys.\"hyper+a\": \"hyper\" is not a modifier",
            "keys.\"ctrl+d\": sort_by can't be run by a key",
            "keys.\"ctrl+e\": eject can't be run by a key",
            "keys.\"ctrl+g\": must be a command id",
            "keys.\"Shift+Ctrl+X\": ctrl+shift+x is bound twice",
        ];
        let mut sorted = keys.problems.clone();
        sorted.sort();
        let mut want = expected.to_vec();
        want.sort();
        assert_eq!(sorted.len(), want.len(), "{:#?}", keys.problems);
        for (problem, start) in sorted.iter().zip(want) {
            assert!(
                problem.starts_with(start),
                "{problem:?} should start with {start:?}"
            );
        }
        assert_eq!(
            keys.pick(&chord("ctrl+shift+x")),
            Some(Pick::BuiltIn(BuiltIn::Cut)),
            "the first of the two is kept"
        );
        assert_eq!(
            keys.pick(&chord("f9")),
            Some(Pick::BuiltIn(BuiltIn::SelectAll))
        );
    }

    #[test]
    fn the_built_in_defaults_have_no_problems() {
        let defaults = Config::parse(crate::config::DEFAULTS, "");
        let menus = menu::load(&defaults, &Config::default());
        let keys = load(&defaults, &Config::default(), &menus.actions);
        assert!(keys.problems.is_empty(), "{:?}", keys.problems);
        assert!(!keys.bindings.is_empty());
        // Every command a key can run has a default key, except the ones
        // listed here.
        let unbound = [
            BuiltIn::OpenInNewTab,
            BuiltIn::CopyPath,
            BuiltIn::CopyToOtherPane,
            BuiltIn::MoveToOtherPane,
            BuiltIn::ToggleSplit,
        ];
        for builtin in BuiltIn::ALL {
            if !builtin.works_as_key() || unbound.contains(&builtin) {
                continue;
            }
            assert!(
                !keys.chords_for(Pick::BuiltIn(builtin)).is_empty(),
                "{} has no default key",
                builtin.id()
            );
        }
    }

    fn load_text(user: &str) -> KeyConfig {
        let defaults = Config::parse(TEST_DEFAULTS, "");
        let user = Config::parse(user, "");
        let menus = menu::load(&defaults, &user);
        return load(&defaults, &user, &menus.actions);
    }

    fn chord(text: &str) -> Chord {
        return Chord::parse(text).expect("a valid chord");
    }

    /// The bindings as `key = id`; actions as `action<index>`.
    fn bound(keys: &KeyConfig) -> Vec<String> {
        let mut pairs = Vec::new();
        for binding in &keys.bindings {
            let id = match binding.pick {
                Pick::BuiltIn(builtin) => builtin.id().to_string(),
                Pick::Action(index) => format!("action{index}"),
            };
            pairs.push(format!("{} = {id}", binding.chord.text()));
        }
        return pairs;
    }
}
