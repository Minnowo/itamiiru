// FFI calls into libc, each with a SAFETY comment.
#![allow(unsafe_code)]

use std::path::PathBuf;

pub mod dbus;
mod places;
mod udisks;
mod xdnd;
pub use places::{
    add_bookmark, bookmarks, disk_usage, drives, is_bookmarked, network_places, quick_access,
    remove_bookmark,
};
pub use udisks::{eject_volume, mount_volume, removable_volumes, unmount_volume};

/// Drags `paths` out of the window into another app (XDND on X11). Call
/// while the button is down and the pointer has left the window. The
/// handle shows how it's going and can cancel it.
pub fn start_drag_out(paths: Vec<PathBuf>) -> Result<super::DragOut, String> {
    return xdnd::start(paths);
}

/// Where the pointer is on the screen, in pixels, when the windowing
/// system will say (X11). Used while files from another app are dragged
/// over the window, when winit doesn't report the pointer.
pub fn pointer_position() -> Option<(f32, f32)> {
    return xdnd::pointer_position();
}

/// `$XDG_CONFIG_HOME`, or `~/.config`.
pub fn config_home() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CONFIG_HOME") {
        let dir = PathBuf::from(dir);
        // The XDG spec says relative paths are invalid and must be ignored.
        if dir.is_absolute() {
            return Some(dir);
        }
    }
    return Some(home_dir()?.join(".config"));
}

/// The itamiiru config folder: `$XDG_CONFIG_HOME/itamiiru`, or `~/.config/itamiiru`.
pub fn config_dir() -> Option<PathBuf> {
    return Some(config_home()?.join("itamiiru"));
}

/// Dotfiles are hidden.
pub fn is_hidden(name: &std::ffi::OsStr, _meta: &std::fs::Metadata) -> bool {
    return name.as_encoded_bytes().first() == Some(&b'.');
}

/// The home folder, from `$HOME`.
pub fn home_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    if home.is_empty() {
        return None;
    }
    return Some(PathBuf::from(home));
}

/// The local time zone's offset from UTC, in seconds, at the given Unix time.
/// The first call reads the time zone file, so call it once at startup.
pub fn utc_offset_seconds(unix_secs: i64) -> i64 {
    // time_t is 64-bit on every target we build for, so this cast keeps the value.
    let time = unix_secs as libc::time_t;
    // SAFETY: libc::tm is a plain C struct of integers and a pointer;
    // all zeroes is a valid value for it.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: localtime_r reads `time` and writes only into `tm`. Both are
    // valid local variables for the whole call. It's the thread-safe variant.
    let result = unsafe { libc::localtime_r(&time, &mut tm) };
    if result.is_null() {
        return 0;
    }
    return tm.tm_gmtoff as i64;
}

/// A regular file with any execute bit set.
pub fn is_executable(_name: &std::ffi::OsStr, meta: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::PermissionsExt;
    return meta.is_file() && meta.permissions().mode() & 0o111 != 0;
}

/// The system's own text fonts, as fontconfig chooses them (the same
/// choice GTK apps get): (sans-serif, monospace), each as (file, face
/// index). Runs `fc-match`: call it on a worker thread.
pub fn main_font_files() -> super::MainFonts {
    return super::MainFonts {
        sans: fc_match("sans-serif"),
        mono: fc_match("monospace"),
    };
}

/// The languages fallback fonts are found for: Japanese, Simplified and
/// Traditional Chinese, Korean, Thai, Hebrew, Arabic and Hindi.
const FALLBACK_LANGS: [&str; 8] = ["ja", "zh-cn", "zh-tw", "ko", "th", "he", "ar", "hi"];

/// How many covering fonts to keep per language, best first, in case
/// egui can't read the first.
const CANDIDATES_PER_LANG: usize = 5;

/// Fonts for scripts the main fonts may lack: for each language with one
/// installed, (language, covering fonts best first), each as (file, face
/// index in a collection). Asks fontconfig, so it runs `fc-match` once per
/// language: call it on a worker thread. Files may still not exist.
pub fn fallback_font_files() -> Vec<(String, Vec<super::FontFile>)> {
    let mut out = Vec::new();
    for lang in FALLBACK_LANGS {
        let fonts = fonts_for_lang(lang);
        if !fonts.is_empty() {
            out.push((lang.to_string(), fonts));
        }
    }
    return out;
}

