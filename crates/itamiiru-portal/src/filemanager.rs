//! org.freedesktop.FileManager1: what apps call for "Show in
//! folder" (Firefox, Chromium, ...). Run with `--file-manager`, as its own
//! process with its own D-Bus activation file. Each call opens an `itami`
//! window and returns at once.
//! Spec: https://www.freedesktop.org/wiki/Specifications/file-manager-interface/

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::process::Command;

use crate::portal::{Event, picker_binary};
use itamiiru_core::platform::dbus::conn::Connection;
use itamiiru_core::platform::dbus::msg::{Kind, Message};
use itamiiru_core::platform::dbus::wire::Value;

pub const BUS_NAME: &str = "org.freedesktop.FileManager1";
const OBJECT_PATH: &str = "/org/freedesktop/FileManager1";
const INTERFACE: &str = "org.freedesktop.FileManager1";

/// One call can't open more windows than this, however many URIs it sends.
const MAX_WINDOWS: usize = 16;
/// Nor select more items than this in one window.
const MAX_ITEMS: usize = 1000;

const INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.FileManager1">
    <method name="ShowFolders">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
    <method name="ShowItems">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
    <method name="ShowItemProperties">
      <arg type="as" name="URIs" direction="in"/>
      <arg type="s" name="StartupId" direction="in"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect">
      <arg type="s" name="xml_data" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
  </interface>
</node>
"#;

/// One `itami` window to open: a folder, with these items selected.
#[derive(Debug, PartialEq, Eq)]
pub struct Window {
    pub folder: PathBuf,
    pub select: Vec<PathBuf>,
}

pub struct FileManager {
    conn: Connection,
}

impl FileManager {
    pub fn new(conn: Connection) -> FileManager {
        return FileManager { conn };
    }

    /// Handles one event. Returns false when it's time to exit.
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Incoming(msg) => {
                if msg.kind == Kind::MethodCall {
                    self.on_call(msg);
                }
                return true;
            }
            Event::Disconnected(reason) => {
                eprintln!("xdg-desktop-portal-itamiiru: bus connection lost: {reason}");
                return false;
            }
            // Only the portal starts pickers.
            Event::PickDone { .. } => return true,
        }
    }

    fn send(&mut self, msg: Message) {
        if let Err(err) = self.conn.send(&msg) {
            eprintln!("xdg-desktop-portal-itamiiru: can't send a message: {err}");
        }
    }

    fn reply(&mut self, call: &Message, body: Vec<Value>) {
        if call.wants_reply() {
            self.send(Message::method_return(call, body));
        }
    }

    fn reply_error(&mut self, call: &Message, name: &str, text: &str) {
        if call.wants_reply() {
            self.send(Message::error(call, name, text));
        }
    }

    fn on_call(&mut self, call: Message) {
        let path = call.path.clone().unwrap_or_default();
        let interface = call.interface.clone().unwrap_or_default();
        let member = call.member.clone().unwrap_or_default();
        itamiiru_core::debug!("file manager: call {interface}.{member} on {path}");
        if path != OBJECT_PATH {
            self.reply_error(
                &call,
                "org.freedesktop.DBus.Error.UnknownObject",
                "no such object",
            );
            return;
        }
        match (interface.as_str(), member.as_str()) {
            (INTERFACE, "ShowFolders") => self.show(call, false),
            // There's no properties dialog yet; showing the item is the
            // closest thing.
            (INTERFACE, "ShowItems") | (INTERFACE, "ShowItemProperties") => self.show(call, true),
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                self.reply(&call, vec![Value::Str(INTROSPECTION.to_string())]);
            }
            ("org.freedesktop.DBus.Peer", "Ping") => self.reply(&call, Vec::new()),
            _ => self.reply_error(
                &call,
                "org.freedesktop.DBus.Error.UnknownMethod",
                "no such method",
            ),
        }
    }

    /// Replies, then opens the windows.
    fn show(&mut self, call: Message, items: bool) {
        if call.signature() != "ass" {
            self.reply_error(
                &call,
                "org.freedesktop.DBus.Error.InvalidArgs",
                "expected (ass)",
            );
            return;
        }
        let mut uris = Vec::new();
        if let Value::Array(_, values) = &call.body[0] {
            for value in values {
                if let Some(uri) = value.as_str() {
                    uris.push(uri.to_string());
                }
            }
        }
        let startup_id = call.body[1].as_str().unwrap_or("").to_string();
        self.reply(&call, Vec::new());
        for window in windows(&uris, items) {
            open_window(&window, &startup_id);
        }
    }
}

