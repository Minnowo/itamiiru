//! The FileChooser backend: turns portal calls into picker
//! requests, runs `itami --pick`, and turns its answer into the reply.
//! Interface: org.freedesktop.impl.portal.FileChooser.

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::Sender;

use itamiiru_core::protocol::{self, Choice, Filter, Mode, Pattern, Request};

use itamiiru_core::platform::dbus::conn::Connection;
use itamiiru_core::platform::dbus::msg::{Kind, Message};
use itamiiru_core::platform::dbus::wire::{Type, Value};

pub const BUS_NAME: &str = "org.freedesktop.impl.portal.desktop.itamiiru";
pub const OBJECT_PATH: &str = "/org/freedesktop/portal/desktop";
const FILE_CHOOSER: &str = "org.freedesktop.impl.portal.FileChooser";
const REQUEST: &str = "org.freedesktop.impl.portal.Request";
/// The picker's answer is a few lines; more than this is refused.
const MAX_PICKER_OUTPUT: u64 = 1024 * 1024;

/// Portal response codes.
const RESPONSE_OK: u32 = 0;
const RESPONSE_CANCELLED: u32 = 1;
const RESPONSE_OTHER: u32 = 2;

const INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.impl.portal.FileChooser">
    <method name="OpenFile">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFile">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
    <method name="SaveFiles">
      <arg type="o" name="handle" direction="in"/>
      <arg type="s" name="app_id" direction="in"/>
      <arg type="s" name="parent_window" direction="in"/>
      <arg type="s" name="title" direction="in"/>
      <arg type="a{sv}" name="options" direction="in"/>
      <arg type="u" name="response" direction="out"/>
      <arg type="a{sv}" name="results" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect">
      <arg type="s" name="xml_data" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
    <method name="GetMachineId">
      <arg type="s" name="machine_uuid" direction="out"/>
    </method>
  </interface>
  <interface name="org.freedesktop.DBus.Properties">
    <method name="Get">
      <arg type="s" name="interface_name" direction="in"/>
      <arg type="s" name="property_name" direction="in"/>
      <arg type="v" name="value" direction="out"/>
    </method>
    <method name="GetAll">
      <arg type="s" name="interface_name" direction="in"/>
      <arg type="a{sv}" name="props" direction="out"/>
    </method>
  </interface>
</node>
"#;