/// The fonts in fontconfig's ranked fallback list for `lang` that really
/// cover it, best first. Plain `fc-match :lang=ja` isn't enough: it can
/// answer with the preferred family even when that has no Japanese (Arch
/// with Noto Sans first, though Noto Sans CJK is installed).
fn fonts_for_lang(lang: &str) -> Vec<super::FontFile> {
    let output = std::process::Command::new("fc-match")
        .args([
            "-s",
            "-f",
            "%{file}\t%{index}\t%{lang}\n",
            &format!("sans-serif:lang={lang}"),
        ])
        .output();
    return match output {
        Ok(output) if output.status.success() => {
            covering_fonts(&output.stdout, lang, CANDIDATES_PER_LANG)
        }
        _ => Vec::new(),
    };
}

/// From `fc-match -s` output (file, index and the languages it covers,
/// tab-separated, one font per line), the first `limit` fonts whose
/// languages include `lang`. Each file appears once.
pub fn covering_fonts(listing: &[u8], lang: &str, limit: usize) -> Vec<super::FontFile> {
    use std::os::unix::ffi::OsStrExt;
    let mut out: Vec<super::FontFile> = Vec::new();
    for line in listing.split(|&b| b == b'\n') {
        if out.len() >= limit {
            break;
        }
        let mut fields = line.split(|&b| b == b'\t');
        let (Some(file), Some(index), Some(langs)) = (fields.next(), fields.next(), fields.next())
        else {
            continue;
        };
        let langs = String::from_utf8_lossy(langs);
        if file.is_empty() || !langs.split('|').any(|l| l == lang) {
            continue;
        }
        let path = PathBuf::from(std::ffi::OsStr::from_bytes(file));
        if out.iter().any(|(seen, _)| *seen == path) {
            continue;
        }
        let index: u32 = String::from_utf8_lossy(index).trim().parse().unwrap_or(0);
        out.push((path, index));
    }
    return out;
}

/// What fontconfig says about the fonts itamiiru asks for, for
/// `itami --font-info`: its version, the match for each pattern with its
/// format, and which installed fonts cover Japanese.
pub fn font_report() -> Vec<String> {
    let run = |args: &[&str]| -> String {
        return match std::process::Command::new(args[0])
            .args(&args[1..])
            .output()
        {
            // Some versions print --version to stderr.
            Ok(output) => {
                let mut text = String::from_utf8_lossy(&output.stdout).into_owned();
                text.push_str(&String::from_utf8_lossy(&output.stderr));
                text.trim().to_string()
            }
            Err(err) => format!("(can't run {}: {err})", args[0]),
        };
    };
    let mut lines = vec![format!("fontconfig: {}", run(&["fc-match", "--version"]))];
    let format = "%{file} | index %{index} | %{family[0]} | %{fontformat} | variable=%{variable}";
    for pattern in ["sans-serif", "monospace"] {
        lines.push(format!(
            "{pattern} -> {}",
            run(&["fc-match", "-f", format, pattern])
        ));
    }
    for lang in FALLBACK_LANGS {
        let chosen = match fonts_for_lang(lang).first() {
            Some((path, index)) => format!("{} (face {index})", path.display()),
            None => "none installed".to_string(),
        };
        let plain = run(&["fc-match", "-f", "%{file}", &format!(":lang={lang}")]);
        lines.push(format!(
            "{lang}: covering font {chosen}; plain fc-match says {plain}"
        ));
    }
    let japanese = run(&["fc-list", ":lang=ja", "file"]);
    let files: Vec<&str> = japanese.lines().collect();
    lines.push(format!(
        "fonts covering Japanese (fc-list :lang=ja): {}",
        files.len()
    ));
    for file in files.iter().take(8) {
        lines.push(format!("  {file}"));
    }
    return lines;
}

/// The font file fontconfig picks for `pattern`, e.g. "sans-serif" or
/// ":lang=ja", as (file, face index).
fn fc_match(pattern: &str) -> Option<(PathBuf, u32)> {
    use std::os::unix::ffi::OsStrExt;
    let output = std::process::Command::new("fc-match")
        .args(["-f", "%{file}|%{index}", pattern])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    // "path|index"; the path is raw bytes, the index after the last '|'.
    let text = output.stdout;
    let bar = text.iter().rposition(|&b| b == b'|')?;
    let path = PathBuf::from(std::ffi::OsStr::from_bytes(&text[..bar]));
    let index_text = String::from_utf8_lossy(&text[bar + 1..]);
    let index: u32 = index_text.trim().parse().unwrap_or(0);
    if path.as_os_str().is_empty() {
        return None;
    }
    return Some((path, index));
}

