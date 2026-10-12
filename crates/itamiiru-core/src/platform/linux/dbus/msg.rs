//! D-Bus messages: the fixed header, header fields and body.

use super::wire::{self, Reader, Type, Value, WireError, Writer};

/// Messages bigger than this are refused (the portal's are a few KiB).
pub const MAX_MESSAGE_SIZE: usize = 1024 * 1024;
/// The caller doesn't want a reply.
pub const NO_REPLY_EXPECTED: u8 = 0x1;
/// The caller is happy to wait while polkit asks the user for a password.
/// Without it, actions that need one fail straight away.
pub const ALLOW_INTERACTIVE_AUTHORIZATION: u8 = 0x4;

const FIELD_PATH: u8 = 1;
const FIELD_INTERFACE: u8 = 2;
const FIELD_MEMBER: u8 = 3;
const FIELD_ERROR_NAME: u8 = 4;
const FIELD_REPLY_SERIAL: u8 = 5;
const FIELD_DESTINATION: u8 = 6;
const FIELD_SENDER: u8 = 7;
const FIELD_SIGNATURE: u8 = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    MethodCall,
    MethodReturn,
    Error,
    Signal,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Message {
    pub kind: Kind,
    pub flags: u8,
    pub serial: u32,
    pub path: Option<String>,
    pub interface: Option<String>,
    pub member: Option<String>,
    pub error_name: Option<String>,
    pub destination: Option<String>,
    pub sender: Option<String>,
    pub reply_serial: Option<u32>,
    pub body: Vec<Value>,
}

impl Message {
    fn empty(kind: Kind) -> Message {
        return Message {
            kind,
            flags: 0,
            serial: 0,
            path: None,
            interface: None,
            member: None,
            error_name: None,
            destination: None,
            sender: None,
            reply_serial: None,
            body: Vec::new(),
        };
    }

    pub fn method_call(
        destination: &str,
        path: &str,
        interface: &str,
        member: &str,
        body: Vec<Value>,
    ) -> Message {
        let mut msg = Message::empty(Kind::MethodCall);
        msg.destination = Some(destination.to_string());
        msg.path = Some(path.to_string());
        msg.interface = Some(interface.to_string());
        msg.member = Some(member.to_string());
        msg.body = body;
        return msg;
    }

    pub fn method_return(call: &Message, body: Vec<Value>) -> Message {
        let mut msg = Message::empty(Kind::MethodReturn);
        msg.destination = call.sender.clone();
        msg.reply_serial = Some(call.serial);
        msg.body = body;
        return msg;
    }

    pub fn error(call: &Message, name: &str, text: &str) -> Message {
        let mut msg = Message::empty(Kind::Error);
        msg.destination = call.sender.clone();
        msg.reply_serial = Some(call.serial);
        msg.error_name = Some(name.to_string());
        msg.body = vec![Value::Str(text.to_string())];
        return msg;
    }

    pub fn wants_reply(&self) -> bool {
        return self.kind == Kind::MethodCall && self.flags & NO_REPLY_EXPECTED == 0;
    }

    pub fn signature(&self) -> String {
        let mut out = String::new();
        for value in &self.body {
            out.push_str(&value.value_type().signature());
        }
        return out;
    }