pub enum Event {
    Incoming(Message),
    /// The bus connection broke; time to exit.
    Disconnected(String),
    /// A picker closed its stdout (it answered or exited).
    PickDone {
        handle: String,
        output: Vec<u8>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Method {
    OpenFile,
    SaveFile,
    SaveFiles,
}

/// A portal call turned into a picker request, plus what's needed to
/// build the reply.
#[derive(Debug)]
pub struct Translated {
    pub request: Request,
    /// The filters as the app sent them, to echo back as current_filter.
    pub filters: Vec<Value>,
    /// SaveFiles: the file names to add to the chosen folder.
    pub files: Vec<Vec<u8>>,
    pub has_choices: bool,
}

/// A picker that's open, waiting for its answer.
struct Pick {
    call: Message,
    method: Method,
    child: Child,
    translated: Translated,
}

pub struct Portal {
    conn: Connection,
    picks: HashMap<String, Pick>,
    events: Sender<Event>,
}

impl Portal {
    pub fn new(conn: Connection, events: Sender<Event>) -> Portal {
        return Portal {
            conn,
            picks: HashMap::new(),
            events,
        };
    }

    /// Handles one event. Returns false when the portal should exit.
    pub fn handle(&mut self, event: Event) -> bool {
        match event {
            Event::Incoming(msg) => {
                if msg.kind == Kind::MethodCall {
                    self.on_call(msg);
                }
                return true;
            }
            Event::PickDone { handle, output } => {
                self.finish_pick(&handle, &output);
                return true;
            }
            Event::Disconnected(reason) => {
                eprintln!("xdg-desktop-portal-itamiiru: bus connection lost: {reason}");
                return false;
            }
        }
    }

    /// Closes any open pickers.
    pub fn shutdown(&mut self) {
        for (_, mut pick) in self.picks.drain() {
            let _ = pick.child.kill();
            let _ = pick.child.wait();
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
        itamiiru_core::debug!("portal: call {interface}.{member} on {path}");

        // A handle object's Close: the app (or the frontend) gave up.
        if interface == REQUEST && member == "Close" {
            self.close_pick(&path);
            self.reply(&call, Vec::new());
            return;
        }
        if path != OBJECT_PATH {
            self.reply_error(
                &call,
                "org.freedesktop.DBus.Error.UnknownObject",
                "no such object",
            );
            return;
        }
        match (interface.as_str(), member.as_str()) {
            (FILE_CHOOSER, "OpenFile") => self.start_pick(call, Method::OpenFile),
            (FILE_CHOOSER, "SaveFile") => self.start_pick(call, Method::SaveFile),
            (FILE_CHOOSER, "SaveFiles") => self.start_pick(call, Method::SaveFiles),
            ("org.freedesktop.DBus.Introspectable", "Introspect") => {
                self.reply(&call, vec![Value::Str(INTROSPECTION.to_string())]);
            }
            ("org.freedesktop.DBus.Peer", "Ping") => self.reply(&call, Vec::new()),
            ("org.freedesktop.DBus.Peer", "GetMachineId") => {
                let id = std::fs::read_to_string("/etc/machine-id").unwrap_or_default();
                self.reply(&call, vec![Value::Str(id.trim().to_string())]);
            }
            // The impl FileChooser interface has no properties.
            ("org.freedesktop.DBus.Properties", "GetAll") => {
                self.reply(&call, vec![Value::dict(Vec::new())])
            }
            ("org.freedesktop.DBus.Properties", "Get")
            | ("org.freedesktop.DBus.Properties", "Set") => {
                self.reply_error(
                    &call,
                    "org.freedesktop.DBus.Error.UnknownProperty",
                    "no such property",
                );
            }
            _ => self.reply_error(
                &call,
                "org.freedesktop.DBus.Error.UnknownMethod",
                "no such method",
            ),
        }
    }

    /// Starts a picker for a FileChooser call. The reply is sent later,
    /// when the picker answers (D-Bus allows a delayed reply).
    fn start_pick(&mut self, call: Message, method: Method) {
        let body = &call.body;
        if call.signature() != "osssa{sv}" {
            self.reply_error(
                &call,
                "org.freedesktop.DBus.Error.InvalidArgs",
                "expected (osssa{sv})",
            );
            return;
        }
        let handle = body[0].as_str().unwrap_or("").to_string();
        let app_id = body[1].as_str().unwrap_or("");
        let parent = body[2].as_str().unwrap_or("");
        let title = body[3].as_str().unwrap_or("");
        let translated = translate(method, app_id, parent, title, &body[4]);
        let text = protocol::write_request(&translated.request);

        let child = match spawn_picker(&text, &handle, self.events.clone()) {
            Ok(child) => child,
            Err(err) => {
                eprintln!("xdg-desktop-portal-itamiiru: can't start the picker: {err}");
                self.reply(
                    &call,
                    vec![Value::U32(RESPONSE_OTHER), Value::dict(Vec::new())],
                );
                return;
            }
        };
        self.picks.insert(
            handle,
            Pick {
                call,
                method,
                child,
                translated,
            },
        );
    }

    fn finish_pick(&mut self, handle: &str, output: &[u8]) {
        let Some(mut pick) = self.picks.remove(handle) else {
            // Already closed.
            return;
        };
        let exit_code = match pick.child.wait() {
            Ok(status) => status.code(),
            Err(_) => None,
        };
        let (response, results) = results(pick.method, &pick.translated, exit_code, output);
        itamiiru_core::debug!("portal: {handle} answered {response}");
        self.reply(&pick.call, vec![Value::U32(response), results]);
    }

    fn close_pick(&mut self, handle: &str) {
        let Some(mut pick) = self.picks.remove(handle) else {
            return;
        };
        let _ = pick.child.kill();
        let _ = pick.child.wait();
        self.reply(
            &pick.call,
            vec![Value::U32(RESPONSE_OTHER), Value::dict(Vec::new())],
        );
    }
}

/// The picker binary: `$ITAMI_BIN`, else `itami` next to this program,
/// else `itami` on `PATH`.
pub fn picker_binary() -> PathBuf {
    if let Some(bin) = std::env::var_os("ITAMI_BIN") {
        return PathBuf::from(bin);
    }
    if let Ok(exe) = std::env::current_exe()
        && let Some(dir) = exe.parent()
    {
        let next_to_us = dir.join("itami");
        if next_to_us.is_file() {
            return next_to_us;
        }
    }
    return PathBuf::from("itami");
}

/// Starts `itami --pick`. A thread writes the request to its stdin, reads
/// its stdout to the end, and reports back with PickDone.
fn spawn_picker(request: &str, handle: &str, events: Sender<Event>) -> std::io::Result<Child> {
    let mut child = Command::new(picker_binary())
        .arg("--pick")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()?;
    let stdin = child.stdin.take();
    let stdout = child.stdout.take();
    let request = request.to_string();
    let handle = handle.to_string();
    std::thread::spawn(move || {
        if let Some(mut stdin) = stdin {
            let _ = stdin.write_all(request.as_bytes());
            // Dropping stdin closes it, so the picker sees the end.
        }
        let mut output = Vec::new();
        if let Some(stdout) = stdout {
            let _ = stdout.take(MAX_PICKER_OUTPUT).read_to_end(&mut output);
        }
        let _ = events.send(Event::PickDone { handle, output });
    });
    return Ok(child);
}

/// An `ay` path: drops trailing NULs, keeps it only if absolute.
fn path_from_ay(value: &Value) -> Option<PathBuf> {
    let mut bytes = value.as_bytes()?;
    while bytes.last() == Some(&0) {
        bytes.pop();
    }
    let path = itamiiru_core::platform::path_from_bytes(bytes)?;
    if !path.is_absolute() {
        return None;
    }
    return Some(path);
}

/// A `(sa(us))` filter; kind 0 is a glob, 1 a MIME type.
fn filter_from_value(value: &Value) -> Option<Filter> {
    let fields = value.as_struct()?;
    let name = fields.first()?.as_str()?.to_string();
    let mut patterns = Vec::new();
    for item in fields.get(1)?.as_array()? {
        let pair = item.as_struct()?;
        let kind = pair.first()?.as_u32()?;
        let text = pair.get(1)?.as_str()?.to_string();
        match kind {
            0 => patterns.push(Pattern::Glob(text)),
            1 => patterns.push(Pattern::Mime(text)),
            _ => {}
        }
    }
    if patterns.is_empty() {
        return None;
    }
    return Some(Filter { name, patterns });
}

/// A `(ssa(ss)s)` choice: id, label, options, default.
fn choice_from_value(value: &Value) -> Option<Choice> {
    let fields = value.as_struct()?;
    let mut options = Vec::new();
    for item in fields.get(2)?.as_array()? {
        let pair = item.as_struct()?;
        options.push((
            pair.first()?.as_str()?.to_string(),
            pair.get(1)?.as_str()?.to_string(),
        ));
    }
    return Some(Choice {
        id: fields.first()?.as_str()?.to_string(),
        label: fields.get(1)?.as_str()?.to_string(),
        default: fields.get(3)?.as_str()?.to_string(),
        options,
    });
}

/// Turns a FileChooser call's arguments into a picker request.
/// Options of the wrong type are ignored rather than refused.
pub fn translate(
    method: Method,
    app_id: &str,
    parent: &str,
    title: &str,
    options: &Value,
) -> Translated {
    let directory = options
        .dict_get("directory")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let mode = match method {
        Method::OpenFile if directory => Mode::Folder,
        Method::OpenFile => Mode::Open,
        Method::SaveFile => Mode::Save,
        Method::SaveFiles => Mode::SaveFiles,
    };
    let mut request = Request::new(mode);
    request.title = title.to_string();
    request.app_id = app_id.to_string();
    request.parent = parent.to_string();
    request.accept_label = options
        .dict_get("accept_label")
        .and_then(Value::as_str)
        .map(String::from);
    request.modal = options
        .dict_get("modal")
        .and_then(Value::as_bool)
        .unwrap_or(true);
    request.multiple = options
        .dict_get("multiple")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    request.folder = options.dict_get("current_folder").and_then(path_from_ay);
    request.file = options.dict_get("current_file").and_then(path_from_ay);
    request.name = options
        .dict_get("current_name")
        .and_then(Value::as_str)
        .map(String::from);

    let mut filters = Vec::new();
    for value in options
        .dict_get("filters")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        if let Some(filter) = filter_from_value(value) {
            request.filters.push(filter);
            filters.push(value.clone());
        }
    }
    // current_filter should be one of the filters; if it isn't (or there
    // are none), it's added so it can be selected.
    if let Some(current) = options.dict_get("current_filter") {
        let mut index = None;
        for (i, value) in filters.iter().enumerate() {
            if value == current {
                index = Some(i);
            }
        }
        if index.is_none()
            && let Some(filter) = filter_from_value(current)
        {
            request.filters.push(filter);
            filters.push(current.clone());
            index = Some(filters.len() - 1);
        }
        request.current_filter = index;
    }

    let mut has_choices = false;
    for value in options
        .dict_get("choices")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        if let Some(choice) = choice_from_value(value) {
            request.choices.push(choice);
            has_choices = true;
        }
    }

    let mut files = Vec::new();
    for value in options
        .dict_get("files")
        .and_then(Value::as_array)
        .unwrap_or(&[])
    {
        if let Some(mut name) = value.as_bytes() {
            while name.last() == Some(&0) {
                name.pop();
            }
            if !name.is_empty() {
                files.push(name);
            }
        }
    }
    return Translated {
        request,
        filters,
        files,
        has_choices,
    };
}

/// The reply for a finished picker: (response code, results).
pub fn results(
    method: Method,
    translated: &Translated,
    exit_code: Option<i32>,
    output: &[u8],
) -> (u32, Value) {
    let empty = Value::dict(Vec::new());
    match exit_code {
        Some(0) => {}
        Some(1) => return (RESPONSE_CANCELLED, empty),
        _ => return (RESPONSE_OTHER, empty),
    }
    let text = String::from_utf8_lossy(output);
    let response = match protocol::parse_response(&text) {
        Ok(response) => response,
        Err(err) => {
            eprintln!("xdg-desktop-portal-itamiiru: bad picker answer: {err}");
            return (RESPONSE_OTHER, empty);
        }
    };

    let mut uris = Vec::new();
    if method == Method::SaveFiles {
        // The picker chose a folder; each file name goes inside it.
        let Some(folder) = response
            .uris
            .first()
            .and_then(|uri| itamiiru_core::percent::file_uri_to_path(uri))
        else {
            return (RESPONSE_OTHER, empty);
        };
        for name in &translated.files {
            let Some(name_path) = itamiiru_core::platform::path_from_bytes(name.clone()) else {
                continue;
            };
            // Only the last component: a name can't point elsewhere.
            let Some(file_name) = name_path.file_name() else {
                continue;
            };
            uris.push(itamiiru_core::percent::path_to_file_uri(
                &folder.join(file_name),
            ));
        }
    } else {
        uris = response.uris.clone();
    }
    if uris.is_empty() {
        return (RESPONSE_OTHER, empty);
    }

    let mut pairs = vec![("uris", Value::strings(uris))];
    if translated.has_choices {
        let pair_type = Type::Struct(vec![Type::Str, Type::Str]);
        let mut items = Vec::new();
        for (id, value) in &response.choices {
            items.push(Value::Struct(vec![
                Value::Str(id.clone()),
                Value::Str(value.clone()),
            ]));
        }
        pairs.push(("choices", Value::Array(pair_type, items)));
    }
    if let Some(index) = response.filter
        && let Some(filter) = translated.filters.get(index)
    {
        pairs.push(("current_filter", filter.clone()));
    }
    return (RESPONSE_OK, Value::dict(pairs));
}

#[cfg(test)]
mod tests {
    use super::*;

