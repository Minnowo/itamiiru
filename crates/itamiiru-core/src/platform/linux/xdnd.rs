//! Dragging files out of itamiiru into other apps on X11, with the XDND
//! protocol (PLAN.md D24): https://freedesktop.org/wiki/Specifications/XDND/
//!
//! The drag runs on its own X connection, in its own thread, from its own
//! hidden window: drop targets answer the window named in our messages,
//! so winit's window and connection are never touched. Without grabbing
//! the pointer (winit holds it while the button is down), the thread asks
//! the server where the pointer is about every 10 ms, finds the window
//! under it that accepts drops, and sends it XDND messages. On release it
//! drops, and the target fetches the paths as `text/uri-list` (or plain
//! text) through the XdndSelection selection, which we own.
//!
//! Only copying is offered: the target decides what to do with the paths
//! (upload, open, paste them), and nothing here deletes anything.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    self, Atom, AtomEnum, ClientMessageEvent, ConnectionExt as _, CreateWindowAux, EventMask,
    KeyButMask, PropMode, SelectionNotifyEvent, SelectionRequestEvent, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;

use crate::platform::{DragOut, DragState};

/// The XDND version we speak; targets may speak an older one (3 or up).
const VERSION: u32 = 5;
/// How often the pointer is checked.
const POLL: Duration = Duration::from_millis(10);
/// After the drop, how long the target gets to fetch the data and say
/// it's finished.
const AFTER_DROP: Duration = Duration::from_secs(10);
/// A drag can't last longer than this (a lost button release).
const MAX_DRAG: Duration = Duration::from_secs(300);
/// A target that doesn't answer a position within this gets another.
const STATUS_WAIT: Duration = Duration::from_millis(100);
/// How deep to look under the pointer for a window that accepts drops.
const MAX_DEPTH: usize = 10;

/// Starts dragging `paths` out of the window, on a new thread. Call it
/// while the mouse button is still down and the pointer has left the
/// window. Fails straight away when there's no X11 display to drag on.
/// The returned handle shows where the drag stands and can cancel it.
pub fn start(paths: Vec<PathBuf>) -> Result<DragOut, String> {
    let handle = DragOut::new();
    if paths.is_empty() {
        handle.set_state(DragState::Ended);
        return Ok(handle);
    }
    // winit uses Wayland when it can; then this window isn't on X11.
    if std::env::var_os("WAYLAND_DISPLAY").is_some() {
        return Err("dragging out needs an X11 session".to_string());
    }
    if std::env::var_os("DISPLAY").is_none() {
        return Err("no X11 display".to_string());
    }
    let shared = handle.clone();
    let spawned = std::thread::Builder::new()
        .name("xdnd".to_string())
        .spawn(move || {
            if let Err(err) = run(&paths, &shared) {
                crate::debug!("xdnd: drag failed: {err}");
            }
            shared.set_state(DragState::Ended);
        });
    if let Err(err) = spawned {
        return Err(format!("can't start the drag: {err}"));
    }
    return Ok(handle);
}

/// The atoms XDND uses, looked up once per drag.
struct Atoms {
    aware: Atom,
    proxy: Atom,
    enter: Atom,
    position: Atom,
    status: Atom,
    leave: Atom,
    drop: Atom,
    finished: Atom,
    selection: Atom,
    action_copy: Atom,
    uri_list: Atom,
    utf8_string: Atom,
    text_plain: Atom,
    targets: Atom,
}

impl Atoms {
    fn intern(conn: &RustConnection) -> Result<Atoms, String> {
        let get = |name: &[u8]| -> Result<Atom, String> {
            let cookie = conn.intern_atom(false, name).map_err(|e| e.to_string())?;
            let reply = cookie.reply().map_err(|e| e.to_string())?;
            return Ok(reply.atom);
        };
        return Ok(Atoms {
            aware: get(b"XdndAware")?,
            proxy: get(b"XdndProxy")?,
            enter: get(b"XdndEnter")?,
            position: get(b"XdndPosition")?,
            status: get(b"XdndStatus")?,
            leave: get(b"XdndLeave")?,
            drop: get(b"XdndDrop")?,
            finished: get(b"XdndFinished")?,
            selection: get(b"XdndSelection")?,
            action_copy: get(b"XdndActionCopy")?,
            uri_list: get(b"text/uri-list")?,
            utf8_string: get(b"UTF8_STRING")?,
            text_plain: get(b"text/plain;charset=utf-8")?,
            targets: get(b"TARGETS")?,
        });
    }

