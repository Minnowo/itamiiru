//! Filename globs for file-type filters: `*`, `?` and `[...]`.
//!
//! `*` matches any run of characters (including none), `?` exactly one,
//! `[abc]`, `[a-z]` and `[!a-z]` (or `[^a-z]`) one character from (or not
//! from) a set. A `[` with no closing `]` is a plain character. Matching is
//! case-sensitive unless the glob was made with `case_insensitive`.

#[derive(Clone, Debug, PartialEq, Eq)]
enum Token {
    Char(char),
    /// `?`
    One,
    /// `*`
    Many,
    Class {
        negated: bool,
        ranges: Vec<(char, char)>,
    },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Glob {
    tokens: Vec<Token>,
    case_insensitive: bool,
}

impl Glob {
    pub fn new(pattern: &str) -> Glob {
        return Glob {
            tokens: parse(pattern),
            case_insensitive: false,
        };
    }

    /// For shared-mime-info globs, which are matched ignoring case.
    pub fn case_insensitive(pattern: &str) -> Glob {
        return Glob {
            tokens: parse(&pattern.to_lowercase()),
            case_insensitive: true,
        };
    }

    /// True if the pattern uses any wildcard (so typing it means "filter",
    /// not "this file name").
    pub fn has_wildcards(pattern: &str) -> bool {
        return pattern.contains(['*', '?', '[']);
    }

    pub fn matches(&self, name: &str) -> bool {
        let lowered;
        let name = if self.case_insensitive {
            lowered = name.to_lowercase();
            lowered.as_str()
        } else {
            name
        };
        let chars: Vec<char> = name.chars().collect();
        return match_tokens(&self.tokens, &chars);
    }
}

fn parse(pattern: &str) -> Vec<Token> {
    let chars: Vec<char> = pattern.chars().collect();
    let mut tokens = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        match chars[i] {
            '*' => {
                // "**" means the same as "*".
                if tokens.last() != Some(&Token::Many) {
                    tokens.push(Token::Many);
                }
                i += 1;
            }
            '?' => {
                tokens.push(Token::One);
                i += 1;
            }
            '[' => match parse_class(&chars, i) {
                Some((token, next)) => {
                    tokens.push(token);
                    i = next;
                }
                None => {
                    tokens.push(Token::Char('['));
                    i += 1;
                }
            },
            c => {
                tokens.push(Token::Char(c));
                i += 1;
            }
        }
    }
    return tokens;
}

/// Parses `[...]` starting at `start` (the `[`). Returns the token and the
/// index after the `]`, or None if there's no closing `]`.
fn parse_class(chars: &[char], start: usize) -> Option<(Token, usize)> {
    let mut i = start + 1;
    let mut negated = false;
    if i < chars.len() && (chars[i] == '!' || chars[i] == '^') {
        negated = true;
        i += 1;
    }
    let mut ranges = Vec::new();
    let mut first = true;
    while i < chars.len() {
        let c = chars[i];
        // A `]` right after `[` or `[!` is a member, not the end.
        if c == ']' && !first {
            return Some((Token::Class { negated, ranges }, i + 1));
        }
        first = false;
        if i + 2 < chars.len() && chars[i + 1] == '-' && chars[i + 2] != ']' {
            ranges.push((c, chars[i + 2]));
            i += 3;
        } else {
            ranges.push((c, c));
            i += 1;
        }
    }
    return None;
}

fn token_matches_char(token: &Token, c: char) -> bool {
    return match token {
        Token::Char(expected) => *expected == c,
        Token::One => true,
        Token::Many => true,
        Token::Class { negated, ranges } => {
            let mut inside = false;
            for (low, high) in ranges {
                if *low <= c && c <= *high {
                    inside = true;
                    break;
                }
            }
            inside != *negated
        }
    };
}

/// Matching with backtracking to the last `*` only: since every other
/// token matches exactly one character, that's enough and runs in
/// O(pattern * name) at worst.
fn match_tokens(tokens: &[Token], name: &[char]) -> bool {
    let mut t = 0;
    let mut n = 0;
    // Where to resume after the last `*`: (token after it, name position).
    let mut star: Option<(usize, usize)> = None;
    while n < name.len() {
        if t < tokens.len() && tokens[t] == Token::Many {
            star = Some((t + 1, n));
            t += 1;
            continue;
        }
        if t < tokens.len() && token_matches_char(&tokens[t], name[n]) {
            t += 1;
            n += 1;
            continue;
        }
        // Mismatch: let the last `*` eat one more character, if there is one.
        let Some((after_star, star_n)) = star else {
            return false;
        };
        t = after_star;
        n = star_n + 1;
        star = Some((after_star, n));
    }
    // The name is used up; only trailing `*`s may remain.
    while t < tokens.len() && tokens[t] == Token::Many {
        t += 1;
    }
    return t == tokens.len();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn m(pattern: &str, name: &str) -> bool {
        return Glob::new(pattern).matches(name);
    }

    #[test]
    fn stars_and_question_marks() {
        assert!(m("*.txt", "notes.txt"));
        assert!(m("*.txt", ".txt"));
        assert!(!m("*.txt", "notes.txt.bak"));
        assert!(m("*", ""));
        assert!(m("*", "anything"));
        assert!(m("a*b*c", "aXXbYYc"));
        assert!(m("a*b*c", "abc"));
        assert!(!m("a*b*c", "acb"));
        assert!(m("??.md", "ab.md"));
        assert!(!m("??.md", "abc.md"));
        assert!(m("**.rs", "main.rs"));
        assert!(!m("", "x"));
        assert!(m("", ""));
    }

    #[test]
    fn case_sensitive_by_default() {
        assert!(!m("*.png", "photo.PNG"));
        assert!(m("*.[pP][nN][gG]", "photo.PNG"));
        assert!(m("*.[pP][nN][gG]", "photo.png"));
        assert!(Glob::case_insensitive("*.PNG").matches("photo.png"));
        assert!(Glob::case_insensitive("*.png").matches("Photo.PnG"));
    }

    #[test]
    fn classes() {
        assert!(m("file[0-9].txt", "file7.txt"));
        assert!(!m("file[0-9].txt", "fileX.txt"));
        assert!(m("file[!0-9].txt", "fileX.txt"));
        assert!(m("file[^0-9].txt", "fileX.txt"));
        assert!(!m("file[!0-9].txt", "file7.txt"));
        assert!(m("[]]", "]"));
        assert!(m("[a-]", "-"), "a trailing - is a member");
        assert!(m("x[", "x["), "unterminated [ is literal");
        assert!(!m("x[", "xa"));
    }

    #[test]
    fn unicode_names() {
        assert!(m("*.txt", "\u{65e5}\u{672c}.txt"));
        assert!(
            m("?.txt", "\u{65e5}.txt"),
            "? is one character, not one byte"
        );
    }

    #[test]
    fn long_inputs_stay_fast() {
        let name = "a".repeat(10_000);
        assert!(!m("*a*a*a*a*a*b", &name));
    }

    #[test]
    fn wildcard_detection() {
        assert!(Glob::has_wildcards("*.log"));
        assert!(Glob::has_wildcards("file?.txt"));
        assert!(!Glob::has_wildcards("plain name.txt"));
    }
}
