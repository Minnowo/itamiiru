//! The picker <-> portal protocol, v1 (spec in PLAN.md).
//!
//! One record per line: a keyword, then fields separated by single spaces.
//! Every field is percent-encoded; the token `-` is the empty string.
//! Requests come from another process, so they're treated as hostile:
//! sizes are capped and nothing here panics on bad input.

use std::path::PathBuf;

use crate::percent;

pub const HEADER: &str = "itamiiru-protocol 1";
/// Requests bigger than this are refused.
pub const MAX_REQUEST_SIZE: usize = 1024 * 1024;
const MAX_FILTERS: usize = 256;
const MAX_PATTERNS: usize = 256;
const MAX_CHOICES: usize = 64;
const MAX_OPTIONS: usize = 256;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Open,
    Save,
    /// Pick a folder; the portal adds the file names (SaveFiles).
    SaveFiles,
    Folder,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pattern {
    Glob(String),
    Mime(String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    pub name: String,
    pub patterns: Vec<Pattern>,
}

/// An extra control in the footer. No options means a checkbox whose
/// value is "true" or "false".
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice {
    pub id: String,
    pub label: String,
    pub default: String,
    /// (option id, option label)
    pub options: Vec<(String, String)>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Request {
    pub mode: Mode,
    pub title: String,
    pub accept_label: Option<String>,
    pub multiple: bool,
    pub modal: bool,
    pub app_id: String,
    pub parent: String,
    pub folder: Option<PathBuf>,
    pub name: Option<String>,
    pub file: Option<PathBuf>,
    pub filters: Vec<Filter>,
    pub current_filter: Option<usize>,
    pub choices: Vec<Choice>,
}

impl Request {
    pub fn new(mode: Mode) -> Request {
        return Request {
            mode,
            title: String::new(),
            accept_label: None,
            multiple: false,
            modal: false,
            app_id: String::new(),
            parent: String::new(),
            folder: None,
            name: None,
            file: None,
            filters: Vec::new(),
            current_filter: None,
            choices: Vec::new(),
        };
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Response {
    pub uris: Vec<String>,
    pub filter: Option<usize>,
    /// (choice id, value)
    pub choices: Vec<(String, String)>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ProtocolError {
    TooLarge,
    /// The first line isn't the protocol header.
    BadHeader,
    /// No `end` line (the request was cut off).
    MissingEnd,
    MissingMode,
    /// Line number (1-based) and what's wrong with it.
    BadLine(usize, String),
}

impl std::fmt::Display for ProtocolError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        return match self {
            ProtocolError::TooLarge => write!(f, "request is too large"),
            ProtocolError::BadHeader => write!(f, "first line must be \"{HEADER}\""),
            ProtocolError::MissingEnd => write!(f, "request has no \"end\" line"),
            ProtocolError::MissingMode => write!(f, "request has no \"mode\" line"),
            ProtocolError::BadLine(line, reason) => write!(f, "line {line}: {reason}"),
        };
    }
}

impl std::error::Error for ProtocolError {}

/// Encodes one field: space, `%`, control bytes and bytes >= 0x80 as `%XX`;
/// the empty string as `-`, and a literal `-` as `%2D`.
pub fn encode_field(bytes: &[u8]) -> String {
    if bytes.is_empty() {
        return "-".to_string();
    }
    if bytes == b"-" {
        return "%2D".to_string();
    }
    let mut out = String::with_capacity(bytes.len());
    for &b in bytes {
        if b <= b' ' || b == b'%' || b >= 0x7f {
            out.push_str(&format!("%{b:02X}"));
        } else {
            out.push(b as char);
        }
    }
    return out;
}

/// Decodes one field to bytes.
pub fn decode_field(token: &str) -> Result<Vec<u8>, String> {
    if token == "-" {
        return Ok(Vec::new());
    }
    return match percent::decode(token) {
        Ok(bytes) => Ok(bytes),
        Err(_) => Err(format!("bad %-escape in {token:?}")),
    };
}

fn decode_text(token: &str) -> Result<String, String> {
    let bytes = decode_field(token)?;
    return match String::from_utf8(bytes) {
        Ok(text) => Ok(text),
        Err(_) => Err(format!("{token:?} is not UTF-8 text")),
    };
}

fn decode_path(token: &str) -> Result<PathBuf, String> {
    let bytes = decode_field(token)?;
    if bytes.first() != Some(&b'/') && !cfg!(windows) {
        return Err(format!("{token:?} is not an absolute path"));
    }
    return match crate::platform::path_from_bytes(bytes) {
        Some(path) => Ok(path),
        None => Err(format!("{token:?} is not a valid path")),
    };
}

fn decode_bool(token: &str) -> Result<bool, String> {
    return match token {
        "0" => Ok(false),
        "1" => Ok(true),
        _ => Err(format!("expected 0 or 1, got {token:?}")),
    };
}

/// One field per argument, e.g. `fields_line("title", &["My Title"])`.
fn line(keyword: &str, fields: &[&[u8]]) -> String {
    let mut out = keyword.to_string();
    for field in fields {
        out.push(' ');
        out.push_str(&encode_field(field));
    }
    out.push('\n');
    return out;
}

pub fn parse_request(text: &str) -> Result<Request, ProtocolError> {
    if text.len() > MAX_REQUEST_SIZE {
        return Err(ProtocolError::TooLarge);
    }
    let mut lines = text.lines();
    if lines.next().map(|l| l.trim_end_matches('\r')) != Some(HEADER) {
        return Err(ProtocolError::BadHeader);
    }
    let mut request = Request::new(Mode::Open);
    let mut have_mode = false;
    let mut ended = false;

    for (index, raw) in lines.enumerate() {
        let number = index + 2;
        let raw = raw.trim_end_matches('\r');
        if raw.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = raw.split(' ').collect();
        let keyword = tokens[0];
        let fields = &tokens[1..];
        let bad = |reason: String| ProtocolError::BadLine(number, reason);
        let one = || -> Result<&str, ProtocolError> {
            if fields.len() != 1 {
                return Err(ProtocolError::BadLine(
                    number,
                    format!("{keyword} takes one field"),
                ));
            }
            return Ok(fields[0]);
        };
        match keyword {
            "end" => {
                ended = true;
                break;
            }
            "mode" => {
                request.mode = match one()? {
                    "open" => Mode::Open,
                    "save" => Mode::Save,
                    "save-files" => Mode::SaveFiles,
                    "folder" => Mode::Folder,
                    other => return Err(bad(format!("unknown mode {other:?}"))),
                };
                have_mode = true;
            }
            "title" => request.title = decode_text(one()?).map_err(bad)?,
            "accept_label" => request.accept_label = Some(decode_text(one()?).map_err(bad)?),
            "multiple" => request.multiple = decode_bool(one()?).map_err(bad)?,
            "modal" => request.modal = decode_bool(one()?).map_err(bad)?,
            "app_id" => request.app_id = decode_text(one()?).map_err(bad)?,
            "parent" => request.parent = decode_text(one()?).map_err(bad)?,
            "folder" => request.folder = Some(decode_path(one()?).map_err(bad)?),
            "name" => request.name = Some(decode_text(one()?).map_err(bad)?),
            "file" => request.file = Some(decode_path(one()?).map_err(bad)?),
            "current_filter" => match one()?.parse::<usize>() {
                Ok(index) => request.current_filter = Some(index),
                Err(_) => return Err(bad("current_filter must be a number".to_string())),
            },
            "filter" => {
                if fields.len() < 2 {
                    return Err(bad(
                        "filter needs a name and at least one pattern".to_string()
                    ));
                }
                if request.filters.len() >= MAX_FILTERS || fields.len() - 1 > MAX_PATTERNS {
                    return Err(bad("too many filters or patterns".to_string()));
                }
                let name = decode_text(fields[0]).map_err(bad)?;
                let mut patterns = Vec::new();
                for field in &fields[1..] {
                    let pattern = decode_text(field).map_err(bad)?;
                    if let Some(glob) = pattern.strip_prefix("glob:") {
                        patterns.push(Pattern::Glob(glob.to_string()));
                    } else if let Some(mime) = pattern.strip_prefix("mime:") {
                        patterns.push(Pattern::Mime(mime.to_string()));
                    } else {
                        return Err(bad(format!("unknown pattern {pattern:?}")));
                    }
                }
                request.filters.push(Filter { name, patterns });
            }
            "choice" => {
                if fields.len() < 3 || !(fields.len() - 3).is_multiple_of(2) {
                    return Err(bad(
                        "choice needs id, label, default, then pairs".to_string()
                    ));
                }
                if request.choices.len() >= MAX_CHOICES || (fields.len() - 3) / 2 > MAX_OPTIONS {
                    return Err(bad("too many choices or options".to_string()));
                }
                let mut options = Vec::new();
                let mut i = 3;
                while i + 1 < fields.len() {
                    let id = decode_text(fields[i]).map_err(bad)?;
                    let label = decode_text(fields[i + 1]).map_err(bad)?;
                    options.push((id, label));
                    i += 2;
                }
                request.choices.push(Choice {
                    id: decode_text(fields[0]).map_err(bad)?,
                    label: decode_text(fields[1]).map_err(bad)?,
                    default: decode_text(fields[2]).map_err(bad)?,
                    options,
                });
            }
            // Unknown keywords are ignored, for forward compatibility.
            _ => {}
        }
    }

    if !ended {
        return Err(ProtocolError::MissingEnd);
    }
    if !have_mode {
        return Err(ProtocolError::MissingMode);
    }
    // An out-of-range current filter is ignored rather than refused.
    if let Some(index) = request.current_filter
        && index >= request.filters.len()
    {
        request.current_filter = None;
    }
    return Ok(request);
}

pub fn write_request(request: &Request) -> String {
    let mut out = format!("{HEADER}\n");
    let mode = match request.mode {
        Mode::Open => "open",
        Mode::Save => "save",
        Mode::SaveFiles => "save-files",
        Mode::Folder => "folder",
    };
    out.push_str(&format!("mode {mode}\n"));
    out.push_str(&line("title", &[request.title.as_bytes()]));
    if let Some(label) = &request.accept_label {
        out.push_str(&line("accept_label", &[label.as_bytes()]));
    }
    out.push_str(&format!("multiple {}\n", request.multiple as u8));
    out.push_str(&format!("modal {}\n", request.modal as u8));
    out.push_str(&line("app_id", &[request.app_id.as_bytes()]));
    out.push_str(&line("parent", &[request.parent.as_bytes()]));
    if let Some(folder) = &request.folder {
        out.push_str(&line("folder", &[folder.as_os_str().as_encoded_bytes()]));
    }
    if let Some(name) = &request.name {
        out.push_str(&line("name", &[name.as_bytes()]));
    }
    if let Some(file) = &request.file {
        out.push_str(&line("file", &[file.as_os_str().as_encoded_bytes()]));
    }
    for filter in &request.filters {
        let mut patterns: Vec<String> = Vec::new();
        for pattern in &filter.patterns {
            patterns.push(match pattern {
                Pattern::Glob(glob) => format!("glob:{glob}"),
                Pattern::Mime(mime) => format!("mime:{mime}"),
            });
        }
        let mut fields: Vec<&[u8]> = vec![filter.name.as_bytes()];
        for pattern in &patterns {
            fields.push(pattern.as_bytes());
        }
        out.push_str(&line("filter", &fields));
    }
    if let Some(index) = request.current_filter {
        out.push_str(&format!("current_filter {index}\n"));
    }
    for choice in &request.choices {
        let mut fields: Vec<&[u8]> = vec![
            choice.id.as_bytes(),
            choice.label.as_bytes(),
            choice.default.as_bytes(),
        ];
        for (id, label) in &choice.options {
            fields.push(id.as_bytes());
            fields.push(label.as_bytes());
        }
        out.push_str(&line("choice", &fields));
    }
    out.push_str("end\n");
    return out;
}

pub fn write_response(response: &Response) -> String {
    let mut out = format!("{HEADER}\n");
    for uri in &response.uris {
        out.push_str(&line("uri", &[uri.as_bytes()]));
    }
    if let Some(index) = response.filter {
        out.push_str(&format!("filter {index}\n"));
    }
    for (id, value) in &response.choices {
        out.push_str(&line("choice", &[id.as_bytes(), value.as_bytes()]));
    }
    return out;
}

pub fn parse_response(text: &str) -> Result<Response, ProtocolError> {
    if text.len() > MAX_REQUEST_SIZE {
        return Err(ProtocolError::TooLarge);
    }
    let mut lines = text.lines();
    if lines.next().map(|l| l.trim_end_matches('\r')) != Some(HEADER) {
        return Err(ProtocolError::BadHeader);
    }
    let mut response = Response::default();
    for (index, raw) in lines.enumerate() {
        let number = index + 2;
        let raw = raw.trim_end_matches('\r');
        if raw.is_empty() {
            continue;
        }
        let tokens: Vec<&str> = raw.split(' ').collect();
        let bad = |reason: String| ProtocolError::BadLine(number, reason);
        match tokens[0] {
            "uri" if tokens.len() == 2 => response.uris.push(decode_text(tokens[1]).map_err(bad)?),
            "filter" if tokens.len() == 2 => match tokens[1].parse::<usize>() {
                Ok(index) => response.filter = Some(index),
                Err(_) => return Err(bad("filter must be a number".to_string())),
            },
            "choice" if tokens.len() == 3 => {
                let id = decode_text(tokens[1]).map_err(bad)?;
                let value = decode_text(tokens[2]).map_err(bad)?;
                response.choices.push((id, value));
            }
            "uri" | "filter" | "choice" => return Err(bad("wrong number of fields".to_string())),
            _ => {}
        }
    }
    return Ok(response);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn field_encoding() {
        assert_eq!(encode_field(b""), "-");
        assert_eq!(encode_field(b"-"), "%2D");
        assert_eq!(encode_field(b"a-b"), "a-b");
        assert_eq!(encode_field(b"My Title"), "My%20Title");
        assert_eq!(encode_field(b"100%"), "100%25");
        assert_eq!(encode_field(b"tab\there\x7f\xff"), "tab%09here%7F%FF");
        assert_eq!(decode_field("-"), Ok(Vec::new()));
        assert_eq!(decode_field("%2D"), Ok(b"-".to_vec()));
        assert_eq!(decode_field("a%20b"), Ok(b"a b".to_vec()));
        assert!(decode_field("%zz").is_err());
    }

    #[test]
    fn sample_requests_parse() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/requests");
        let open =
            parse_request(&std::fs::read_to_string(dir.join("open-images.req")).unwrap()).unwrap();
        assert_eq!(open.mode, Mode::Open);
        assert_eq!(open.title, "Open Images");
        assert!(open.multiple);
        assert_eq!(open.filters.len(), 2);
        assert_eq!(
            open.filters[0].patterns[3],
            Pattern::Mime("image/webp".to_string())
        );
        assert_eq!(open.current_filter, Some(0));

        let save =
            parse_request(&std::fs::read_to_string(dir.join("save-text.req")).unwrap()).unwrap();
        assert_eq!(save.mode, Mode::Save);
        assert_eq!(save.folder, Some(PathBuf::from("/tmp")));
        assert_eq!(save.name.as_deref(), Some("untitled.txt"));
        assert_eq!(save.choices.len(), 2);
        assert_eq!(save.choices[0].options.len(), 2);
        assert_eq!(save.choices[1].options.len(), 0, "a checkbox");
        assert_eq!(save.choices[1].label, "Add byte order mark");

        let folder =
            parse_request(&std::fs::read_to_string(dir.join("pick-folder.req")).unwrap()).unwrap();
        assert_eq!(folder.mode, Mode::Folder);
        assert_eq!(folder.accept_label.as_deref(), Some("Select Folder"));
    }

    #[test]
    fn request_round_trip() {
        let mut request = Request::new(Mode::Save);
        request.title = "Save - as".to_string();
        request.accept_label = Some("-".to_string());
        request.app_id = String::new();
        request.folder = Some(PathBuf::from("/tmp/a b"));
        request.name = Some("100% done.txt".to_string());
        request.filters.push(Filter {
            name: "Text".to_string(),
            patterns: vec![
                Pattern::Glob("*.txt".to_string()),
                Pattern::Mime("text/plain".to_string()),
            ],
        });
        request.current_filter = Some(0);
        request.choices.push(Choice {
            id: "enc".to_string(),
            label: "Encoding".to_string(),
            default: "utf8".to_string(),
            options: vec![("utf8".to_string(), "UTF-8".to_string())],
        });
        let text = write_request(&request);
        assert_eq!(parse_request(&text), Ok(request));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn non_utf8_paths_round_trip() {
        use std::os::unix::ffi::OsStringExt;
        let mut request = Request::new(Mode::Open);
        request.folder = Some(PathBuf::from(std::ffi::OsString::from_vec(
            b"/tmp/bad\xffname".to_vec(),
        )));
        let text = write_request(&request);
        assert!(text.contains("folder /tmp/bad%FFname\n"), "{text}");
        assert_eq!(parse_request(&text), Ok(request));
    }

    #[test]
    fn response_round_trip() {
        let response = Response {
            uris: vec![
                "file:///home/me/a%20b.txt".to_string(),
                "file:///x".to_string(),
            ],
            filter: Some(1),
            choices: vec![
                ("enc".to_string(), "latin1".to_string()),
                ("bom".to_string(), "false".to_string()),
            ],
        };
        let text = write_response(&response);
        assert!(
            text.starts_with("itamiiru-protocol 1\nuri file:///home/me/a%2520b.txt\n"),
            "{text}"
        );
        assert_eq!(parse_response(&text), Ok(response));
    }

    #[test]
    fn hostile_requests_are_refused() {
        assert_eq!(parse_request(""), Err(ProtocolError::BadHeader));
        assert_eq!(
            parse_request("itamiiru-protocol 2\nmode open\nend\n"),
            Err(ProtocolError::BadHeader)
        );
        assert_eq!(
            parse_request("itamiiru-protocol 1\nmode open\n"),
            Err(ProtocolError::MissingEnd)
        );
        assert_eq!(
            parse_request("itamiiru-protocol 1\nend\n"),
            Err(ProtocolError::MissingMode)
        );
        let bad = [
            "mode sideways",
            "mode",
            "multiple yes",
            "folder relative/path",
            "title %zz",
            "filter OnlyName",
            "filter Name weird:thing",
            "choice id label default oneoption",
            "current_filter x",
        ];
        for line in bad {
            let text = format!("itamiiru-protocol 1\nmode open\n{line}\nend\n");
            assert!(
                matches!(parse_request(&text), Err(ProtocolError::BadLine(3, _))),
                "{line}"
            );
        }
        let huge = format!("itamiiru-protocol 1\n{}", "x".repeat(MAX_REQUEST_SIZE));
        assert_eq!(parse_request(&huge), Err(ProtocolError::TooLarge));
    }

    #[test]
    fn unknown_keywords_and_bad_filter_index_are_tolerated() {
        let text = "itamiiru-protocol 1\nmode open\nfuture_thing a b c\ncurrent_filter 5\nend\n";
        let request = parse_request(text).unwrap();
        assert_eq!(request.current_filter, None);
    }
}