    /// The data types offered, best first.
    fn offered(&self) -> [Atom; 3] {
        return [self.uri_list, self.utf8_string, self.text_plain];
    }
}

/// A window that accepts drops: where messages go (itself, or its
/// XdndProxy), and the XDND version both sides speak.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Target {
    window: Window,
    send_to: Window,
    version: u32,
}

/// One drag, from our hidden window.
struct Session<'a> {
    conn: &'a RustConnection,
    atoms: Atoms,
    source: Window,
    uri_list: Vec<u8>,
    text: Vec<u8>,
    target: Option<Target>,
    /// The target's last XdndStatus said it would take the drop.
    accepted: bool,
    /// When the last position went out without an answer yet.
    waiting_since: Option<Instant>,
    finished: bool,
}

fn run(paths: &[PathBuf], handle: &DragOut) -> Result<(), String> {
    let (conn, screen) = RustConnection::connect(None).map_err(|e| e.to_string())?;
    let root = conn.setup().roots[screen].root;
    let atoms = Atoms::intern(&conn)?;
    let source = conn.generate_id().map_err(|e| e.to_string())?;
    conn.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        source,
        root,
        -10,
        -10,
        1,
        1,
        0,
        WindowClass::INPUT_ONLY,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new(),
    )
    .map_err(|e| e.to_string())?;
    let mut session = Session::new(&conn, atoms, source, paths)?;
    let result = session.follow_pointer(root, handle);
    // Leaving the selection and the window lets the server clean up.
    let _ = conn.destroy_window(source);
    let _ = conn.flush();
    return result;
}