    fn filter(name: &str, globs: &[(u32, &str)]) -> Value {
        let mut items = Vec::new();
        for (kind, pattern) in globs {
            items.push(Value::Struct(vec![
                Value::U32(*kind),
                Value::Str(pattern.to_string()),
            ]));
        }
        return Value::Struct(vec![
            Value::Str(name.to_string()),
            Value::Array(Type::Struct(vec![Type::U32, Type::Str]), items),
        ]);
    }

    fn filters_value(filters: Vec<Value>) -> Value {
        let element = filters[0].value_type();
        return Value::Array(element, filters);
    }

    #[test]
    fn open_file_options() {
        let images = filter("Images", &[(0, "*.png"), (1, "image/jpeg")]);
        let all = filter("All", &[(0, "*")]);
        let choice = Value::Struct(vec![
            Value::Str("enc".to_string()),
            Value::Str("Encoding".to_string()),
            Value::Array(
                Type::Struct(vec![Type::Str, Type::Str]),
                vec![Value::Struct(vec![
                    Value::Str("utf8".to_string()),
                    Value::Str("UTF-8".to_string()),
                ])],
            ),
            Value::Str("utf8".to_string()),
        ]);
        let options = Value::dict(vec![
            ("multiple", Value::Bool(true)),
            ("accept_label", Value::Str("Pick".to_string())),
            ("filters", filters_value(vec![images.clone(), all])),
            ("current_filter", images),
            ("current_folder", Value::bytes(b"/tmp\0")),
            ("choices", Value::Array(choice.value_type(), vec![choice])),
        ]);
        let t = translate(Method::OpenFile, "org.app", "x11:1", "Open it", &options);
        assert_eq!(t.request.mode, Mode::Open);
        assert!(t.request.multiple);
        assert_eq!(t.request.title, "Open it");
        assert_eq!(t.request.accept_label.as_deref(), Some("Pick"));
        assert_eq!(t.request.folder, Some(PathBuf::from("/tmp")));
        assert_eq!(t.request.filters.len(), 2);
        assert_eq!(
            t.request.filters[0].patterns[1],
            Pattern::Mime("image/jpeg".to_string())
        );
        assert_eq!(t.request.current_filter, Some(0));
        assert_eq!(t.request.choices[0].default, "utf8");
        assert!(t.has_choices);
        // It survives the trip to the picker.
        let text = protocol::write_request(&t.request);
        assert_eq!(protocol::parse_request(&text).unwrap(), t.request);
    }

