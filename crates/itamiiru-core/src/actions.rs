//! Custom right-click actions, from the `[[action]]` tables in
//! config.toml. Each has a name, a command and what it applies to:
//!
//! ```text
//! [[action]]
//! name = "Open in terminal"
//! command = "xfce4-terminal --working-directory=%d"
//! for = ["folders", "background"]
//! ```
//!
//! The command is split into words like a shell would (quotes and
//! backslashes), but never run through a shell: each placeholder becomes
//! part of one argument, so a file named `a; rm -rf ~` is just a name.
//!
//! Placeholders: `%f` the selected path (with several selected, the
//! command runs once for each), `%F` every selected path as separate
//! arguments (once), `%d` the folder the action is about (a right-clicked
//! folder itself, otherwise the folder on screen), `%n` the selected name,
//! `%%` a percent sign.
//!
//! `for` takes `files`, `folders`, `background` (right-click on empty
//! space) and name patterns like `*.jpg`; an action shows when every
//! selected item matches one of them. Without `for`: files and folders.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use toml_edit::{Item as TomlItem, Table};

use crate::config::Config;
use crate::glob::Glob;

/// More actions than this would make a useless menu.
const MAX_ACTIONS: usize = 50;
/// Words in one command.
const MAX_WORDS: usize = 64;
/// Characters in an action's name.
const MAX_NAME_CHARS: usize = 80;

#[derive(Debug)]
pub struct Action {
    pub name: String,
    words: Vec<String>,
    targets: Vec<Target>,
}

#[derive(Debug)]
enum Target {
    Files,
    Folders,
    Background,
    Pattern(Glob),
}

/// One selected item: its path, and whether it's a folder.
#[derive(Clone, Copy, Debug)]
pub struct Item<'a> {
    pub path: &'a Path,
    pub is_dir: bool,
}

/// One command line to run: the program and its arguments.
pub type CommandLine = Vec<OsString>;

/// The actions in config.toml, and a list of problems (actions that
/// were skipped, and why). Touches no files.
pub fn from_config(config: &Config) -> (Vec<Action>, Vec<String>) {
    let mut actions = Vec::new();
    let mut problems = Vec::new();
    for (index, table) in config.base_tables("action").into_iter().enumerate() {
        match parse_action(table) {
            Ok(action) => actions.push(action),
            Err(err) => problems.push(format!("action {}: {err}", index + 1)),
        }
    }
    if actions.len() > MAX_ACTIONS {
        problems.push(format!("only the first {MAX_ACTIONS} actions are used"));
        actions.truncate(MAX_ACTIONS);
    }
    return (actions, problems);
}

/// One `[[action]]` table.
fn parse_action(table: &Table) -> Result<Action, String> {
    let mut name = None;
    let mut command = None;
    let mut targets = None;
    for (key, item) in table.iter() {
        match key {
            "name" => name = Some(string_value(key, item)?),
            "command" => command = Some(string_value(key, item)?),
            "for" => targets = Some(parse_targets(item)?),
            other => return Err(format!("unknown key {other:?}")),
        }
    }
    let Some(name) = name else {
        return Err("no name".to_string());
    };
    if name.is_empty() || name.chars().count() > MAX_NAME_CHARS {
        return Err(format!("the name must be 1 to {MAX_NAME_CHARS} characters"));
    }
    let Some(command) = command else {
        return Err(format!("{name:?} has no command"));
    };
    let words = split_words(&command).map_err(|err| format!("{name:?}: {err}"))?;
    if words.is_empty() || words.len() > MAX_WORDS {
        return Err(format!(
            "{name:?}: the command must have 1 to {MAX_WORDS} words"
        ));
    }
    let targets = match targets {
        Some(targets) => targets,
        None => vec![Target::Files, Target::Folders],
    };
    return Ok(Action {
        name,
        words,
        targets,
    });
}

fn string_value(key: &str, item: &TomlItem) -> Result<String, String> {
    return match item.as_str() {
        Some(text) => Ok(text.to_string()),
        None => Err(format!("{key} must be a string")),
    };
}