impl<'a> Session<'a> {
    fn new(
        conn: &'a RustConnection,
        atoms: Atoms,
        source: Window,
        paths: &[PathBuf],
    ) -> Result<Session<'a>, String> {
        conn.set_selection_owner(source, atoms.selection, x11rb::CURRENT_TIME)
            .map_err(|e| e.to_string())?;
        conn.flush().map_err(|e| e.to_string())?;
        return Ok(Session {
            conn,
            atoms,
            source,
            uri_list: uri_list(paths).into_bytes(),
            text: plain_text(paths).into_bytes(),
            target: None,
            accepted: false,
            waiting_since: None,
            finished: false,
        });
    }

    /// The drag itself: follow the pointer until the button comes up,
    /// then drop (or leave), and wait for the target to finish. `handle`
    /// gets the state as it changes, and can cancel (Escape).
    fn follow_pointer(&mut self, root: Window, handle: &DragOut) -> Result<(), String> {
        let started = Instant::now();
        let mut last_position = None;
        let mut first = true;
        loop {
            self.handle_events()?;
            handle.set_state(self.state());
            if started.elapsed() > MAX_DRAG || handle.is_cancelled() {
                self.leave()?;
                return Ok(());
            }
            let pointer = self
                .conn
                .query_pointer(root)
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?;
            let down = u16::from(pointer.mask) & u16::from(KeyButMask::BUTTON1) != 0;
            if first && !down {
                // Released before the drag got going: nothing to do.
                return Ok(());
            }
            first = false;
            let target = self.find_target(root, pointer.child)?;
            if target != self.target {
                self.leave()?;
                if let Some(target) = target {
                    self.enter(target)?;
                }
                last_position = None;
            }
            if !down {
                break;
            }
            let position = (pointer.root_x, pointer.root_y);
            let stale = self
                .waiting_since
                .is_some_and(|since| since.elapsed() > STATUS_WAIT);
            if self.target.is_some()
                && last_position != Some(position)
                && (self.waiting_since.is_none() || stale)
            {
                self.position(position.0, position.1)?;
                last_position = Some(position);
            }
            std::thread::sleep(POLL);
        }
        // Button up: drop on a target that said yes, otherwise just leave.
        if self.target.is_none() || !self.accepted {
            self.leave()?;
            return Ok(());
        }
        self.drop_here()?;
        let dropped = Instant::now();
        while !self.finished && dropped.elapsed() < AFTER_DROP {
            self.handle_events()?;
            std::thread::sleep(POLL);
        }
        return Ok(());
    }

    /// Where the drag stands, for the app's cursor.
    fn state(&self) -> DragState {
        if self.target.is_none() {
            return DragState::Nothing;
        }
        if self.accepted {
            return DragState::Accepted;
        }
        return DragState::Refused;
    }

    /// The window under the pointer that accepts drops: down from the
    /// top-level window under it, the first with XdndAware.
    fn find_target(&self, root: Window, top: Window) -> Result<Option<Target>, String> {
        let mut window = top;
        for _ in 0..MAX_DEPTH {
            if window == x11rb::NONE || window == self.source {
                return Ok(None);
            }
            if let Some(version) = self.property_u32(window, self.atoms.aware, AtomEnum::ATOM)?
                && version >= 3
            {
                // A proxy gets the messages; they still name the target.
                let send_to = self
                    .property_u32(window, self.atoms.proxy, AtomEnum::WINDOW)?
                    .unwrap_or(window);
                return Ok(Some(Target {
                    window,
                    send_to,
                    version: version.min(VERSION),
                }));
            }
            let below = self
                .conn
                .query_pointer(window)
                .map_err(|e| e.to_string())?
                .reply()
                .map_err(|e| e.to_string())?;
            if below.child == window || window == root {
                return Ok(None);
            }
            window = below.child;
        }
        return Ok(None);
    }

    /// The first 32-bit value of a property, if the window has it.
    fn property_u32(
        &self,
        window: Window,
        property: Atom,
        kind: AtomEnum,
    ) -> Result<Option<u32>, String> {
        let reply = self
            .conn
            .get_property(false, window, property, kind, 0, 1)
            .map_err(|e| e.to_string())?
            .reply();
        // The window may be gone already; that's just "no".
        let Ok(reply) = reply else {
            return Ok(None);
        };
        return Ok(reply.value32().and_then(|mut values| values.next()));
    }

    fn send(&self, target: Target, kind: Atom, data: [u32; 5]) -> Result<(), String> {
        let event = ClientMessageEvent::new(32, target.window, kind, data);
        self.conn
            .send_event(false, target.send_to, EventMask::NO_EVENT, event)
            .map_err(|e| e.to_string())?;
        self.conn.flush().map_err(|e| e.to_string())?;
        return Ok(());
    }

    fn enter(&mut self, target: Target) -> Result<(), String> {
        let data = enter_data(self.source, target.version, self.atoms.offered());
        self.send(target, self.atoms.enter, data)?;
        self.target = Some(target);
        self.accepted = false;
        self.waiting_since = None;
        return Ok(());
    }

    fn position(&mut self, x: i16, y: i16) -> Result<(), String> {
        let Some(target) = self.target else {
            return Ok(());
        };
        let data = position_data(self.source, x, y, self.atoms.action_copy);
        self.send(target, self.atoms.position, data)?;
        self.waiting_since = Some(Instant::now());
        return Ok(());
    }

    fn leave(&mut self) -> Result<(), String> {
        if let Some(target) = self.target.take() {
            self.send(target, self.atoms.leave, [self.source, 0, 0, 0, 0])?;
        }
        self.accepted = false;
        self.waiting_since = None;
        return Ok(());
    }

    fn drop_here(&mut self) -> Result<(), String> {
        if let Some(target) = self.target {
            let time = x11rb::CURRENT_TIME;
            self.send(target, self.atoms.drop, [self.source, 0, time, 0, 0])?;
        }
        return Ok(());
    }

    /// Answers whatever arrived: the target's status and finished
    /// messages, and its requests for the data.
    fn handle_events(&mut self) -> Result<(), String> {
        while let Some(event) = self.conn.poll_for_event().map_err(|e| e.to_string())? {
            match event {
                Event::ClientMessage(message) => {
                    let data = message.data.as_data32();
                    let from_target = self.target.is_some_and(|t| t.window == data[0]);
                    if message.type_ == self.atoms.status && from_target {
                        self.accepted = data[1] & 1 != 0;
                        self.waiting_since = None;
                    } else if message.type_ == self.atoms.finished {
                        self.finished = true;
                    }
                }
                Event::SelectionRequest(request) => self.answer(&request)?,
                _ => {}
            }
        }
        return Ok(());
    }

    /// Gives the paths to a target that asked for them (or the list of
    /// types it can ask for).
    fn answer(&self, request: &SelectionRequestEvent) -> Result<(), String> {
        // Very old clients leave the property empty; use the target then.
        let property = if request.property == x11rb::NONE {
            request.target
        } else {
            request.property
        };
        let atoms = &self.atoms;
        let max = self.conn.maximum_request_bytes().saturating_sub(64);
        let mut given = x11rb::NONE;
        if request.selection == atoms.selection {
            if request.target == atoms.targets {
                let list = [
                    atoms.targets,
                    atoms.uri_list,
                    atoms.utf8_string,
                    atoms.text_plain,
                ];
                self.conn
                    .change_property32(
                        PropMode::REPLACE,
                        request.requestor,
                        property,
                        AtomEnum::ATOM,
                        &list,
                    )
                    .map_err(|e| e.to_string())?;
                given = property;
            } else {
                let bytes = if request.target == atoms.uri_list {
                    Some(&self.uri_list)
                } else if request.target == atoms.utf8_string || request.target == atoms.text_plain
                {
                    Some(&self.text)
                } else {
                    None
                };
                // Too big for one request (thousands of files): refuse
                // rather than implement the INCR protocol.
                if let Some(bytes) = bytes
                    && bytes.len() < max
                {
                    self.conn
                        .change_property8(
                            PropMode::REPLACE,
                            request.requestor,
                            property,
                            request.target,
                            bytes,
                        )
                        .map_err(|e| e.to_string())?;
                    given = property;
                }
            }
        }
        let notify = SelectionNotifyEvent {
            response_type: xproto::SELECTION_NOTIFY_EVENT,
            sequence: 0,
            time: request.time,
            requestor: request.requestor,
            selection: request.selection,
            target: request.target,
            property: given,
        };
        self.conn
            .send_event(false, request.requestor, EventMask::NO_EVENT, notify)
            .map_err(|e| e.to_string())?;
        self.conn.flush().map_err(|e| e.to_string())?;
        return Ok(());
    }
}