/// Raw bytes (e.g. a decoded file URI) to a path. Unix paths are bytes.
pub fn path_from_bytes(bytes: Vec<u8>) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt;
    return Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)));
}

/// The real user id of this process.
pub fn current_uid() -> u32 {
    // SAFETY: getuid takes no arguments, has no preconditions and can't fail.
    return unsafe { libc::getuid() };
}

/// Opens a file in its default app with `xdg-open`. Returns once it started;
/// a thread waits for it to exit so it doesn't linger as a zombie process.
pub fn open_with_default_app(path: &std::path::Path) -> std::io::Result<()> {
    let mut command = std::process::Command::new("xdg-open");
    command.arg(path);
    return spawn_and_reap(command);
}

fn spawn_and_reap(mut command: std::process::Command) -> std::io::Result<()> {
    use std::process::Stdio;
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    return Ok(());
}

/// Recreates the symlink `src` at `dst`, pointing at the same target.
pub fn copy_symlink(src: &std::path::Path, dst: &std::path::Path) -> std::io::Result<()> {
    let target = std::fs::read_link(src)?;
    return std::os::unix::fs::symlink(target, dst);
}

/// Moves `path` to the trash, following the freedesktop trash spec
/// (https://specifications.freedesktop.org/trash-spec/latest/).
pub fn trash(path: &std::path::Path) -> std::io::Result<()> {
    let Some(home_trash) = home_trash_dir() else {
        return Err(std::io::Error::other(
            "can't find the home folder for the trash",
        ));
    };
    return trash_into(path, &home_trash);
}

/// `$XDG_DATA_HOME/Trash`, or `~/.local/share/Trash`.
fn home_trash_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_DATA_HOME") {
        let dir = PathBuf::from(dir);
        if dir.is_absolute() {
            return Some(dir.join("Trash"));
        }
    }
    return Some(home_dir()?.join(".local").join("share").join("Trash"));
}

/// Trashes `path` into `home_trash` when both are on the same filesystem,
/// otherwise into a trash folder at the top of the path's filesystem.
/// Split out from `trash` so tests can use a temporary home trash.
pub fn trash_into(path: &std::path::Path, home_trash: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::MetadataExt;
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::other("can't trash this path"));
    };
    // Resolve the parent (not the item itself: a symlink is trashed as a link).
    let parent = match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => std::fs::canonicalize(parent)?,
        _ => std::env::current_dir()?,
    };
    let real_path = parent.join(name);
    let device = std::fs::symlink_metadata(&real_path)?.dev();

    if device == nearest_existing_device(home_trash)? {
        // The home trash stores absolute paths.
        let info_path = real_path.as_os_str().as_encoded_bytes().to_vec();
        return move_to_trash(&real_path, home_trash, &info_path);
    }

    // Another drive: use a trash at the top of that drive, storing paths
    // relative to it. First $topdir/.Trash/$uid, if an admin made .Trash
    // (it must be a real directory with the sticky bit), else $topdir/.Trash-$uid.
    let topdir = mount_top(&parent, device);
    let relative = real_path.strip_prefix(&topdir).unwrap_or(&real_path);
    let info_path = relative.as_os_str().as_encoded_bytes().to_vec();
    let uid = current_uid();
    let shared = topdir.join(".Trash");
    if let Ok(meta) = std::fs::symlink_metadata(&shared) {
        use std::os::unix::fs::PermissionsExt;
        let sticky = meta.permissions().mode() & 0o1000 != 0;
        if meta.is_dir() && sticky {
            let dir = shared.join(uid.to_string());
            if create_private_dir(&dir).is_ok() {
                return move_to_trash(&real_path, &dir, &info_path);
            }
        }
    }
    let dir = topdir.join(format!(".Trash-{uid}"));
    create_private_dir(&dir)?;
    return move_to_trash(&real_path, &dir, &info_path);
}