/// The windows to open for a call. ShowFolders opens each folder;
/// ShowItems opens each item's folder once, with its items selected.
/// URIs that aren't local `file://` URIs are skipped.
pub fn windows(uris: &[String], items: bool) -> Vec<Window> {
    if !items {
        let mut out = Vec::new();
        for uri in uris {
            if out.len() >= MAX_WINDOWS {
                break;
            }
            if let Some(folder) = itamiiru_core::percent::file_uri_to_path(uri) {
                out.push(Window {
                    folder,
                    select: Vec::new(),
                });
            }
        }
        return out;
    }
    // A sorted map keeps the windows in a stable order.
    let mut by_folder: BTreeMap<PathBuf, Vec<PathBuf>> = BTreeMap::new();
    for uri in uris {
        let Some(item) = itamiiru_core::percent::file_uri_to_path(uri) else {
            continue;
        };
        // "/" has no folder above it; show it as a folder.
        let Some(parent) = item.parent() else {
            by_folder.entry(item).or_default();
            continue;
        };
        let parent = parent.to_path_buf();
        if !by_folder.contains_key(&parent) && by_folder.len() >= MAX_WINDOWS {
            continue;
        }
        let selected = by_folder.entry(parent).or_default();
        if selected.len() < MAX_ITEMS {
            selected.push(item);
        }
    }
    let mut out = Vec::new();
    for (folder, select) in by_folder {
        out.push(Window { folder, select });
    }
    return out;
}

/// Starts `itami FOLDER --select ITEM...`, without waiting for it to close.
fn open_window(window: &Window, startup_id: &str) {
    let mut command = Command::new(picker_binary());
    command.arg(&window.folder);
    for item in &window.select {
        command.arg("--select").arg(item);
    }
    // Lets the window manager give the new window focus.
    if !startup_id.is_empty() {
        command.env("DESKTOP_STARTUP_ID", startup_id);
        command.env("XDG_ACTIVATION_TOKEN", startup_id);
    }
    match command.spawn() {
        Ok(mut child) => {
            // Someone has to wait() for it, or it lingers as a zombie
            // after it exits.
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
        Err(err) => eprintln!("xdg-desktop-portal-itamiiru: can't start itami: {err}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(items: &[&str]) -> Vec<String> {
        let mut out = Vec::new();
        for item in items {
            out.push(item.to_string());
        }
        return out;
    }

    #[test]
    fn show_folders() {
        let uris = strings(&[
            "file:///home/me/a%20b",
            "https://example.com/",
            "file:///tmp",
        ]);
        assert_eq!(
            windows(&uris, false),
            vec![
                Window {
                    folder: PathBuf::from("/home/me/a b"),
                    select: Vec::new(),
                },
                Window {
                    folder: PathBuf::from("/tmp"),
                    select: Vec::new(),
                },
            ]
        );
    }

    #[test]
    fn show_items_groups_by_folder() {
        let uris = strings(&[
            "file:///home/me/Downloads/b.zip",
            "file:///etc/hosts",
            "file:///home/me/Downloads/a.pdf",
            "not a uri",
        ]);
        assert_eq!(
            windows(&uris, true),
            vec![
                Window {
                    folder: PathBuf::from("/etc"),
                    select: vec![PathBuf::from("/etc/hosts")],
                },
                Window {
                    folder: PathBuf::from("/home/me/Downloads"),
                    select: vec![
                        PathBuf::from("/home/me/Downloads/b.zip"),
                        PathBuf::from("/home/me/Downloads/a.pdf"),
                    ],
                },
            ]
        );
    }

    #[test]
    fn root_is_shown_as_a_folder() {
        let uris = strings(&["file:///"]);
        assert_eq!(
            windows(&uris, true),
            vec![Window {
                folder: PathBuf::from("/"),
                select: Vec::new(),
            }]
        );
    }

    #[test]
    fn floods_are_capped() {
        let mut uris = Vec::new();
        for i in 0..500 {
            uris.push(format!("file:///folder{i}/item"));
            uris.push(format!("file:///same/item{i}"));
        }
        let shown = windows(&uris, true);
        assert_eq!(shown.len(), MAX_WINDOWS);
        assert_eq!(windows(&uris, false).len(), MAX_WINDOWS);

        let mut many = Vec::new();
        for i in 0..5000 {
            many.push(format!("file:///same/item{i}"));
        }
        let shown = windows(&many, true);
        assert_eq!(shown[0].select.len(), MAX_ITEMS);
    }
}