/// `for = ["files", "*.jpg"]`.
fn parse_targets(item: &TomlItem) -> Result<Vec<Target>, String> {
    let Some(array) = item.as_array() else {
        return Err("for must be a list like [\"files\", \"folders\"]".to_string());
    };
    let mut targets = Vec::new();
    for value in array.iter() {
        let Some(part) = value.as_str() else {
            return Err("for must hold strings".to_string());
        };
        let target = match part.trim() {
            "files" => Target::Files,
            "folders" => Target::Folders,
            "background" => Target::Background,
            part if Glob::has_wildcards(part) => Target::Pattern(Glob::case_insensitive(part)),
            part => return Err(format!("unknown target {part:?} in for")),
        };
        targets.push(target);
    }
    if targets.is_empty() {
        return Err("for is empty".to_string());
    }
    return Ok(targets);
}

/// Splits a command like a shell does, without running one: spaces
/// separate words; "double" and 'single' quotes keep spaces; a backslash
/// outside single quotes takes the next character as it is.
pub fn split_words(command: &str) -> Result<Vec<String>, String> {
    let mut words = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = command.chars();
    while let Some(c) = chars.next() {
        match c {
            ' ' | '\t' => {
                if in_word {
                    words.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(inner) => word.push(inner),
                        None => return Err("a ' quote isn't closed".to_string()),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(escaped) => word.push(escaped),
                            None => return Err("a \" quote isn't closed".to_string()),
                        },
                        Some(inner) => word.push(inner),
                        None => return Err("a \" quote isn't closed".to_string()),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(escaped) = chars.next() {
                    word.push(escaped);
                }
            }
            _ => {
                in_word = true;
                word.push(c);
            }
        }
    }
    if in_word {
        words.push(word);
    }
    return Ok(words);
}

impl Action {
    /// Whether the action belongs in the menu for these items. No items
    /// means a right-click on empty space.
    pub fn applies_to(&self, items: &[Item]) -> bool {
        if items.is_empty() {
            return self.targets.iter().any(|t| matches!(t, Target::Background));
        }
        for item in items {
            if !self.targets.iter().any(|target| target.matches(item)) {
                return false;
            }
        }
        return true;
    }

    /// The command lines to run for these items (empty: the folder on
    /// screen, `current`). Several when the command uses `%f` or `%n` and
    /// several items are selected: one per item.
    pub fn command_lines(&self, items: &[Item], current: &Path) -> Vec<CommandLine> {
        let per_item = self
            .words
            .iter()
            .any(|word| has_placeholder(word, 'f') || has_placeholder(word, 'n'));
        if per_item && items.len() > 1 {
            let mut lines = Vec::new();
            for item in items {
                lines.push(self.expand(std::slice::from_ref(item), items, current));
            }
            return lines;
        }
        return vec![self.expand(items, items, current)];
    }

    /// One command line. `one` is the item for `%f`, `%n` and `%d`; `all`
    /// the items for `%F`.
    fn expand(&self, one: &[Item], all: &[Item], current: &Path) -> CommandLine {
        let first = one.first();
        // %d: a folder is its own; a file's is the one it's in.
        let folder: PathBuf = match first {
            Some(item) if item.is_dir => item.path.to_path_buf(),
            Some(item) => match item.path.parent() {
                Some(parent) => parent.to_path_buf(),
                None => current.to_path_buf(),
            },
            None => current.to_path_buf(),
        };
        let mut line = Vec::new();
        for word in &self.words {
            if word == "%F" {
                for item in all {
                    line.push(item.path.as_os_str().to_os_string());
                }
                continue;
            }
            line.push(expand_word(word, first, &folder));
        }
        return line;
    }
}

impl Target {
    fn matches(&self, item: &Item) -> bool {
        return match self {
            Target::Files => !item.is_dir,
            Target::Folders => item.is_dir,
            Target::Background => false,
            Target::Pattern(glob) => match item.path.file_name() {
                Some(name) => glob.matches(&name.to_string_lossy()),
                None => false,
            },
        };
    }
}