/// The filesystem device of `path`, or of its nearest existing parent.
fn nearest_existing_device(path: &std::path::Path) -> std::io::Result<u64> {
    use std::os::unix::fs::MetadataExt;
    let mut current = Some(path);
    while let Some(p) = current {
        if let Ok(meta) = std::fs::metadata(p) {
            return Ok(meta.dev());
        }
        current = p.parent();
    }
    return Err(std::io::Error::other("no existing parent folder"));
}

/// The top folder of the filesystem `dir` is on: go up while the parent
/// is still on the same device.
fn mount_top(dir: &std::path::Path, device: u64) -> PathBuf {
    use std::os::unix::fs::MetadataExt;
    let mut top = dir.to_path_buf();
    while let Some(parent) = top.parent() {
        match std::fs::metadata(parent) {
            Ok(meta) if meta.dev() == device => top = parent.to_path_buf(),
            _ => break,
        }
    }
    return top;
}

/// Creates a folder (and missing parents) readable only by this user.
fn create_private_dir(dir: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    return std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir);
}

/// Moves `path` into `trash_dir/files` and writes its `.trashinfo`.
/// `info_path` is the original path as stored in the info file.
fn move_to_trash(
    path: &std::path::Path,
    trash_dir: &std::path::Path,
    info_path: &[u8],
) -> std::io::Result<()> {
    use std::io::Write;
    let files = trash_dir.join("files");
    let info = trash_dir.join("info");
    create_private_dir(&files)?;
    create_private_dir(&info)?;
    let Some(name) = path.file_name() else {
        return Err(std::io::Error::other("can't trash this path"));
    };
    let keep_extension = !std::fs::symlink_metadata(path)?.is_dir();
    let deleted_at = crate::format::format_local_iso(std::time::SystemTime::now());

    for n in 1..=10_000 {
        let candidate = if n == 1 {
            name.to_os_string()
        } else {
            crate::names::numbered(name, n, keep_extension)
        };
        let mut info_name = candidate.clone();
        info_name.push(".trashinfo");
        let info_file = info.join(&info_name);
        // Creating the info file with create_new reserves the name; the spec
        // relies on this being atomic.
        let mut file = match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&info_file)
        {
            Ok(file) => file,
            Err(err) if err.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err),
        };
        let target = files.join(&candidate);
        if std::fs::symlink_metadata(&target).is_ok() {
            // A leftover without an info file; pick another name.
            drop(file);
            let _ = std::fs::remove_file(&info_file);
            continue;
        }
        let contents = format!(
            "[Trash Info]\nPath={}\nDeletionDate={}\n",
            crate::percent::encode(info_path),
            deleted_at
        );
        if let Err(err) = file.write_all(contents.as_bytes()) {
            drop(file);
            let _ = std::fs::remove_file(&info_file);
            return Err(err);
        }
        drop(file);
        if let Err(err) = std::fs::rename(path, &target) {
            let _ = std::fs::remove_file(&info_file);
            return Err(err);
        }
        return Ok(());
    }
    return Err(std::io::Error::other(
        "too many items with this name in the trash",
    ));
}

/// Virtual filesystems a search shouldn't walk into.
pub fn is_virtual_dir(path: &std::path::Path) -> bool {
    for dir in ["/proc", "/sys", "/dev", "/run"] {
        if path == std::path::Path::new(dir) {
            return true;
        }
    }
    return false;
}

/// shared-mime-info globs2 files: `$XDG_DATA_HOME` first, then `$XDG_DATA_DIRS`.
pub fn mime_glob_files() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    match std::env::var_os("XDG_DATA_HOME") {
        Some(dir) if PathBuf::from(&dir).is_absolute() => dirs.push(PathBuf::from(dir)),
        _ => {
            if let Some(home) = home_dir() {
                dirs.push(home.join(".local").join("share"));
            }
        }
    }
    let data_dirs = std::env::var("XDG_DATA_DIRS").unwrap_or_default();
    let data_dirs = if data_dirs.is_empty() {
        "/usr/local/share:/usr/share".to_string()
    } else {
        data_dirs
    };
    for dir in data_dirs.split(':') {
        if dir.starts_with('/') {
            dirs.push(PathBuf::from(dir));
        }
    }
    let mut files = Vec::new();
    for dir in dirs {
        files.push(dir.join("mime").join("globs2"));
    }
    return files;
}