/// The paths as `text/uri-list`: one `file://` URI per line, CRLF ended.
pub fn uri_list(paths: &[PathBuf]) -> String {
    let mut out = String::new();
    for path in paths {
        out.push_str(&crate::percent::path_to_file_uri(path));
        out.push_str("\r\n");
    }
    return out;
}

/// The paths as plain text, one per line, for targets that only take text
/// (a terminal pastes them).
pub fn plain_text(paths: &[PathBuf]) -> String {
    let mut lines = Vec::new();
    for path in paths {
        lines.push(path.to_string_lossy().into_owned());
    }
    return lines.join("\n");
}

/// XdndEnter: who's dragging, the version, and up to three types (more
/// would need the XdndTypeList property).
fn enter_data(source: Window, version: u32, types: [Atom; 3]) -> [u32; 5] {
    return [source, version << 24, types[0], types[1], types[2]];
}

/// XdndPosition: the pointer in root coordinates, packed x << 16 | y, and
/// the action we'd like (copy).
fn position_data(source: Window, x: i16, y: i16, action: Atom) -> [u32; 5] {
    let packed = ((x as u16 as u32) << 16) | (y as u16 as u32);
    return [source, 0, packed, x11rb::CURRENT_TIME, action];
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uri_list_and_text() {
        let paths = vec![PathBuf::from("/home/me/a b.txt"), PathBuf::from("/tmp/x")];
        assert_eq!(
            uri_list(&paths),
            "file:///home/me/a%20b.txt\r\nfile:///tmp/x\r\n"
        );
        assert_eq!(plain_text(&paths), "/home/me/a b.txt\n/tmp/x");
    }

    #[test]
    fn message_data() {
        assert_eq!(enter_data(7, 5, [1, 2, 3]), [7, 5 << 24, 1, 2, 3]);
        let data = position_data(7, 100, 200, 9);
        assert_eq!(data[2], (100 << 16) | 200);
        assert_eq!(data[4], 9);
        // Off-screen (negative) coordinates keep their 16 bits each.
        assert_eq!(position_data(7, -1, 2, 9)[2], 0xffff_0002);
    }
}

/// Talks XDND with a pretend drop target on the real X server, with two
/// connections and windows that are never mapped (nothing shows, and the
/// pointer isn't used). Needs DISPLAY; run by hand:
/// `cargo test -p itamiiru-core live_xdnd -- --ignored --nocapture`
#[cfg(test)]
mod live {
    use super::*;