/// `%x` in `word`, not counting `%%x`.
fn has_placeholder(word: &str, letter: char) -> bool {
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            match chars.next() {
                Some(next) if next == letter => return true,
                _ => {}
            }
        }
    }
    return false;
}

/// One word with its placeholders filled in. Paths stay as OS strings,
/// so names that aren't valid UTF-8 pass through unchanged.
fn expand_word(word: &str, item: Option<&Item>, folder: &Path) -> OsString {
    let mut out = OsString::new();
    let mut text = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        if c != '%' {
            text.push(c);
            continue;
        }
        let next = chars.next();
        let piece: Option<OsString> = match next {
            Some('%') => {
                text.push('%');
                None
            }
            // A %F inside a longer word: the first item, like %f.
            Some('f') | Some('F') => item.map(|item| item.path.as_os_str().to_os_string()),
            Some('n') => item.and_then(|item| item.path.file_name().map(|n| n.to_os_string())),
            Some('d') => Some(folder.as_os_str().to_os_string()),
            Some(other) => {
                // Not a placeholder: keep it as written.
                text.push('%');
                text.push(other);
                None
            }
            None => {
                text.push('%');
                None
            }
        };
        if let Some(piece) = piece {
            out.push(&text);
            text.clear();
            out.push(piece);
        }
    }
    out.push(&text);
    return out;
}

/// Starts a command line in `folder` without waiting for it, with no
/// stdin and its output discarded. A thread waits for it so it doesn't
/// linger as a zombie.
pub fn run(line: &CommandLine, folder: &Path) -> std::io::Result<()> {
    use std::process::{Command, Stdio};
    let Some((program, args)) = line.split_first() else {
        return Ok(());
    };
    let mut child = Command::new(program)
        .args(args)
        .current_dir(folder)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    return Ok(());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(line: &CommandLine) -> Vec<String> {
        let mut out = Vec::new();
        for arg in line {
            out.push(arg.to_string_lossy().into_owned());
        }
        return out;
    }

    fn file(path: &str) -> Item<'_> {
        return Item {
            path: Path::new(path),
            is_dir: false,
        };
    }

    fn folder(path: &str) -> Item<'_> {
        return Item {
            path: Path::new(path),
            is_dir: true,
        };
    }

    const SAMPLE: &str = r#"
# Comments and blank lines are fine.
[[action]]
name = "Open in terminal"
command = "xfce4-terminal --working-directory=%d"
for = ["folders", "background"]

[[action]]
name = "Edit"
command = "codium %F"