/// Whether this user may create files in `dir` (access with W_OK).
pub fn is_writable_dir(dir: &std::path::Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c_path) = std::ffi::CString::new(dir.as_os_str().as_bytes()) else {
        return false;
    };
    // SAFETY: c_path is a valid NUL-terminated string that outlives the call;
    // access only reads it.
    let result = unsafe { libc::access(c_path.as_ptr(), libc::W_OK) };
    return result == 0;
}

/// The shared thumbnail cache: `$XDG_CACHE_HOME/thumbnails`, or
/// `~/.cache/thumbnails` (freedesktop thumbnail spec).
pub fn thumbnail_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os("XDG_CACHE_HOME") {
        let dir = PathBuf::from(dir);
        // Relative paths are invalid, as for XDG_CONFIG_HOME.
        if dir.is_absolute() {
            return Some(dir.join("thumbnails"));
        }
    }
    return Some(home_dir()?.join(".cache").join("thumbnails"));
}

/// Creates a new file only this user can read (0600). Fails if it exists.
pub fn create_private_file(path: &std::path::Path) -> std::io::Result<std::fs::File> {
    use std::os::unix::fs::OpenOptionsExt;
    return std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path);
}

/// Creates a folder and any missing parents, only this user can open (0700).
pub fn create_private_dir_all(path: &std::path::Path) -> std::io::Result<()> {
    use std::os::unix::fs::DirBuilderExt;
    return std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path);
}