    /// The bytes on the wire, little-endian, with the given serial.
    pub fn encode(&self, serial: u32) -> Result<Vec<u8>, WireError> {
        let mut body = Writer::default();
        for value in &self.body {
            body.write(value)?;
        }

        let mut fields = Vec::new();
        let mut field = |code: u8, value: Value| {
            fields.push(Value::Struct(vec![
                Value::Byte(code),
                Value::Variant(Box::new(value)),
            ]));
        };
        if let Some(path) = &self.path {
            field(FIELD_PATH, Value::ObjectPath(path.clone()));
        }
        if let Some(interface) = &self.interface {
            field(FIELD_INTERFACE, Value::Str(interface.clone()));
        }
        if let Some(member) = &self.member {
            field(FIELD_MEMBER, Value::Str(member.clone()));
        }
        if let Some(name) = &self.error_name {
            field(FIELD_ERROR_NAME, Value::Str(name.clone()));
        }
        if let Some(reply) = self.reply_serial {
            field(FIELD_REPLY_SERIAL, Value::U32(reply));
        }
        if let Some(destination) = &self.destination {
            field(FIELD_DESTINATION, Value::Str(destination.clone()));
        }
        if let Some(sender) = &self.sender {
            field(FIELD_SENDER, Value::Str(sender.clone()));
        }
        if !self.body.is_empty() {
            field(FIELD_SIGNATURE, Value::Signature(self.signature()));
        }

        let kind = match self.kind {
            Kind::MethodCall => 1,
            Kind::MethodReturn => 2,
            Kind::Error => 3,
            Kind::Signal => 4,
        };
        let mut out = Writer::default();
        out.buf.extend_from_slice(&[b'l', kind, self.flags, 1]);
        out.buf
            .extend_from_slice(&(body.buf.len() as u32).to_le_bytes());
        out.buf.extend_from_slice(&serial.to_le_bytes());
        let field_type = Type::Struct(vec![Type::Byte, Type::Variant]);
        out.write(&Value::Array(field_type, fields))?;
        // The body starts at a multiple of 8, so its own alignment holds.
        out.pad(8);
        out.buf.extend_from_slice(&body.buf);
        if out.buf.len() > MAX_MESSAGE_SIZE {
            return Err(WireError("message too large".to_string()));
        }
        return Ok(out.buf);
    }
}

/// The whole message's length, from its first 16 bytes.
pub fn total_length(start: &[u8; 16]) -> Result<usize, WireError> {
    let big_endian = match start[0] {
        b'l' => false,
        b'B' => true,
        _ => return Err(WireError("bad byte-order mark".to_string())),
    };
    let read = |offset: usize| {
        let mut bytes = [
            start[offset],
            start[offset + 1],
            start[offset + 2],
            start[offset + 3],
        ];
        if big_endian {
            bytes.reverse();
        }
        u32::from_le_bytes(bytes) as usize
    };
    let body_length = read(4);
    let fields_length = read(12);
    if body_length > MAX_MESSAGE_SIZE || fields_length > MAX_MESSAGE_SIZE {
        return Err(WireError("message too large".to_string()));
    }
    let header = (16 + fields_length).div_ceil(8) * 8;
    let total = header + body_length;
    if total > MAX_MESSAGE_SIZE {
        return Err(WireError("message too large".to_string()));
    }
    return Ok(total);
}

