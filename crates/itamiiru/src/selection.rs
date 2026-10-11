//! Which rows are selected, Windows-style: click, Ctrl+click, Shift+click,
//! arrow keys and type-ahead.
//!
//! Rows are remembered by path, not by index, because indices change when
//! the list is re-sorted or more entries stream in. Every method that needs
//! row order takes `paths`: the visible rows, top to bottom.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Default)]
pub struct Selection {
    selected: HashSet<PathBuf>,
    /// Where a Shift+click or Shift+arrow range starts.
    anchor: Option<PathBuf>,
    /// The row the keyboard is on.
    cursor: Option<PathBuf>,
}

impl Selection {
    pub fn clear(&mut self) {
        self.selected.clear();
        self.anchor = None;
        self.cursor = None;
    }

    pub fn is_selected(&self, path: &Path) -> bool {
        return self.selected.contains(path);
    }

    pub fn count(&self) -> usize {
        return self.selected.len();
    }

    pub fn cursor(&self) -> Option<&Path> {
        return self.cursor.as_deref();
    }

    /// Plain click: select only this row.
    pub fn click(&mut self, paths: &[&Path], index: usize) {
        let Some(path) = paths.get(index) else {
            return;
        };
        self.selected.clear();
        self.selected.insert(path.to_path_buf());
        self.anchor = Some(path.to_path_buf());
        self.cursor = Some(path.to_path_buf());
    }

    /// Ctrl+click: add or remove this row, keep the rest.
    pub fn ctrl_click(&mut self, paths: &[&Path], index: usize) {
        let Some(path) = paths.get(index) else {
            return;
        };
        if !self.selected.remove(*path) {
            self.selected.insert(path.to_path_buf());
        }
        self.anchor = Some(path.to_path_buf());
        self.cursor = Some(path.to_path_buf());
    }

    /// Shift+click: select every row from the anchor to this one.
    pub fn shift_click(&mut self, paths: &[&Path], index: usize) {
        if index >= paths.len() {
            return;
        }
        let anchor = match &self.anchor {
            Some(anchor) => index_of(paths, anchor).unwrap_or(0),
            None => 0,
        };
        self.select_range(paths, anchor, index);
        self.cursor = Some(paths[index].to_path_buf());
        if self.anchor.is_none() {
            self.anchor = Some(paths[anchor].to_path_buf());
        }
    }

    pub fn select_all(&mut self, paths: &[&Path]) {
        self.selected.clear();
        for path in paths {
            self.selected.insert(path.to_path_buf());
        }
    }

    /// Moves the keyboard cursor by `delta` rows (negative is up), clamped to
    /// the list. With `extend` (Shift held) the range from the anchor is
    /// selected; otherwise only the new row. Returns the new row, to scroll to.
    pub fn move_cursor(&mut self, paths: &[&Path], delta: isize, extend: bool) -> Option<usize> {
        if paths.is_empty() {
            return None;
        }
        let last = paths.len() - 1;
        let target = match self.cursor.as_deref().and_then(|c| index_of(paths, c)) {
            Some(current) => {
                let moved = current as isize + delta;
                moved.clamp(0, last as isize) as usize
            }
            // No cursor yet: Down starts at the top, Up at the bottom.
            None => {
                if delta >= 0 {
                    0
                } else {
                    last
                }
            }
        };
        if extend {
            self.shift_click(paths, target);
        } else {
            self.click(paths, target);
        }
        return Some(target);
    }

    /// Selects exactly the rows of `wanted` that are in `paths`, e.g. the
    /// files just pasted. Returns the first one's row, to scroll to.
    pub fn select_paths(&mut self, paths: &[&Path], wanted: &[PathBuf]) -> Option<usize> {
        let wanted_set: HashSet<&Path> = wanted.iter().map(|p| p.as_path()).collect();
        self.selected.clear();
        self.anchor = None;
        self.cursor = None;
        let mut first = None;
        for (row, path) in paths.iter().enumerate() {
            if wanted_set.contains(path) {
                self.selected.insert(path.to_path_buf());
                if first.is_none() {
                    first = Some(row);
                    self.anchor = Some(path.to_path_buf());
                    self.cursor = Some(path.to_path_buf());
                }
            }
        }
        return first;
    }

