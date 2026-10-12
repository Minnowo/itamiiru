//! The D-Bus wire format (P3.2): types, signatures, and marshalling values
//! to and from bytes. https://dbus.freedesktop.org/doc/dbus-specification.html
//!
//! Every value is aligned to its type's size from the start of the message.
//! We read both byte orders and always write little-endian. Input comes
//! from other processes, so every length is checked before it's used.

/// Nesting deeper than this (in a value or a signature) is refused.
const MAX_DEPTH: usize = 64;
/// Longest array the spec allows, in bytes.
const MAX_ARRAY_BYTES: usize = 64 * 1024 * 1024;

#[derive(Debug, PartialEq, Eq)]
pub struct WireError(pub String);

impl std::fmt::Display for WireError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        return write!(f, "D-Bus wire error: {}", self.0);
    }
}

impl std::error::Error for WireError {}

fn error(text: impl Into<String>) -> WireError {
    return WireError(text.into());
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Type {
    Byte,
    Bool,
    I16,
    U16,
    I32,
    U32,
    I64,
    U64,
    Double,
    Str,
    ObjectPath,
    Signature,
    Variant,
    UnixFd,
    Array(Box<Type>),
    Struct(Vec<Type>),
    DictEntry(Box<Type>, Box<Type>),
}

impl Type {
    /// Values of this type start at a multiple of this many bytes.
    pub fn alignment(&self) -> usize {
        return match self {
            Type::Byte | Type::Signature | Type::Variant => 1,
            Type::I16 | Type::U16 => 2,
            Type::Bool
            | Type::I32
            | Type::U32
            | Type::Str
            | Type::ObjectPath
            | Type::UnixFd
            | Type::Array(_) => 4,
            Type::I64 | Type::U64 | Type::Double | Type::Struct(_) | Type::DictEntry(_, _) => 8,
        };
    }

    pub fn signature(&self) -> String {
        let mut out = String::new();
        self.push_signature(&mut out);
        return out;
    }

    fn push_signature(&self, out: &mut String) {
        match self {
            Type::Byte => out.push('y'),
            Type::Bool => out.push('b'),
            Type::I16 => out.push('n'),
            Type::U16 => out.push('q'),
            Type::I32 => out.push('i'),
            Type::U32 => out.push('u'),
            Type::I64 => out.push('x'),
            Type::U64 => out.push('t'),
            Type::Double => out.push('d'),
            Type::Str => out.push('s'),
            Type::ObjectPath => out.push('o'),
            Type::Signature => out.push('g'),
            Type::Variant => out.push('v'),
            Type::UnixFd => out.push('h'),
            Type::Array(element) => {
                out.push('a');
                element.push_signature(out);
            }
            Type::Struct(fields) => {
                out.push('(');
                for field in fields {
                    field.push_signature(out);
                }
                out.push(')');
            }
            Type::DictEntry(key, value) => {
                out.push('{');
                key.push_signature(out);
                value.push_signature(out);
                out.push('}');
            }
        }
    }

    fn is_basic(&self) -> bool {
        return !matches!(
            self,
            Type::Variant | Type::Array(_) | Type::Struct(_) | Type::DictEntry(_, _)
        );
    }
}

/// Parses a signature like `sa{sv}` into its complete types.
pub fn parse_signature(signature: &str) -> Result<Vec<Type>, WireError> {
    if signature.len() > 255 {
        return Err(error("signature longer than 255"));
    }
    let bytes = signature.as_bytes();
    let mut pos = 0;
    let mut types = Vec::new();
    while pos < bytes.len() {
        types.push(parse_one(bytes, &mut pos, 0)?);
    }
    return Ok(types);
}

fn parse_one(bytes: &[u8], pos: &mut usize, depth: usize) -> Result<Type, WireError> {
    if depth > MAX_DEPTH {
        return Err(error("signature nested too deep"));
    }
    let Some(&c) = bytes.get(*pos) else {
        return Err(error("signature ends early"));
    };
    *pos += 1;
    let simple = match c {
        b'y' => Some(Type::Byte),
        b'b' => Some(Type::Bool),
        b'n' => Some(Type::I16),
        b'q' => Some(Type::U16),
        b'i' => Some(Type::I32),
        b'u' => Some(Type::U32),
        b'x' => Some(Type::I64),
        b't' => Some(Type::U64),
        b'd' => Some(Type::Double),
        b's' => Some(Type::Str),
        b'o' => Some(Type::ObjectPath),
        b'g' => Some(Type::Signature),
        b'v' => Some(Type::Variant),
        b'h' => Some(Type::UnixFd),
        _ => None,
    };
    if let Some(t) = simple {
        return Ok(t);
    }
    match c {
        b'a' => {
            if bytes.get(*pos) == Some(&b'{') {
                *pos += 1;
                let key = parse_one(bytes, pos, depth + 1)?;
                if !key.is_basic() {
                    return Err(error("dict key must be a basic type"));
                }
                let value = parse_one(bytes, pos, depth + 1)?;
                if bytes.get(*pos) != Some(&b'}') {
                    return Err(error("dict entry must have exactly two types"));
                }
                *pos += 1;
                return Ok(Type::Array(Box::new(Type::DictEntry(
                    Box::new(key),
                    Box::new(value),
                ))));
            }
            let element = parse_one(bytes, pos, depth + 1)?;
            return Ok(Type::Array(Box::new(element)));
        }
        b'(' => {
            let mut fields = Vec::new();
            loop {
                match bytes.get(*pos) {
                    Some(b')') => {
                        *pos += 1;
                        break;
                    }
                    Some(_) => fields.push(parse_one(bytes, pos, depth + 1)?),
                    None => return Err(error("struct not closed")),
                }
            }
            if fields.is_empty() {
                return Err(error("empty struct"));
            }
            return Ok(Type::Struct(fields));
        }
        _ => return Err(error(format!("bad signature character {:?}", c as char))),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    Byte(u8),
    Bool(bool),
    I16(i16),
    U16(u16),
    I32(i32),
    U32(u32),
    I64(i64),
    U64(u64),
    Double(f64),
    Str(String),
    ObjectPath(String),
    Signature(String),
    Variant(Box<Value>),
    UnixFd(u32),
    /// The element type is kept so empty arrays still know what they hold.
    Array(Type, Vec<Value>),
    Struct(Vec<Value>),
    DictEntry(Box<Value>, Box<Value>),
}

impl Value {
    pub fn value_type(&self) -> Type {
        return match self {
            Value::Byte(_) => Type::Byte,
            Value::Bool(_) => Type::Bool,
            Value::I16(_) => Type::I16,
            Value::U16(_) => Type::U16,
            Value::I32(_) => Type::I32,
            Value::U32(_) => Type::U32,
            Value::I64(_) => Type::I64,
            Value::U64(_) => Type::U64,
            Value::Double(_) => Type::Double,
            Value::Str(_) => Type::Str,
            Value::ObjectPath(_) => Type::ObjectPath,
            Value::Signature(_) => Type::Signature,
            Value::Variant(_) => Type::Variant,
            Value::UnixFd(_) => Type::UnixFd,
            Value::Array(element, _) => Type::Array(Box::new(element.clone())),
            Value::Struct(fields) => {
                let mut types = Vec::new();
                for field in fields {
                    types.push(field.value_type());
                }
                Type::Struct(types)
            }
            Value::DictEntry(key, value) => {
                Type::DictEntry(Box::new(key.value_type()), Box::new(value.value_type()))
            }
        };
    }

    /// The inside of a variant, or the value itself.
    pub fn unwrap_variant(&self) -> &Value {
        if let Value::Variant(inner) = self {
            return inner.unwrap_variant();
        }
        return self;
    }

    pub fn as_str(&self) -> Option<&str> {
        return match self.unwrap_variant() {
            Value::Str(s) | Value::ObjectPath(s) | Value::Signature(s) => Some(s),
            _ => None,
        };
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self.unwrap_variant() {
            return Some(*b);
        }
        return None;
    }

    pub fn as_u32(&self) -> Option<u32> {
        if let Value::U32(n) = self.unwrap_variant() {
            return Some(*n);
        }
        return None;
    }

    pub fn as_u64(&self) -> Option<u64> {
        if let Value::U64(n) = self.unwrap_variant() {
            return Some(*n);
        }
        return None;
    }

    /// The elements of an array (inside a variant or not).
    pub fn as_array(&self) -> Option<&[Value]> {
        if let Value::Array(_, items) = self.unwrap_variant() {
            return Some(items);
        }
        return None;
    }

    pub fn as_struct(&self) -> Option<&[Value]> {
        if let Value::Struct(fields) = self.unwrap_variant() {
            return Some(fields);
        }
        return None;
    }

    /// An `ay` as bytes.
    pub fn as_bytes(&self) -> Option<Vec<u8>> {
        let items = self.as_array()?;
        let mut out = Vec::with_capacity(items.len());
        for item in items {
            let Value::Byte(b) = item else {
                return None;
            };
            out.push(*b);
        }
        return Some(out);
    }

    /// Looks up `key` in an `a{sv}` dict; returns the value inside the variant.
    pub fn dict_get(&self, key: &str) -> Option<&Value> {
        for entry in self.as_array()? {
            if let Value::DictEntry(k, v) = entry
                && k.as_str() == Some(key)
            {
                return Some(v.unwrap_variant());
            }
        }
        return None;
    }

    /// An `ay` from bytes.
    pub fn bytes(data: &[u8]) -> Value {
        let mut items = Vec::with_capacity(data.len());
        for &b in data {
            items.push(Value::Byte(b));
        }
        return Value::Array(Type::Byte, items);
    }

    /// An `a{sv}` from (key, value) pairs; each value is wrapped in a variant.
    pub fn dict(pairs: Vec<(&str, Value)>) -> Value {
        let mut entries = Vec::new();
        for (key, value) in pairs {
            entries.push(Value::DictEntry(
                Box::new(Value::Str(key.to_string())),
                Box::new(Value::Variant(Box::new(value))),
            ));
        }
        return Value::Array(
            Type::DictEntry(Box::new(Type::Str), Box::new(Type::Variant)),
            entries,
        );
    }

    /// An `as`.
    pub fn strings(items: Vec<String>) -> Value {
        let mut values = Vec::new();
        for item in items {
            values.push(Value::Str(item));
        }
        return Value::Array(Type::Str, values);
    }
}

/// Appends values in little-endian. Alignment is counted from the start of
/// `buf`, which must be the start of the message (or an 8-aligned point in it).
#[derive(Default)]
pub struct Writer {
    pub buf: Vec<u8>,
}

impl Writer {
    pub fn pad(&mut self, alignment: usize) {
        while !self.buf.len().is_multiple_of(alignment) {
            self.buf.push(0);
        }
    }

    pub fn write(&mut self, value: &Value) -> Result<(), WireError> {
        self.pad(value.value_type().alignment());
        match value {
            Value::Byte(b) => self.buf.push(*b),
            Value::Bool(b) => self.buf.extend_from_slice(&(*b as u32).to_le_bytes()),
            Value::I16(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::U16(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::I32(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::U32(n) | Value::UnixFd(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::I64(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::U64(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::Double(n) => self.buf.extend_from_slice(&n.to_le_bytes()),
            Value::Str(s) | Value::ObjectPath(s) => {
                if s.contains('\0') {
                    return Err(error("strings can't contain NUL"));
                }
                self.buf.extend_from_slice(&(s.len() as u32).to_le_bytes());
                self.buf.extend_from_slice(s.as_bytes());
                self.buf.push(0);
            }
            Value::Signature(s) => self.write_signature(s)?,
            Value::Variant(inner) => {
                self.write_signature(&inner.value_type().signature())?;
                self.write(inner)?;
            }
            Value::Array(element, items) => {
                let length_at = self.buf.len();
                self.buf.extend_from_slice(&[0, 0, 0, 0]);
                // The padding before the first element isn't part of the length.
                self.pad(element.alignment());
                let start = self.buf.len();
                for item in items {
                    if item.value_type() != *element {
                        return Err(error("array element of the wrong type"));
                    }
                    self.write(item)?;
                }
                let length = self.buf.len() - start;
                if length > MAX_ARRAY_BYTES {
                    return Err(error("array too long"));
                }
                self.buf[length_at..length_at + 4].copy_from_slice(&(length as u32).to_le_bytes());
            }
            Value::Struct(fields) => {
                for field in fields {
                    self.write(field)?;
                }
            }
            Value::DictEntry(key, value) => {
                self.write(key)?;
                self.write(value)?;
            }
        }
        return Ok(());
    }

    fn write_signature(&mut self, signature: &str) -> Result<(), WireError> {
        if signature.len() > 255 {
            return Err(error("signature longer than 255"));
        }
        self.buf.push(signature.len() as u8);
        self.buf.extend_from_slice(signature.as_bytes());
        self.buf.push(0);
        return Ok(());
    }
}

/// Reads values from a whole message. `pos` counts from the message start.
pub struct Reader<'a> {
    buf: &'a [u8],
    pub pos: usize,
    big_endian: bool,
}

impl<'a> Reader<'a> {
    pub fn new(buf: &'a [u8], big_endian: bool) -> Reader<'a> {
        return Reader {
            buf,
            pos: 0,
            big_endian,
        };
    }

    pub fn align(&mut self, alignment: usize) -> Result<(), WireError> {
        let padded = self.pos.div_ceil(alignment) * alignment;
        if padded > self.buf.len() {
            return Err(error("message ends in padding"));
        }
        self.pos = padded;
        return Ok(());
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], WireError> {
        let end = self
            .pos
            .checked_add(n)
            .ok_or_else(|| error("length overflow"))?;
        if end > self.buf.len() {
            return Err(error("message ends early"));
        }
        let bytes = &self.buf[self.pos..end];
        self.pos = end;
        return Ok(bytes);
    }

    fn fixed<const N: usize>(&mut self) -> Result<[u8; N], WireError> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        if self.big_endian {
            out.reverse();
        }
        // Now little-endian either way.
        return Ok(out);
    }

    pub fn read_u32(&mut self) -> Result<u32, WireError> {
        self.align(4)?;
        return Ok(u32::from_le_bytes(self.fixed::<4>()?));
    }

    pub fn read(&mut self, t: &Type) -> Result<Value, WireError> {
        return self.read_at_depth(t, 0);
    }

    fn read_at_depth(&mut self, t: &Type, depth: usize) -> Result<Value, WireError> {
        if depth > MAX_DEPTH {
            return Err(error("value nested too deep"));
        }
        self.align(t.alignment())?;
        let value = match t {
            Type::Byte => Value::Byte(self.take(1)?[0]),
            Type::Bool => match u32::from_le_bytes(self.fixed::<4>()?) {
                0 => Value::Bool(false),
                1 => Value::Bool(true),
                other => return Err(error(format!("boolean {other}"))),
            },
            Type::I16 => Value::I16(i16::from_le_bytes(self.fixed::<2>()?)),
            Type::U16 => Value::U16(u16::from_le_bytes(self.fixed::<2>()?)),
            Type::I32 => Value::I32(i32::from_le_bytes(self.fixed::<4>()?)),
            Type::U32 => Value::U32(u32::from_le_bytes(self.fixed::<4>()?)),
            Type::UnixFd => Value::UnixFd(u32::from_le_bytes(self.fixed::<4>()?)),
            Type::I64 => Value::I64(i64::from_le_bytes(self.fixed::<8>()?)),
            Type::U64 => Value::U64(u64::from_le_bytes(self.fixed::<8>()?)),
            Type::Double => Value::Double(f64::from_le_bytes(self.fixed::<8>()?)),
            Type::Str => Value::Str(self.read_string()?),
            Type::ObjectPath => Value::ObjectPath(self.read_string()?),
            Type::Signature => Value::Signature(self.read_signature()?),
            Type::Variant => {
                let signature = self.read_signature()?;
                let types = parse_signature(&signature)?;
                if types.len() != 1 {
                    return Err(error("variant must hold exactly one type"));
                }
                Value::Variant(Box::new(self.read_at_depth(&types[0], depth + 1)?))
            }
            Type::Array(element) => {
                let length = u32::from_le_bytes(self.fixed::<4>()?) as usize;
                if length > MAX_ARRAY_BYTES {
                    return Err(error("array too long"));
                }
                self.align(element.alignment())?;
                let end = self
                    .pos
                    .checked_add(length)
                    .ok_or_else(|| error("length overflow"))?;
                if end > self.buf.len() {
                    return Err(error("array longer than the message"));
                }
                let mut items = Vec::new();
                while self.pos < end {
                    items.push(self.read_at_depth(element, depth + 1)?);
                }
                if self.pos != end {
                    return Err(error("array elements overrun its length"));
                }
                Value::Array((**element).clone(), items)
            }
            Type::Struct(fields) => {
                let mut values = Vec::with_capacity(fields.len());
                for field in fields {
                    values.push(self.read_at_depth(field, depth + 1)?);
                }
                Value::Struct(values)
            }
            Type::DictEntry(key, value) => {
                let k = self.read_at_depth(key, depth + 1)?;
                let v = self.read_at_depth(value, depth + 1)?;
                Value::DictEntry(Box::new(k), Box::new(v))
            }
        };
        return Ok(value);
    }

    fn read_string(&mut self) -> Result<String, WireError> {
        let length = u32::from_le_bytes(self.fixed::<4>()?) as usize;
        let bytes = self.take(length)?;
        if self.take(1)?[0] != 0 {
            return Err(error("string not NUL-terminated"));
        }
        return match std::str::from_utf8(bytes) {
            Ok(s) if !s.contains('\0') => Ok(s.to_string()),
            _ => Err(error("string is not valid UTF-8")),
        };
    }

    fn read_signature(&mut self) -> Result<String, WireError> {
        let length = self.take(1)?[0] as usize;
        let bytes = self.take(length)?;
        if self.take(1)?[0] != 0 {
            return Err(error("signature not NUL-terminated"));
        }
        return match std::str::from_utf8(bytes) {
            Ok(s) => Ok(s.to_string()),
            Err(_) => Err(error("signature is not ASCII")),
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn bytes_of(value: &Value) -> Vec<u8> {
        let mut writer = Writer::default();
        writer.write(value).unwrap();
        return writer.buf;
    }

    fn round_trip(value: Value) {
        let bytes = bytes_of(&value);
        let mut reader = Reader::new(&bytes, false);
        assert_eq!(reader.read(&value.value_type()).unwrap(), value);
        assert_eq!(reader.pos, bytes.len());
    }

    #[test]
    fn signatures() {
        let types = parse_signature("sa{sv}a(sa(us))ay").unwrap();
        assert_eq!(types.len(), 4);
        assert_eq!(types[1].signature(), "a{sv}");
        assert_eq!(types[2].signature(), "a(sa(us))");
        for bad in ["a", "(", "()", "{sv}", "a{vs}", "a{s}", "a{sss}", "z", "(s"] {
            assert!(parse_signature(bad).is_err(), "{bad}");
        }
        assert!(parse_signature(&"a".repeat(100).to_string()).is_err());
    }

    #[test]
    fn known_layouts() {
        // (yu): byte, 3 bytes of padding, then the u32.
        let value = Value::Struct(vec![Value::Byte(1), Value::U32(2)]);
        assert_eq!(bytes_of(&value), [1, 0, 0, 0, 2, 0, 0, 0]);
        // An empty array of u64: length 0, then padding to 8 (not counted).
        let empty = Value::Array(Type::U64, Vec::new());
        assert_eq!(bytes_of(&empty), [0, 0, 0, 0, 0, 0, 0, 0]);
        // A string: length, bytes, NUL.
        assert_eq!(
            bytes_of(&Value::Str("hi".to_string())),
            [2, 0, 0, 0, b'h', b'i', 0]
        );
        // A variant holding a u32: signature "u", then padding to 4.
        let variant = Value::Variant(Box::new(Value::U32(7)));
        assert_eq!(bytes_of(&variant), [1, b'u', 0, 0, 7, 0, 0, 0]);
    }

    #[test]
    fn round_trips() {
        round_trip(Value::Bool(true));
        round_trip(Value::I16(-2));
        round_trip(Value::I64(-1));
        round_trip(Value::Double(1.5));
        round_trip(Value::ObjectPath(
            "/org/freedesktop/portal/desktop".to_string(),
        ));
        round_trip(Value::Signature("a{sv}".to_string()));
        round_trip(Value::bytes(b"/tmp/a\xff\0"));
        round_trip(Value::dict(vec![
            ("multiple", Value::Bool(true)),
            ("accept_label", Value::Str("Open".to_string())),
            ("current_folder", Value::bytes(b"/tmp\0")),
        ]));
        let filter = Value::Struct(vec![
            Value::Str("Images".to_string()),
            Value::Array(
                Type::Struct(vec![Type::U32, Type::Str]),
                vec![Value::Struct(vec![
                    Value::U32(0),
                    Value::Str("*.png".to_string()),
                ])],
            ),
        ]);
        round_trip(Value::Array(filter.value_type(), vec![filter]));
    }

    #[test]
    fn big_endian_input() {
        // u32 0x01020304 then the string "ab", big-endian.
        let bytes = [1, 2, 3, 4, 0, 0, 0, 2, b'a', b'b', 0];
        let mut reader = Reader::new(&bytes, true);
        assert_eq!(reader.read(&Type::U32).unwrap(), Value::U32(0x01020304));
        assert_eq!(
            reader.read(&Type::Str).unwrap(),
            Value::Str("ab".to_string())
        );
    }

    #[test]
    fn hostile_input() {
        // An array claiming 1 GiB.
        let bytes = [0, 0, 0, 0x40];
        assert!(
            Reader::new(&bytes, false)
                .read(&Type::Array(Box::new(Type::Byte)))
                .is_err()
        );
        // A string longer than the data.
        assert!(
            Reader::new(&[9, 0, 0, 0, b'a'], false)
                .read(&Type::Str)
                .is_err()
        );
        // Missing NUL, bad UTF-8, a boolean of 2.
        assert!(
            Reader::new(&[1, 0, 0, 0, b'a', b'x'], false)
                .read(&Type::Str)
                .is_err()
        );
        assert!(
            Reader::new(&[1, 0, 0, 0, 0xff, 0], false)
                .read(&Type::Str)
                .is_err()
        );
        assert!(Reader::new(&[2, 0, 0, 0], false).read(&Type::Bool).is_err());
        // A variant whose signature has two types.
        assert!(
            Reader::new(&[2, b'u', b'u', 0, 0, 0, 0, 0], false)
                .read(&Type::Variant)
                .is_err()
        );
        // Deeply nested variants.
        let mut nested = Value::U32(1);
        for _ in 0..100 {
            nested = Value::Variant(Box::new(nested));
        }
        let bytes = bytes_of(&nested);
        assert!(Reader::new(&bytes, false).read(&Type::Variant).is_err());
    }

    #[test]
    fn dict_helpers() {
        let dict = Value::dict(vec![
            ("directory", Value::Bool(true)),
            ("title", Value::Str("x".to_string())),
        ]);
        assert_eq!(
            dict.dict_get("directory").and_then(Value::as_bool),
            Some(true)
        );
        assert_eq!(dict.dict_get("title").and_then(Value::as_str), Some("x"));
        assert!(dict.dict_get("missing").is_none());
        assert_eq!(Value::bytes(b"ab").as_bytes(), Some(b"ab".to_vec()));
    }
}
