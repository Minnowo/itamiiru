//! Connecting to the session bus (P3.1): the address, the socket, SASL
//! EXTERNAL authentication, Hello and RequestName.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;

use super::msg::{self, Kind, Message};
use super::wire::Value;

/// Where the bus listens.
#[derive(Debug, PartialEq, Eq)]
pub enum Address {
    Path(PathBuf),
    /// A Linux abstract socket name (no file).
    Abstract(Vec<u8>),
}

/// Parses `$DBUS_SESSION_BUS_ADDRESS`, e.g. `unix:path=/run/user/1000/bus`.
/// Several addresses may be separated by `;`; the first usable one wins.
/// Values may be %-escaped. Other keys (like `guid=`) are ignored.
pub fn parse_address(text: &str) -> Option<Address> {
    for address in text.split(';') {
        let Some(params) = address.strip_prefix("unix:") else {
            continue;
        };
        for pair in params.split(',') {
            let Some((key, value)) = pair.split_once('=') else {
                continue;
            };
            let Ok(value) = itamiiru_core::percent::decode(value) else {
                continue;
            };
            match key {
                "path" => {
                    use std::os::unix::ffi::OsStringExt;
                    return Some(Address::Path(PathBuf::from(std::ffi::OsString::from_vec(
                        value,
                    ))));
                }
                "abstract" => return Some(Address::Abstract(value)),
                _ => {}
            }
        }
    }
    return None;
}

/// The session bus address, falling back to `$XDG_RUNTIME_DIR/bus`.
pub fn session_address() -> Option<Address> {
    if let Ok(text) = std::env::var("DBUS_SESSION_BUS_ADDRESS")
        && let Some(address) = parse_address(&text)
    {
        return Some(address);
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR")?;
    return Some(Address::Path(PathBuf::from(runtime).join("bus")));
}

/// The sending half of a bus connection. The receiving half is a cloned
/// socket, read on its own thread.
pub struct Connection {
    stream: UnixStream,
    next_serial: u32,
    pub unique_name: String,
}

impl Connection {
    /// Connects and authenticates. Returns the connection and a socket
    /// for reading.
    pub fn open(address: &Address) -> io::Result<(Connection, UnixStream)> {
        let mut stream = match address {
            Address::Path(path) => UnixStream::connect(path)?,
            Address::Abstract(name) => {
                use std::os::linux::net::SocketAddrExt;
                let addr = std::os::unix::net::SocketAddr::from_abstract_name(name)?;
                UnixStream::connect_addr(&addr)?
            }
        };
        authenticate(&mut stream)?;
        let reader = stream.try_clone()?;
        let connection = Connection {
            stream,
            next_serial: 1,
            unique_name: String::new(),
        };
        return Ok((connection, reader));
    }

    /// Sends a message, giving it the next serial. Returns that serial.
    pub fn send(&mut self, message: &Message) -> io::Result<u32> {
        let serial = self.next_serial;
        self.next_serial = self.next_serial.wrapping_add(1).max(1);
        let bytes = message.encode(serial).map_err(io::Error::other)?;
        self.stream.write_all(&bytes)?;
        return Ok(serial);
    }

    /// Sends a call and waits for its reply, dropping anything else that
    /// arrives meanwhile. Only for startup, before the reader thread runs.
    pub fn call(&mut self, reader: &mut UnixStream, message: &Message) -> io::Result<Message> {
        let serial = self.send(message)?;
        loop {
            let reply = read_message(reader)?;
            if reply.reply_serial != Some(serial) {
                continue;
            }
            if reply.kind == Kind::Error {
                let name = reply.error_name.clone().unwrap_or_default();
                let text = reply
                    .body
                    .first()
                    .and_then(Value::as_str)
                    .unwrap_or("")
                    .to_string();
                return Err(io::Error::other(format!("{name}: {text}")));
            }
            return Ok(reply);
        }
    }

    /// Hello, then RequestName with DO_NOT_QUEUE. Fails if someone else
    /// already owns the name.
    pub fn register(&mut self, reader: &mut UnixStream, name: &str) -> io::Result<()> {
        let bus = |member: &str, body: Vec<Value>| {
            Message::method_call(
                "org.freedesktop.DBus",
                "/org/freedesktop/DBus",
                "org.freedesktop.DBus",
                member,
                body,
            )
        };
        let hello = self.call(reader, &bus("Hello", Vec::new()))?;
        self.unique_name = hello
            .body
            .first()
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();

        const DO_NOT_QUEUE: u32 = 4;
        let request = bus(
            "RequestName",
            vec![Value::Str(name.to_string()), Value::U32(DO_NOT_QUEUE)],
        );
        let reply = self.call(reader, &request)?;
        // 1 = became the owner, 4 = already the owner.
        return match reply.body.first().and_then(Value::as_u32) {
            Some(1) | Some(4) => Ok(()),
            Some(3) => Err(io::Error::other(format!(
                "{name} is already taken by another process"
            ))),
            other => Err(io::Error::other(format!(
                "RequestName({name}) answered {other:?}"
            ))),
        };
    }
}

/// SASL EXTERNAL: prove who we are with our user id, as the kernel sees it.
fn authenticate(stream: &mut UnixStream) -> io::Result<()> {
    let uid = itamiiru_core::platform::current_uid().to_string();
    let mut hex = String::new();
    for b in uid.bytes() {
        hex.push_str(&format!("{b:02x}"));
    }
    stream.write_all(b"\0")?;
    stream.write_all(format!("AUTH EXTERNAL {hex}\r\n").as_bytes())?;
    let line = read_line(stream)?;
    if !line.starts_with("OK ") {
        return Err(io::Error::other(format!(
            "bus refused authentication: {line}"
        )));
    }
    stream.write_all(b"BEGIN\r\n")?;
    return Ok(());
}

/// Reads one `\r\n`-terminated line, a byte at a time so nothing past it
/// is consumed.
fn read_line(stream: &mut UnixStream) -> io::Result<String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 1024 {
        stream.read_exact(&mut byte)?;
        if byte[0] == b'\n' {
            let text = String::from_utf8_lossy(&line);
            return Ok(text.trim_end_matches('\r').to_string());
        }
        line.push(byte[0]);
    }
    return Err(io::Error::other("authentication line too long"));
}