[[action]]
name = "Convert to PNG"
command = 'convert %f "%f.png"'
for = ["*.jpg", "*.WEBP"]
"#;

    #[test]
    fn parses_sections() {
        let (actions, problems) = parse(SAMPLE);
        assert!(problems.is_empty(), "{problems:?}");
        let names: Vec<&str> = actions.iter().map(|a| a.name.as_str()).collect();
        assert_eq!(names, ["Open in terminal", "Edit", "Convert to PNG"]);
    }

    #[test]
    fn what_each_action_applies_to() {
        let (actions, _) = parse(SAMPLE);
        let (terminal, edit, convert) = (&actions[0], &actions[1], &actions[2]);
        assert!(terminal.applies_to(&[]));
        assert!(terminal.applies_to(&[folder("/a")]));
        assert!(!terminal.applies_to(&[file("/a.txt")]));
        assert!(!terminal.applies_to(&[folder("/a"), file("/b.txt")]));
        // No `for`: files and folders, not empty space.
        assert!(edit.applies_to(&[file("/a.txt"), folder("/b")]));
        assert!(!edit.applies_to(&[]));
        // Patterns ignore case.
        assert!(convert.applies_to(&[file("/x/Photo.JPG"), file("/x/b.webp")]));
        assert!(!convert.applies_to(&[file("/x/Photo.JPG"), file("/x/c.png")]));
    }

    #[test]
    fn placeholders() {
        let (actions, _) = parse(SAMPLE);
        let (terminal, edit, convert) = (&actions[0], &actions[1], &actions[2]);
        let current = Path::new("/home/me");
        // %d: a right-clicked folder itself; empty space: the folder shown.
        assert_eq!(
            strings(&terminal.command_lines(&[folder("/home/me/src")], current)[0]),
            ["xfce4-terminal", "--working-directory=/home/me/src"]
        );
        assert_eq!(
            strings(&terminal.command_lines(&[], current)[0]),
            ["xfce4-terminal", "--working-directory=/home/me"]
        );
        // %F: every path as its own argument, once.
        let both = [file("/home/me/a b.txt"), file("/home/me/c.txt")];
        let lines = edit.command_lines(&both, current);
        assert_eq!(lines.len(), 1);
        assert_eq!(
            strings(&lines[0]),
            ["codium", "/home/me/a b.txt", "/home/me/c.txt"]
        );
        // %f with several selected: once per item.
        let photos = [file("/p/a.jpg"), file("/p/b.jpg")];
        let lines = convert.command_lines(&photos, current);
        assert_eq!(lines.len(), 2);
        assert_eq!(strings(&lines[1]), ["convert", "/p/b.jpg", "/p/b.jpg.png"]);
    }

    #[test]
    fn names_are_arguments_not_shell_code() {
        let (actions, _) = parse("[[action]]\nname = \"x\"\ncommand = \"echo %n %%n 100%\"\n");
        let item = file("/tmp/a; rm -rf ~ $(boom)");
        let line = &actions[0].command_lines(&[item], Path::new("/tmp"))[0];
        assert_eq!(strings(line), ["echo", "a; rm -rf ~ $(boom)", "%n", "100%"]);
    }

    #[cfg(unix)]
    #[test]
    fn invalid_utf8_names_pass_through() {
        use std::os::unix::ffi::OsStrExt;
        let raw = std::ffi::OsStr::from_bytes(b"/tmp/caf\xe9.txt");
        let item = Item {
            path: Path::new(raw),
            is_dir: false,
        };
        let (actions, _) = parse("[[action]]\nname = \"x\"\ncommand = \"open %f\"\n");
        let line = &actions[0].command_lines(&[item], Path::new("/tmp"))[0];
        assert_eq!(line[1].as_bytes(), b"/tmp/caf\xe9.txt");
    }

    #[test]
    fn splitting_like_a_shell() {
        assert_eq!(
            split_words(r#"a "b c" 'd "e"' f\ g "h\"i""#).expect("splits"),
            ["a", "b c", "d \"e\"", "f g", "h\"i"]
        );
        assert_eq!(split_words("  ").expect("splits"), Vec::<String>::new());
        assert!(split_words("say 'hi").is_err());
        assert!(split_words("say \"hi").is_err());
    }

    #[test]
    fn problems_skip_sections_not_files() {
        let text = r#"
[[action]]
name = "No command"
for = ["files"]
[[action]]
name = "Bad target"
command = "x"
for = ["everything"]
[[action]]
name = "Not a list"
command = "x"
for = "files"
[[action]]
name = "Unknown key"
command = "x"
color = "red"
[[action]]
command = "no name"
[[action]]
name = "Good"
command = "x"
"#;
        let (actions, problems) = parse(text);
        assert_eq!(actions.len(), 1);
        assert_eq!(actions[0].name, "Good");
        assert_eq!(problems.len(), 5, "{problems:?}");
    }

    #[test]
    fn the_shipped_example_parses() {
        let (actions, problems) = parse(include_str!("../../../data/config.example.toml"));
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(actions.len(), 1);
        assert!(actions[0].applies_to(&[]));
    }

    fn parse(text: &str) -> (Vec<Action>, Vec<String>) {
        return from_config(&Config::parse(text, ""));
    }
}
