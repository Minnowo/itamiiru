//! Key bindings in egui terms: finds the bound keys pressed this frame
//! (`itamiiru_core::keys` reads them from the config), and writes a
//! command's key for menus and tooltips ("Ctrl+Shift+N").

use eframe::egui::{self, Event, Key, Modifiers};
use itamiiru_core::keys::{self, Chord, KeyConfig};
use itamiiru_core::menu::{BuiltIn, Pick};

/// Takes the bound keys pressed this frame out of `input`, so egui and the
/// rest of the app don't also act on them, and returns what they run, in
/// order. When a key that types a character (no Ctrl or Alt) is bound,
/// this frame's typed text goes too, so it doesn't reach type-ahead.
pub fn take_pressed(input: &mut egui::InputState, keys: &KeyConfig) -> Vec<Pick> {
    let mut picks = Vec::new();
    let mut ate_text = false;
    // egui-winit turns Ctrl+X/C/V into these events, with no key event;
    // Ctrl was down, whatever the frame's modifiers say by now.
    let clipboard_modifiers = input.modifiers | Modifiers::COMMAND;
    input.events.retain(|event| {
        let chord = match event {
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => chord_of(*key, *modifiers),
            Event::Cut => chord_of(Key::X, clipboard_modifiers),
            Event::Copy => chord_of(Key::C, clipboard_modifiers),
            Event::Paste(_) => chord_of(Key::V, clipboard_modifiers),
            _ => None,
        };
        let Some(chord) = chord else {
            return true;
        };
        let Some(pick) = keys.pick(&chord) else {
            return true;
        };
        picks.push(pick);
        if !chord.ctrl && !chord.alt && types_text(chord.key) {
            ate_text = true;
        }
        return false;
    });
    if ate_text {
        input
            .events
            .retain(|event| !matches!(event, Event::Text(_)));
    }
    return picks;
}

/// The first key bound to `pick`, as the menus show it ("Ctrl+Shift+N"),
/// or "" if none is.
pub fn hint(keys: &KeyConfig, pick: Pick) -> String {
    return match keys.chords_for(pick).first() {
        Some(chord) => chord_text(chord),
        None => String::new(),
    };
}

/// A tooltip with the built-in's key, if it has one: "Back (Alt+Left)".
pub fn tip(keys: &KeyConfig, text: &str, builtin: BuiltIn) -> String {
    let key = hint(keys, Pick::BuiltIn(builtin));
    if key.is_empty() {
        return text.to_string();
    }
    return format!("{text} ({key})");
}

/// "Ctrl+Alt+Shift+PageUp": egui's key name, so it matches its own hints.
fn chord_text(chord: &Chord) -> String {
    let mut text = String::new();
    if chord.ctrl {
        text.push_str("Ctrl+");
    }
    if chord.alt {
        text.push_str("Alt+");
    }
    if chord.shift {
        text.push_str("Shift+");
    }
    match egui_key(chord.key) {
        Some(key) => text.push_str(key.name()),
        None => text.push_str(chord.key),
    }
    return text;
}

/// The chord for a key egui reports; None for keys bindings can't name
/// (Shift or Ctrl on their own).
fn chord_of(key: Key, modifiers: Modifiers) -> Option<Chord> {
    let name = keys::key_name(&key.name().to_ascii_lowercase())?;
    return Some(Chord {
        ctrl: modifiers.command,
        alt: modifiers.alt,
        shift: modifiers.shift,
        key: name,
    });
}

/// The egui key for a name in `keys::KEY_NAMES`.
fn egui_key(name: &str) -> Option<Key> {
    for key in Key::ALL {
        if key.name().eq_ignore_ascii_case(name) {
            return Some(*key);
        }
    }
    return None;
}

/// Whether pressing the key alone types a character.
fn types_text(name: &str) -> bool {
    let named = [
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
    ];
    return name.len() == 1 || named.contains(&name);
}

#[cfg(test)]
mod tests {
    use super::*;
    use itamiiru_core::config::{self, Config};
    use itamiiru_core::menu;

    #[test]
    fn every_key_name_is_an_egui_key() {
        for name in keys::KEY_NAMES {
            let key = egui_key(name).unwrap_or_else(|| panic!("{name:?}"));
            assert_eq!(chord_of(key, Modifiers::NONE).expect("nameable").key, *name);
        }
    }

    #[test]
    fn bound_keys_are_taken_exactly() {
        let keys = default_keys(
            "[keys]\n\"ctrl+shift+n\" = \"new_folder\"\n\"ctrl+alt+r\" = \"rename\"\n\
             \"ctrl+c\" = \"copy\"\n\"ctrl+r\" = \"refresh\"\n",
        );
        let mut input = egui::InputState::default();
        input.events = vec![
            key_event(Key::N, Modifiers::COMMAND | Modifiers::SHIFT),
            // Unbound: left for egui.
            key_event(Key::Q, Modifiers::COMMAND),
            key_event(Key::R, Modifiers::COMMAND | Modifiers::ALT),
            Event::Copy,
            // Ctrl+R alone is refresh; the extra Alt above made it rename.
            key_event(Key::R, Modifiers::COMMAND),
        ];
        let picks = take_pressed(&mut input, &keys);
        assert_eq!(
            picks,
            [
                Pick::BuiltIn(BuiltIn::NewFolder),
                Pick::BuiltIn(BuiltIn::Rename),
                Pick::BuiltIn(BuiltIn::Copy),
                Pick::BuiltIn(BuiltIn::Refresh),
            ]
        );
        assert_eq!(input.events, [key_event(Key::Q, Modifiers::COMMAND)]);
    }

    #[test]
    fn a_bound_letter_does_not_also_type() {
        let keys = default_keys("[keys]\nj = \"cursor_down\"\n");
        let mut input = egui::InputState::default();
        input.events = vec![
            key_event(Key::J, Modifiers::NONE),
            Event::Text("j".to_string()),
        ];
        let picks = take_pressed(&mut input, &keys);
        assert_eq!(picks, [Pick::BuiltIn(BuiltIn::CursorDown)]);
        assert!(input.events.is_empty(), "{:?}", input.events);

        // Unbound letters still type.
        input.events = vec![
            key_event(Key::K, Modifiers::NONE),
            Event::Text("k".to_string()),
        ];
        assert!(take_pressed(&mut input, &keys).is_empty());
        assert_eq!(input.events.len(), 2);
    }

    #[test]
    fn hints_and_tips() {
        let keys = default_keys("");
        assert_eq!(
            hint(&keys, Pick::BuiltIn(BuiltIn::NewFolder)),
            "Ctrl+Shift+N"
        );
        assert_eq!(hint(&keys, Pick::BuiltIn(BuiltIn::CursorPageUp)), "PageUp");
        assert_eq!(tip(&keys, "Up", BuiltIn::Up), "Up (Alt+Up)");
        let only_up = default_keys("[keys]\nf1 = \"up\"\n");
        assert_eq!(tip(&only_up, "Back", BuiltIn::Back), "Back", "unbound");
    }

    fn default_keys(user: &str) -> KeyConfig {
        let defaults = Config::parse(config::DEFAULTS, "");
        let user = Config::parse(user, "");
        let menus = menu::load(&defaults, &user);
        let keys = keys::load(&defaults, &user, &menus.actions);
        assert!(keys.problems.is_empty(), "{:?}", keys.problems);
        return keys;
    }

    fn key_event(key: Key, modifiers: Modifiers) -> Event {
        return Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        };
    }
}
