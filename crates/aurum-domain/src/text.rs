//! Shared text-formatting helpers — port of the former Python backend's `app/core/text.py`,
//! plus the bits of Python's `str` semantics ports keep needing.

/// Python's `str.isspace()` for one char. Rust's `char::is_whitespace`
/// misses U+001C..U+001F, which Python treats as whitespace.
pub fn is_py_space(c: char) -> bool {
    c.is_whitespace() || ('\u{1c}'..='\u{1f}').contains(&c)
}

/// Python's `str.strip()` (no arguments).
pub fn py_strip(value: &str) -> &str {
    value.trim_matches(is_py_space)
}

/// Uppercase the first character, leaving the rest of the string untouched.
///
/// Unlike str.capitalize()/str.title(), this never lowercases the rest of the
/// string — "iPhone 15" stays "iPhone 15", not "Iphone 15".
pub fn capitalize_first_letter(value: &str) -> String {
    let mut chars = value.chars();
    match chars.next() {
        None => String::new(),
        Some(first) => first.to_uppercase().chain(chars).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strip_and_capitalize_match_python() {
        assert_eq!(py_strip("\u{1f}  Trip \t\n"), "Trip");
        assert_eq!(capitalize_first_letter("iPhone 15"), "IPhone 15");
        assert_eq!(capitalize_first_letter("ßx"), "SSx");
        assert_eq!(capitalize_first_letter(""), "");
    }
}