    /// Forgets rows that are no longer in `paths`, e.g. after hiding hidden files.
    pub fn retain_visible(&mut self, paths: &[&Path]) {
        // Nothing to forget. Skips building a set of every path while a
        // big folder streams in.
        if self.selected.is_empty() && self.anchor.is_none() && self.cursor.is_none() {
            return;
        }
        let visible: HashSet<&Path> = paths.iter().copied().collect();
        self.selected.retain(|p| visible.contains(p.as_path()));
        if let Some(anchor) = &self.anchor
            && !visible.contains(anchor.as_path())
        {
            self.anchor = None;
        }
        if let Some(cursor) = &self.cursor
            && !visible.contains(cursor.as_path())
        {
            self.cursor = None;
        }
    }

    fn select_range(&mut self, paths: &[&Path], a: usize, b: usize) {
        let (start, end) = if a <= b { (a, b) } else { (b, a) };
        self.selected.clear();
        for path in &paths[start..=end] {
            self.selected.insert(path.to_path_buf());
        }
    }
}

fn index_of(paths: &[&Path], path: &Path) -> Option<usize> {
    for (i, p) in paths.iter().enumerate() {
        if *p == path {
            return Some(i);
        }
    }
    return None;
}

/// Typing letters in the list jumps to the first name starting with them.
/// Pausing for a second starts a new search. Pressing the same letter again
/// cycles through the names starting with it, like Windows Explorer.
#[derive(Default)]
pub struct TypeAhead {
    typed: String,
    last_key_time: f64,
}

/// How long a pause starts a new search, in seconds.
const TYPE_AHEAD_RESET: f64 = 1.0;

impl TypeAhead {
    /// Adds `text` to the search and returns the row to jump to, if any.
    /// `names` are the visible rows' names (any case), `cursor` the current row.
    pub fn type_text(
        &mut self,
        text: &str,
        now: f64,
        names: &[String],
        cursor: Option<usize>,
    ) -> Option<usize> {
        if now - self.last_key_time > TYPE_AHEAD_RESET {
            self.typed.clear();
        }
        self.last_key_time = now;
        self.typed.push_str(&text.to_lowercase());
        if names.is_empty() {
            return None;
        }

        // "aaa" means "the next name starting with a", not "a name starting with aaa".
        let first = self.typed.chars().next()?;
        let mut all_same = true;
        for c in self.typed.chars() {
            if c != first {
                all_same = false;
            }
        }
        let (needle, start) = if all_same {
            // Search after the current row, so each press moves on.
            let start = match cursor {
                Some(c) => c + 1,
                None => 0,
            };
            (first.to_string(), start)
        } else {
            // Keep the current row if it still matches the longer text.
            (self.typed.clone(), cursor.unwrap_or(0))
        };

        // Look from `start` to the end, then wrap around to the top.
        for step in 0..names.len() {
            let i = (start + step) % names.len();
            if names[i].to_lowercase().starts_with(&needle) {
                return Some(i);
            }
        }
        return None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn paths() -> Vec<PathBuf> {
        let mut v = Vec::new();
        for name in ["a", "b", "c", "d", "e"] {
            v.push(PathBuf::from(name));
        }
        return v;
    }

    fn refs(v: &[PathBuf]) -> Vec<&Path> {
        return v.iter().map(|p| p.as_path()).collect();
    }

    fn selected_names(sel: &Selection, v: &[PathBuf]) -> String {
        let mut out = String::new();
        for p in v {
            if sel.is_selected(p) {
                out.push_str(&p.to_string_lossy());
            }
        }
        return out;
    }

    #[test]
    fn click_ctrl_click_shift_click() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        sel.click(&p, 1);
        assert_eq!(selected_names(&sel, &v), "b");
        sel.ctrl_click(&p, 3);
        assert_eq!(selected_names(&sel, &v), "bd");
        sel.ctrl_click(&p, 1);
        assert_eq!(selected_names(&sel, &v), "d");
        // The anchor moved to b with that Ctrl+click, even though it
        // deselected b. Windows Explorer does the same.
        sel.shift_click(&p, 0);
        assert_eq!(selected_names(&sel, &v), "ab");
        sel.shift_click(&p, 4);
        assert_eq!(selected_names(&sel, &v), "bcde");
        assert_eq!(sel.cursor(), Some(Path::new("e")));
    }