pub fn decode(bytes: &[u8]) -> Result<Message, WireError> {
    if bytes.len() < 16 {
        return Err(WireError("message shorter than its header".to_string()));
    }
    let big_endian = match bytes[0] {
        b'l' => false,
        b'B' => true,
        _ => return Err(WireError("bad byte-order mark".to_string())),
    };
    let kind = match bytes[1] {
        1 => Kind::MethodCall,
        2 => Kind::MethodReturn,
        3 => Kind::Error,
        4 => Kind::Signal,
        other => return Err(WireError(format!("unknown message type {other}"))),
    };
    if bytes[3] != 1 {
        return Err(WireError("unsupported protocol version".to_string()));
    }
    let mut msg = Message::empty(kind);
    msg.flags = bytes[2];

    let mut reader = Reader::new(bytes, big_endian);
    reader.pos = 4;
    let body_length = reader.read_u32()? as usize;
    msg.serial = reader.read_u32()?;
    if msg.serial == 0 {
        return Err(WireError("serial 0".to_string()));
    }
    let field_type = Type::Array(Box::new(Type::Struct(vec![Type::Byte, Type::Variant])));
    let fields = reader.read(&field_type)?;
    let mut signature = String::new();
    for field in fields.as_array().unwrap_or(&[]) {
        let Some([code, value]) = field.as_struct().map(|f| [&f[0], &f[1]]) else {
            continue;
        };
        let Value::Byte(code) = code else {
            continue;
        };
        let value = value.unwrap_variant();
        match *code {
            FIELD_PATH => msg.path = value.as_str().map(String::from),
            FIELD_INTERFACE => msg.interface = value.as_str().map(String::from),
            FIELD_MEMBER => msg.member = value.as_str().map(String::from),
            FIELD_ERROR_NAME => msg.error_name = value.as_str().map(String::from),
            FIELD_REPLY_SERIAL => msg.reply_serial = value.as_u32(),
            FIELD_DESTINATION => msg.destination = value.as_str().map(String::from),
            FIELD_SENDER => msg.sender = value.as_str().map(String::from),
            FIELD_SIGNATURE => signature = value.as_str().unwrap_or("").to_string(),
            // UNIX_FDS and unknown fields are ignored.
            _ => {}
        }
    }
    reader.align(8)?;
    let body_end = reader.pos + body_length;
    if body_end != bytes.len() {
        return Err(WireError(
            "body length doesn't match the message".to_string(),
        ));
    }
    for t in wire::parse_signature(&signature)? {
        msg.body.push(reader.read(&t)?);
    }
    if reader.pos != body_end {
        return Err(WireError("body shorter than its length".to_string()));
    }

    let ok = match kind {
        Kind::MethodCall => msg.path.is_some() && msg.member.is_some(),
        Kind::MethodReturn => msg.reply_serial.is_some(),
        Kind::Error => msg.reply_serial.is_some() && msg.error_name.is_some(),
        Kind::Signal => msg.path.is_some() && msg.interface.is_some() && msg.member.is_some(),
    };
    if !ok {
        return Err(WireError(
            "message is missing required header fields".to_string(),
        ));
    }
    return Ok(msg);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_round_trip() {
        let msg = Message::method_call(
            "org.freedesktop.DBus",
            "/org/freedesktop/DBus",
            "org.freedesktop.DBus",
            "Hello",
            Vec::new(),
        );
        let bytes = msg.encode(1).unwrap();
        assert_eq!(&bytes[..4], &[b'l', 1, 0, 1]);
        assert_eq!(bytes.len() % 8, 0, "no body: the header ends padded to 8");
        let mut start = [0u8; 16];
        start.copy_from_slice(&bytes[..16]);
        assert_eq!(total_length(&start).unwrap(), bytes.len());
        let mut decoded = decode(&bytes).unwrap();
        assert_eq!(decoded.serial, 1);
        decoded.serial = 0;
        assert_eq!(decoded, msg);
    }

    #[test]
    fn reply_with_body() {
        let mut call =
            Message::method_call("x.y", "/", "x.y", "M", vec![Value::Str("a".to_string())]);
        call.sender = Some(":1.5".to_string());
        call.serial = 9;
        let reply = Message::method_return(&call, vec![Value::U32(0), Value::dict(vec![])]);
        let decoded = decode(&reply.encode(3).unwrap()).unwrap();
        assert_eq!(decoded.kind, Kind::MethodReturn);
        assert_eq!(decoded.reply_serial, Some(9));
        assert_eq!(decoded.destination.as_deref(), Some(":1.5"));
        assert_eq!(decoded.signature(), "ua{sv}");
        let error = Message::error(&call, "org.freedesktop.DBus.Error.Failed", "nope");
        assert_eq!(
            decode(&error.encode(4).unwrap())
                .unwrap()
                .error_name
                .as_deref(),
            Some("org.freedesktop.DBus.Error.Failed")
        );
    }

    #[test]
    fn bad_messages() {
        let good = Message::method_call("a.b", "/", "a.b", "M", vec![Value::U32(1)])
            .encode(1)
            .unwrap();
        let mut bad_order = good.clone();
        bad_order[0] = b'x';
        assert!(decode(&bad_order).is_err());
        assert!(decode(&good[..good.len() - 1]).is_err(), "truncated");
        let mut zero_serial = good.clone();
        zero_serial[8..12].copy_from_slice(&0u32.to_le_bytes());
        assert!(decode(&zero_serial).is_err());
        let mut huge = [0u8; 16];
        huge[0] = b'l';
        huge[4..8].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(total_length(&huge).is_err());
        // A method call without a member.
        let mut no_member = Message::method_call("a.b", "/", "a.b", "M", Vec::new());
        no_member.member = None;
        assert!(decode(&no_member.encode(1).unwrap()).is_err());
    }
}