    #[test]
    fn directory_and_unknown_current_filter() {
        let options = Value::dict(vec![
            ("directory", Value::Bool(true)),
            ("current_filter", filter("Text", &[(0, "*.txt")])),
            ("current_folder", Value::bytes(b"relative\0")),
        ]);
        let t = translate(Method::OpenFile, "", "", "", &options);
        assert_eq!(t.request.mode, Mode::Folder);
        assert_eq!(t.request.filters.len(), 1, "a lone current_filter is added");
        assert_eq!(t.request.current_filter, Some(0));
        assert_eq!(t.request.folder, None, "relative folders are ignored");
    }

    #[test]
    fn replies() {
        let options = Value::dict(vec![(
            "filters",
            filters_value(vec![filter("Text", &[(0, "*.txt")])]),
        )]);
        let t = translate(Method::OpenFile, "", "", "", &options);
        let output = b"itamiiru-protocol 1\nuri file:///tmp/a.txt\nfilter 0\n";
        let (code, results) = results(Method::OpenFile, &t, Some(0), output);
        assert_eq!(code, RESPONSE_OK);
        let uris = results.dict_get("uris").and_then(Value::as_array).unwrap();
        assert_eq!(uris[0].as_str(), Some("file:///tmp/a.txt"));
        assert_eq!(results.dict_get("current_filter"), Some(&t.filters[0]));
        assert_eq!(
            super::results(Method::OpenFile, &t, Some(1), b"").0,
            RESPONSE_CANCELLED
        );
        assert_eq!(
            super::results(Method::OpenFile, &t, Some(2), b"").0,
            RESPONSE_OTHER
        );
        assert_eq!(
            super::results(Method::OpenFile, &t, None, b"").0,
            RESPONSE_OTHER
        );
        assert_eq!(
            super::results(Method::OpenFile, &t, Some(0), b"garbage").0,
            RESPONSE_OTHER
        );
    }

    #[test]
    fn save_files_joins_names() {
        let files = Value::Array(
            Type::Array(Box::new(Type::Byte)),
            vec![
                Value::bytes(b"a.txt\0"),
                Value::bytes(b"../evil.txt\0"),
                Value::bytes(b"\0"),
            ],
        );
        let options = Value::dict(vec![("files", files)]);
        let t = translate(Method::SaveFiles, "", "", "", &options);
        assert_eq!(t.request.mode, Mode::SaveFiles);
        assert_eq!(t.files.len(), 2, "empty names are dropped");
        let output = b"itamiiru-protocol 1\nuri file:///home/me/out\n";
        let (code, results) = results(Method::SaveFiles, &t, Some(0), output);
        assert_eq!(code, RESPONSE_OK);
        let uris = results.dict_get("uris").and_then(Value::as_array).unwrap();
        assert_eq!(uris[0].as_str(), Some("file:///home/me/out/a.txt"));
        assert_eq!(
            uris[1].as_str(),
            Some("file:///home/me/out/evil.txt"),
            "only the file name is used"
        );
    }
}
