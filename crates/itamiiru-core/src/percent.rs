//! Percent-encoding and `file://` URIs, working on raw bytes so non-UTF-8
//! paths survive the round trip (PLAN.md section 5).

use std::path::{Path, PathBuf};

const HEX: &[u8; 16] = b"0123456789ABCDEF";

/// Encodes every byte except `A-Z a-z 0-9 - . _ ~ /`. That covers what the
/// picker protocol requires (space, `%`, control bytes, bytes >= 0x80) and
/// what file URIs and `.trashinfo` files need.
pub fn encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b.is_ascii_alphanumeric()
            || b == b'-'
            || b == b'.'
            || b == b'_'
            || b == b'~'
            || b == b'/'
        {
            out.push(b as char);
        } else {
            out.push('%');
            out.push(HEX[(b >> 4) as usize] as char);
            out.push(HEX[(b & 0x0f) as usize] as char);
        }
    }
    return out;
}

#[derive(Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// A `%` not followed by two hex digits, at this byte offset.
    BadEscape(usize),
}

/// Decodes every `%XX`. Other bytes are copied as they are.
pub fn decode(text: &str) -> Result<Vec<u8>, DecodeError> {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] != b'%' {
            out.push(bytes[i]);
            i += 1;
            continue;
        }
        if i + 2 >= bytes.len() {
            return Err(DecodeError::BadEscape(i));
        }
        let (Some(high), Some(low)) = (hex_value(bytes[i + 1]), hex_value(bytes[i + 2])) else {
            return Err(DecodeError::BadEscape(i));
        };
        out.push(high * 16 + low);
        i += 3;
    }
    return Ok(out);
}

fn hex_value(b: u8) -> Option<u8> {
    return match b {
        b'0'..=b'9' => Some(b - b'0'),
        b'a'..=b'f' => Some(b - b'a' + 10),
        b'A'..=b'F' => Some(b - b'A' + 10),
        _ => None,
    };
}

/// `/home/me/a b` becomes `file:///home/me/a%20b`.
pub fn path_to_file_uri(path: &Path) -> String {
    return format!("file://{}", encode(path.as_os_str().as_encoded_bytes()));
}

/// The reverse of `path_to_file_uri`. Accepts `file:///path` and
/// `file://localhost/path`; anything else (other hosts, relative paths,
/// bad escapes) is None.
pub fn file_uri_to_path(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let bytes = decode(rest).ok()?;
    return crate::platform::path_from_bytes(bytes);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_what_it_must() {
        assert_eq!(encode(b"abc-._~/XYZ09"), "abc-._~/XYZ09");
        assert_eq!(encode(b"a b%c"), "a%20b%25c");
        assert_eq!(encode(b"\x00\x1f\x7f\x80\xff"), "%00%1F%7F%80%FF");
        assert_eq!(encode(b"#?&=+"), "%23%3F%26%3D%2B");
        assert_eq!(encode(b""), "");
    }

    #[test]
    fn decodes_and_round_trips_every_byte() {
        let mut all = Vec::new();
        for b in 0..=255u8 {
            all.push(b);
        }
        assert_eq!(decode(&encode(&all)), Ok(all));
        assert_eq!(decode("a%2fb%2Fc"), Ok(b"a/b/c".to_vec()));
        assert_eq!(decode("plain text"), Ok(b"plain text".to_vec()));
    }

    #[test]
    fn bad_escapes() {
        assert_eq!(decode("%"), Err(DecodeError::BadEscape(0)));
        assert_eq!(decode("ab%4"), Err(DecodeError::BadEscape(2)));
        assert_eq!(decode("%zz"), Err(DecodeError::BadEscape(0)));
        assert_eq!(decode("%+1"), Err(DecodeError::BadEscape(0)));
    }

    #[test]
    fn file_uris() {
        let path = Path::new("/home/me/My Files/a#1.txt");
        let uri = path_to_file_uri(path);
        assert_eq!(uri, "file:///home/me/My%20Files/a%231.txt");
        assert_eq!(file_uri_to_path(&uri).as_deref(), Some(path));
        assert_eq!(
            file_uri_to_path("file://localhost/tmp/x").as_deref(),
            Some(Path::new("/tmp/x"))
        );
        assert_eq!(file_uri_to_path("file://otherhost/tmp/x"), None);
        assert_eq!(file_uri_to_path("http://example.com/x"), None);
        assert_eq!(file_uri_to_path("file://relative"), None);
        assert_eq!(file_uri_to_path("file:///bad%zz"), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_uri_round_trip() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;
        let path = PathBuf::from(OsString::from_vec(b"/tmp/bad\xffname".to_vec()));
        let uri = path_to_file_uri(&path);
        assert_eq!(uri, "file:///tmp/bad%FFname");
        assert_eq!(file_uri_to_path(&uri), Some(path));
    }
}