/// Starts `program` with `args` in the background, cut off from the
/// terminal: no stdin, stdout or stderr, and its own process group, so
/// the terminal's hang-up signal (closing the window) doesn't reach it.
/// Doesn't wait for it; when this process exits, init adopts and reaps it.
pub fn spawn_detached(
    program: &std::path::Path,
    args: &[std::ffi::OsString],
) -> std::io::Result<()> {
    use std::os::unix::process::CommandExt;
    use std::process::Stdio;
    std::process::Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .process_group(0)
        .spawn()?;
    return Ok(());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_dir(label: &str) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("itamiiru-trash-{}-{}", std::process::id(), label));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        return dir;
    }

    #[test]
    fn detached_children_leave_the_terminal() {
        // A sleep with an odd length, so it can be found among others.
        let args = vec![std::ffi::OsString::from("7.31")];
        spawn_detached(std::path::Path::new("/bin/sleep"), &args).expect("spawns");
        let mut found = None;
        for _ in 0..100 {
            for entry in std::fs::read_dir("/proc").expect("proc").flatten() {
                let cmdline = std::fs::read(entry.path().join("cmdline")).unwrap_or_default();
                if cmdline == b"/bin/sleep\x007.31\x00" {
                    found = Some(entry.path());
                }
            }
            if found.is_some() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        let proc_dir = found.expect("the child is running");
        let pid: u32 = proc_dir
            .file_name()
            .and_then(|n| n.to_str())
            .and_then(|n| n.parse().ok())
            .expect("pid");
        // Field 5 of /proc/PID/stat is the process group; it leads its own.
        let stat = std::fs::read_to_string(proc_dir.join("stat")).expect("stat");
        let after_name = &stat[stat.rfind(')').expect("comm") + 2..];
        let pgid: u32 = after_name
            .split(' ')
            .nth(2)
            .and_then(|f| f.parse().ok())
            .expect("pgid");
        assert_eq!(pgid, pid, "own process group");
        for fd in ["0", "1", "2"] {
            let target = std::fs::read_link(proc_dir.join("fd").join(fd)).expect("fd");
            assert_eq!(target, PathBuf::from("/dev/null"), "fd {fd}");
        }
        let _ = std::process::Command::new("kill")
            .arg(pid.to_string())
            .status();
    }

    #[test]
    fn covering_fonts_come_from_the_ranked_list() {
        // Shaped like Arch's: the preferred family comes first but has no
        // Japanese; the CJK collection and IPA further down do.
        let listing = b"/usr/share/fonts/noto/NotoSans-Regular.ttf\t0\taa|af|en|fr\n\
/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc\t0\tja|ko|zh-cn|zh-tw\n\
/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc\t2\tja|ko|zh-cn|zh-tw\n\
/usr/share/fonts/OTF/ipagp.ttf\t0\tja\n";
        let cjk = PathBuf::from("/usr/share/fonts/noto-cjk/NotoSansCJK-Regular.ttc");
        let ipa = PathBuf::from("/usr/share/fonts/OTF/ipagp.ttf");
        // Best first, each file once.
        assert_eq!(
            covering_fonts(listing, "ja", 5),
            vec![(cjk.clone(), 0), (ipa, 0)]
        );
        assert_eq!(covering_fonts(listing, "ja", 1), vec![(cjk, 0)]);
        // "zh" alone isn't "zh-cn"; nothing covers Thai.
        assert!(covering_fonts(b"/a.ttf\t2\tzh\n", "zh-cn", 5).is_empty());
        assert!(covering_fonts(listing, "th", 5).is_empty());
        assert_eq!(covering_fonts(b"/b.ttc\t3\tko\n", "ko", 5)[0].1, 3);
        assert!(covering_fonts(b"garbage", "ja", 5).is_empty());
    }

    #[test]
    fn trash_file_writes_info() {
        let dir = test_dir("file");
        let trash = dir.join("Trash");
        let file = dir.join("a b.txt");
        std::fs::write(&file, b"hello").unwrap();

        trash_into(&file, &trash).unwrap();
        assert!(!file.exists());
        assert_eq!(
            std::fs::read(trash.join("files/a b.txt")).unwrap(),
            b"hello"
        );
        let info = std::fs::read_to_string(trash.join("info/a b.txt.trashinfo")).unwrap();
        let real = std::fs::canonicalize(&dir).unwrap().join("a b.txt");
        let expected_path = crate::percent::encode(real.as_os_str().as_encoded_bytes());
        assert!(info.starts_with("[Trash Info]\nPath="), "{info}");
        assert!(info.contains(&format!("Path={expected_path}\n")), "{info}");
        assert!(info.contains("%20"), "spaces are encoded: {info}");
        assert!(info.contains("DeletionDate="), "{info}");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_same_name_twice_and_folders() {
        let dir = test_dir("twice");
        let trash = dir.join("Trash");
        for _ in 0..2 {
            std::fs::write(dir.join("x.txt"), b"").unwrap();
            trash_into(&dir.join("x.txt"), &trash).unwrap();
        }
        assert!(trash.join("files/x.txt").exists());
        assert!(trash.join("files/x (2).txt").exists());
        assert!(trash.join("info/x (2).txt.trashinfo").exists());

        std::fs::create_dir_all(dir.join("folder/inner")).unwrap();
        trash_into(&dir.join("folder"), &trash).unwrap();
        assert!(trash.join("files/folder/inner").is_dir());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_symlink_trashes_the_link() {
        let dir = test_dir("link");
        let trash = dir.join("Trash");
        std::fs::write(dir.join("target"), b"keep me").unwrap();
        std::os::unix::fs::symlink(dir.join("target"), dir.join("link")).unwrap();
        trash_into(&dir.join("link"), &trash).unwrap();
        assert!(dir.join("target").exists(), "the target stays");
        let meta = std::fs::symlink_metadata(trash.join("files/link")).unwrap();
        assert!(meta.file_type().is_symlink());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn trash_missing_file_fails() {
        let dir = test_dir("missing");
        assert!(trash_into(&dir.join("nope"), &dir.join("Trash")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn path_bytes() {
        let path = path_from_bytes(b"/tmp/a\xffb".to_vec()).unwrap();
        assert_eq!(path.as_os_str().as_encoded_bytes(), b"/tmp/a\xffb");
    }

    #[test]
    fn fallback_fonts_are_sane() {
        // Depends on the machine's fonts; only checks the parsing is sane.
        for (lang, fonts) in fallback_font_files() {
            assert!(!fonts.is_empty(), "{lang}");
            for (i, font) in fonts.iter().enumerate() {
                assert!(!font.0.as_os_str().is_empty());
                assert!(!fonts[i + 1..].contains(font), "duplicate {font:?}");
            }
        }
    }

    #[test]
    fn dotfiles_are_hidden() {
        let meta = std::fs::metadata("/").unwrap();
        assert!(is_hidden(std::ffi::OsStr::new(".x"), &meta));
        assert!(!is_hidden(std::ffi::OsStr::new("x.y"), &meta));
        assert!(!is_hidden(std::ffi::OsStr::new(""), &meta));
    }
}