    fn wait_event(conn: &RustConnection) -> Event {
        let start = Instant::now();
        loop {
            if let Some(event) = conn.poll_for_event().expect("events") {
                return event;
            }
            assert!(start.elapsed() < Duration::from_secs(5), "no event");
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    #[ignore]
    fn live_xdnd() {
        // The target: a hidden window that says it accepts drops.
        let (target_conn, screen) = RustConnection::connect(None).expect("X server");
        let root = target_conn.setup().roots[screen].root;
        let window = target_conn.generate_id().expect("id");
        target_conn
            .create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                window,
                root,
                0,
                0,
                1,
                1,
                0,
                WindowClass::INPUT_ONLY,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new(),
            )
            .expect("window");
        let target_atoms = Atoms::intern(&target_conn).expect("atoms");
        target_conn
            .change_property32(
                PropMode::REPLACE,
                window,
                target_atoms.aware,
                AtomEnum::ATOM,
                &[5],
            )
            .expect("aware");
        target_conn.flush().expect("flush");

        // The source: our session, on its own connection.
        let (conn, _) = RustConnection::connect(None).expect("X server");
        let source = conn.generate_id().expect("id");
        conn.create_window(
            x11rb::COPY_DEPTH_FROM_PARENT,
            source,
            root,
            -10,
            -10,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new(),
        )
        .expect("window");
        let paths = vec![PathBuf::from("/tmp/a b.txt"), PathBuf::from("/tmp/c")];
        let atoms = Atoms::intern(&conn).expect("atoms");
        let mut session = Session::new(&conn, atoms, source, &paths).expect("session");

        // Our find_target sees the hidden window as a target.
        let found = session
            .find_target(root, window)
            .expect("look")
            .expect("a target");
        assert_eq!(found.window, window);
        assert_eq!(found.version, 5);

        // Enter and position arrive with our window and types.
        session.enter(found).expect("enter");
        session.position(100, 200).expect("position");
        let Event::ClientMessage(enter) = wait_event(&target_conn) else {
            panic!("expected XdndEnter");
        };
        assert_eq!(enter.type_, target_atoms.enter);
        let data = enter.data.as_data32();
        assert_eq!(data[0], source);
        assert_eq!(data[1] >> 24, 5);
        assert_eq!(data[2], target_atoms.uri_list);
        let Event::ClientMessage(position) = wait_event(&target_conn) else {
            panic!("expected XdndPosition");
        };
        assert_eq!(position.type_, target_atoms.position);
        assert_eq!(position.data.as_data32()[2], (100 << 16) | 200);

        // The target accepts; the session hears it.
        let status = ClientMessageEvent::new(
            32,
            source,
            target_atoms.status,
            [window, 1, 0, 0, target_atoms.action_copy],
        );
        target_conn
            .send_event(false, source, EventMask::NO_EVENT, status)
            .expect("status");
        target_conn.flush().expect("flush");
        let start = Instant::now();
        while !session.accepted && start.elapsed() < Duration::from_secs(5) {
            session.handle_events().expect("events");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(session.accepted, "the session saw the status");

        // Drop, then the target asks for text/uri-list and gets the paths.
        session.drop_here().expect("drop");
        let Event::ClientMessage(dropped) = wait_event(&target_conn) else {
            panic!("expected XdndDrop");
        };
        assert_eq!(dropped.type_, target_atoms.drop);
        let property = target_atoms.uri_list;
        target_conn
            .convert_selection(
                window,
                target_atoms.selection,
                target_atoms.uri_list,
                property,
                x11rb::CURRENT_TIME,
            )
            .expect("convert");
        target_conn.flush().expect("flush");
        let notify = loop {
            session.handle_events().expect("events");
            if let Some(Event::SelectionNotify(notify)) = target_conn.poll_for_event().expect("ev")
            {
                break notify;
            }
            std::thread::sleep(Duration::from_millis(5));
        };
        assert_eq!(notify.property, property, "the data was given");
        let value = target_conn
            .get_property(true, window, property, AtomEnum::ANY, 0, 1024)
            .expect("get")
            .reply()
            .expect("reply");
        assert_eq!(
            String::from_utf8_lossy(&value.value),
            "file:///tmp/a%20b.txt\r\nfile:///tmp/c\r\n"
        );

        // The target says it's done.
        let finished =
            ClientMessageEvent::new(32, source, target_atoms.finished, [window, 1, 0, 0, 0]);
        target_conn
            .send_event(false, source, EventMask::NO_EVENT, finished)
            .expect("finished");
        target_conn.flush().expect("flush");
        let start = Instant::now();
        while !session.finished && start.elapsed() < Duration::from_secs(5) {
            session.handle_events().expect("events");
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(session.finished);
        println!("XDND exchange with the X server worked");
    }
}