    #[test]
    fn shift_click_without_anchor_starts_at_top() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        sel.shift_click(&p, 2);
        assert_eq!(selected_names(&sel, &v), "abc");
    }

    #[test]
    fn arrow_keys() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        assert_eq!(sel.move_cursor(&p, 1, false), Some(0));
        assert_eq!(sel.move_cursor(&p, 1, false), Some(1));
        assert_eq!(selected_names(&sel, &v), "b");
        assert_eq!(sel.move_cursor(&p, 1, true), Some(2));
        assert_eq!(sel.move_cursor(&p, 1, true), Some(3));
        assert_eq!(selected_names(&sel, &v), "bcd");
        assert_eq!(sel.move_cursor(&p, -100, false), Some(0));
        assert_eq!(sel.move_cursor(&p, 100, false), Some(4));
        assert_eq!(selected_names(&sel, &v), "e");
        assert_eq!(sel.move_cursor(&[], 1, false), None);
    }

    #[test]
    fn up_with_no_cursor_starts_at_bottom() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        assert_eq!(sel.move_cursor(&p, -1, false), Some(4));
    }

    #[test]
    fn select_all_and_retain() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        sel.click(&p, 4);
        sel.select_all(&p);
        assert_eq!(sel.count(), 5);
        let fewer = refs(&v[..3]);
        sel.retain_visible(&fewer);
        assert_eq!(selected_names(&sel, &v), "abc");
        assert_eq!(sel.cursor(), None, "the cursor was on e, which is gone");
    }

    #[test]
    fn out_of_range_index_is_ignored() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        sel.click(&p, 99);
        sel.ctrl_click(&p, 99);
        sel.shift_click(&p, 99);
        assert_eq!(sel.count(), 0);
    }

    #[test]
    fn select_paths_picks_visible_ones() {
        let v = paths();
        let p = refs(&v);
        let mut sel = Selection::default();
        sel.click(&p, 0);
        let wanted = vec![PathBuf::from("d"), PathBuf::from("b"), PathBuf::from("zzz")];
        assert_eq!(sel.select_paths(&p, &wanted), Some(1));
        assert_eq!(selected_names(&sel, &v), "bd");
        assert_eq!(sel.cursor(), Some(Path::new("b")));
        assert_eq!(sel.select_paths(&p, &[]), None);
        assert_eq!(sel.count(), 0);
    }

    fn names(list: &[&str]) -> Vec<String> {
        return list.iter().map(|s| s.to_string()).collect();
    }

    #[test]
    fn type_ahead_prefix() {
        let n = names(&["Apple", "apricot", "Banana", "blueberry"]);
        let mut t = TypeAhead::default();
        assert_eq!(t.type_text("b", 10.0, &n, None), Some(2));
        assert_eq!(t.type_text("l", 10.2, &n, Some(2)), Some(3));
        // A pause starts over.
        assert_eq!(t.type_text("a", 12.0, &n, Some(3)), Some(0));
        assert_eq!(t.type_text("p", 12.1, &n, Some(0)), Some(0));
        assert_eq!(t.type_text("r", 12.2, &n, Some(0)), Some(1));
        assert_eq!(t.type_text("x", 12.3, &n, Some(1)), None);
    }

    #[test]
    fn type_ahead_same_letter_cycles() {
        let n = names(&["a1", "b1", "a2", "a3"]);
        let mut t = TypeAhead::default();
        assert_eq!(t.type_text("a", 1.0, &n, None), Some(0));
        assert_eq!(t.type_text("a", 1.1, &n, Some(0)), Some(2));
        assert_eq!(t.type_text("a", 1.2, &n, Some(2)), Some(3));
        assert_eq!(
            t.type_text("a", 1.3, &n, Some(3)),
            Some(0),
            "wraps to the top"
        );
    }
}