/// Reads one whole message from the socket.
pub fn read_message(stream: &mut impl Read) -> io::Result<Message> {
    let mut start = [0u8; 16];
    stream.read_exact(&mut start)?;
    let total = msg::total_length(&start).map_err(io::Error::other)?;
    if total < 16 {
        return Err(io::Error::other("message shorter than its header"));
    }
    let mut bytes = vec![0u8; total];
    bytes[..16].copy_from_slice(&start);
    stream.read_exact(&mut bytes[16..])?;
    return msg::decode(&bytes).map_err(io::Error::other);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses() {
        assert_eq!(
            parse_address("unix:path=/run/user/1000/bus"),
            Some(Address::Path(PathBuf::from("/run/user/1000/bus")))
        );
        assert_eq!(
            parse_address("unix:abstract=/tmp/dbus-XYZ,guid=abc"),
            Some(Address::Abstract(b"/tmp/dbus-XYZ".to_vec()))
        );
        assert_eq!(
            parse_address("tcp:host=x;unix:path=/a%20b"),
            Some(Address::Path(PathBuf::from("/a b")))
        );
        assert_eq!(parse_address("tcp:host=localhost,port=1"), None);
        assert_eq!(parse_address(""), None);
    }

    #[test]
    fn read_message_from_bytes() {
        let msg = Message::method_call("a.b", "/x", "a.b", "Ping", vec![Value::U32(3)]);
        let bytes = msg.encode(5).unwrap();
        let mut cursor = std::io::Cursor::new(bytes);
        let decoded = read_message(&mut cursor).unwrap();
        assert_eq!(decoded.member.as_deref(), Some("Ping"));
        assert_eq!(decoded.serial, 5);
    }
}
