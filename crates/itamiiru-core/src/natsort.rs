//! Natural sort: `file2` comes before `file10`.
//!
//! Runs of ASCII digits compare as numbers; everything else compares byte by
//! byte, ignoring ASCII case. Non-ASCII letters compare by their raw bytes.
//! Ties are broken by the plain bytes, so the order is always the same.

use std::cmp::Ordering;
use std::ffi::OsStr;

pub fn compare(a: &OsStr, b: &OsStr) -> Ordering {
    let a = a.as_encoded_bytes();
    let b = b.as_encoded_bytes();
    let mut i = 0;
    let mut j = 0;

    while i < a.len() && j < b.len() {
        if a[i].is_ascii_digit() && b[j].is_ascii_digit() {
            let start_a = i;
            while i < a.len() && a[i].is_ascii_digit() {
                i += 1;
            }
            let start_b = j;
            while j < b.len() && b[j].is_ascii_digit() {
                j += 1;
            }
            let ord = compare_numbers(&a[start_a..i], &b[start_b..j]);
            if ord != Ordering::Equal {
                return ord;
            }
        } else {
            let ca = a[i].to_ascii_lowercase();
            let cb = b[j].to_ascii_lowercase();
            if ca != cb {
                return ca.cmp(&cb);
            }
            i += 1;
            j += 1;
        }
    }

    // One name is a prefix of the other: the shorter one comes first.
    let rest_a = a.len() - i;
    let rest_b = b.len() - j;
    return rest_a.cmp(&rest_b).then_with(|| a.cmp(b));
}

/// Compares two runs of digits by value, without parsing, so any length works.
fn compare_numbers(a: &[u8], b: &[u8]) -> Ordering {
    let a = trim_leading_zeros(a);
    let b = trim_leading_zeros(b);
    // More digits means a bigger number; with equal length, bytes compare like digits.
    return a.len().cmp(&b.len()).then_with(|| a.cmp(b));
}

fn trim_leading_zeros(digits: &[u8]) -> &[u8] {
    let mut start = 0;
    while start < digits.len() && digits[start] == b'0' {
        start += 1;
    }
    return &digits[start..];
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sorted(names: &[&str]) -> Vec<String> {
        let mut v: Vec<&str> = names.to_vec();
        v.sort_by(|a, b| compare(OsStr::new(a), OsStr::new(b)));
        return v.into_iter().map(String::from).collect();
    }

    #[test]
    fn numbers_by_value() {
        assert_eq!(
            sorted(&["file10", "file2", "file1", "file20"]),
            ["file1", "file2", "file10", "file20"]
        );
    }

    #[test]
    fn ignores_ascii_case() {
        assert_eq!(
            sorted(&["banana", "Apple", "cherry"]),
            ["Apple", "banana", "cherry"]
        );
    }

    #[test]
    fn prefix_comes_first() {
        assert_eq!(sorted(&["abc", "ab", "abcd"]), ["ab", "abc", "abcd"]);
        assert_eq!(sorted(&["a1b", "a1"]), ["a1", "a1b"]);
    }

    #[test]
    fn leading_zeros_and_ties() {
        assert_eq!(sorted(&["a1", "a01", "a001"]), ["a001", "a01", "a1"]);
        assert_eq!(sorted(&["a", "A"]), ["A", "a"]);
        assert_eq!(
            compare(OsStr::new("same"), OsStr::new("same")),
            Ordering::Equal
        );
    }

    #[test]
    fn huge_numbers_dont_overflow() {
        let big = "x99999999999999999999999999999999";
        let bigger = "x100000000000000000000000000000000";
        assert_eq!(sorted(&[bigger, big]), [big, bigger]);
    }

    #[test]
    fn several_number_runs() {
        assert_eq!(
            sorted(&["v1.10.2", "v1.9.10", "v1.9.2", "v1.10.0"]),
            ["v1.9.2", "v1.9.10", "v1.10.0", "v1.10.2"]
        );
    }

    #[test]
    fn digits_before_letters() {
        assert_eq!(sorted(&["b", "2", "a", "10"]), ["2", "10", "a", "b"]);
    }

    #[test]
    fn empty_names() {
        assert_eq!(sorted(&["a", ""]), ["", "a"]);
    }
}
