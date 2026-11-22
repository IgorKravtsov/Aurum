//! A syntax checker that reproduces CPython's `json.loads` errors.
//!
//! FastAPI reports an invalid JSON body as `json_invalid` with
//! `json.JSONDecodeError`'s `msg` and `pos` (a character index). serde_json's
//! errors don't map onto those one to one (it reports a different offset for
//! a truncated object, has no "trailing comma" error, ...), so when serde
//! rejects a body we re-scan it with a port of CPython's C scanner
//! (`Modules/_json.c`, Python 3.13+ messages) to get the exact message and
//! position. Only syntax is checked — no values are built.

/// `(position, message)` of the error `json.loads(text)` would raise, or
/// `None` when Python would accept the text (e.g. `NaN`, which serde
/// rejects).
pub(crate) fn decode_error(text: &str) -> Option<(usize, &'static str)> {
    let chars: Vec<char> = text.chars().collect();
    Scanner { s: &chars }.document().err()
}

type Scan = Result<usize, (usize, &'static str)>;

struct Scanner<'a> {
    s: &'a [char],
}

const EXPECTING_VALUE: &str = "Expecting value";

impl Scanner<'_> {
    fn at(&self, idx: usize) -> Option<char> {
        self.s.get(idx).copied()
    }

    fn is(&self, idx: usize, c: char) -> bool {
        self.at(idx) == Some(c)
    }

    /// `WHITESPACE = re.compile(r'[ \t\n\r]*')`.
    fn skip_ws(&self, mut idx: usize) -> usize {
        while matches!(self.at(idx), Some(' ' | '\t' | '\n' | '\r')) {
            idx += 1;
        }
        idx
    }

    /// `JSONDecoder.decode`: one value surrounded by whitespace.
    fn document(&self) -> Scan {
        let end = self.value(self.skip_ws(0))?;
        let end = self.skip_ws(end);
        if end != self.s.len() {
            return Err((end, "Extra data"));
        }
        Ok(end)
    }

    /// `scan_once_unicode`: a StopIteration there becomes "Expecting value".
    fn value(&self, idx: usize) -> Scan {
        let rest = |word: &str| {
            self.s.get(idx..idx + word.chars().count()).is_some_and(|w| w.iter().copied().eq(word.chars()))
        };
        match self.at(idx) {
            None => Err((idx, EXPECTING_VALUE)),
            Some('"') => self.string(idx + 1),
            Some('{') => self.object(idx + 1),
            Some('[') => self.array(idx + 1),
            Some('n') if rest("null") => Ok(idx + 4),
            Some('t') if rest("true") => Ok(idx + 4),
            Some('f') if rest("false") => Ok(idx + 5),
            Some('N') if rest("NaN") => Ok(idx + 3),
            Some('I') if rest("Infinity") => Ok(idx + 8),
            Some('-') if rest("-Infinity") => Ok(idx + 9),
            Some(_) => self.number(idx),
        }
    }

    /// `_match_number_unicode`: `-?(0|[1-9]\d*)(\.\d+)?([eE][-+]?\d+)?`,
    /// backtracking over a fraction/exponent that has no digits.
    fn number(&self, start: usize) -> Scan {
        let digit = |i: usize| self.at(i).is_some_and(|c| c.is_ascii_digit());
        let mut idx = start;
        if self.is(idx, '-') {
            idx += 1;
        }
        match self.at(idx) {
            Some('1'..='9') => {
                idx += 1;
                while digit(idx) {
                    idx += 1;
                }
            }
            Some('0') => idx += 1,
            _ => return Err((start, EXPECTING_VALUE)),
        }
        if self.is(idx, '.') && digit(idx + 1) {
            idx += 2;
            while digit(idx) {
                idx += 1;
            }
        }
        if matches!(self.at(idx), Some('e' | 'E')) {
            let mut e = idx + 1;
            if matches!(self.at(e), Some('-' | '+')) {
                e += 1;
            }
            if digit(e) {
                while digit(e) {
                    e += 1;
                }
                idx = e;
            }
        }
        Ok(idx)
    }

    /// `scanstring_unicode` (strict): `end` is just past the opening quote.
    fn string(&self, mut end: usize) -> Scan {
        let begin = end - 1;
        loop {
            let mut next = end;
            let mut c = None;
            while let Some(d) = self.at(next) {
                if d == '"' || d == '\\' {
                    c = Some(d);
                    break;
                }
                if (d as u32) <= 0x1f {
                    return Err((next, "Invalid control character at"));
                }
                next += 1;
            }
            match c {
                Some('"') => return Ok(next + 1),
                Some(_) => {}
                None => return Err((begin, "Unterminated string starting at")),
            }
            // A backslash at `next`.
            next += 1;
            let Some(esc) = self.at(next) else {
                return Err((begin, "Unterminated string starting at"));
            };
            if esc != 'u' {
                if !matches!(esc, '"' | '\\' | '/' | 'b' | 'f' | 'n' | 'r' | 't') {
                    return Err((next - 1, "Invalid \\escape"));
                }
                end = next + 1;
                continue;
            }
            next += 1;
            end = next + 4;
            let hex4 = |from: usize| -> Option<u32> {
                (from..from + 4).try_fold(0u32, |acc, i| Some(acc * 16 + self.at(i)?.to_digit(16)?))
            };
            let Some(c) = hex4(next) else {
                return Err((end - 5, "Invalid \\uXXXX escape"));
            };
            // A high surrogate followed by `\u`: the second escape must be
            // valid (and complete) hex too, whether or not it is a low
            // surrogate. A missing hex digit is reported like a bad one.
            if (0xd800..=0xdbff).contains(&c) && self.is(end, '\\') && self.is(end + 1, 'u') {
                if hex4(end + 2).is_none() {
                    return Err((end + 1, "Invalid \\uXXXX escape"));
                }
                let c2 = hex4(end + 2).unwrap_or(0);
                if (0xdc00..=0xdfff).contains(&c2) {
                    end += 6;
                }
            }
        }
    }

    /// `_parse_object_unicode`: `idx` is just past `{`.
    fn object(&self, idx: usize) -> Scan {
        let mut idx = self.skip_ws(idx);
        if self.is(idx, '}') {
            return Ok(idx + 1);
        }
        loop {
            if !self.is(idx, '"') {
                return Err((idx, "Expecting property name enclosed in double quotes"));
            }
            idx = self.skip_ws(self.string(idx + 1)?);
            if !self.is(idx, ':') {
                return Err((idx, "Expecting ':' delimiter"));
            }
            idx = self.skip_ws(self.value(self.skip_ws(idx + 1))?);
            if self.is(idx, '}') {
                return Ok(idx + 1);
            }
            if !self.is(idx, ',') {
                return Err((idx, "Expecting ',' delimiter"));
            }
            let comma = idx;
            idx = self.skip_ws(idx + 1);
            if self.is(idx, '}') {
                return Err((comma, "Illegal trailing comma before end of object"));
            }
        }
    }

    /// `_parse_array_unicode`: `idx` is just past `[`.
    fn array(&self, idx: usize) -> Scan {
        let mut idx = self.skip_ws(idx);
        if self.is(idx, ']') {
            return Ok(idx + 1);
        }
        loop {
            idx = self.skip_ws(self.value(idx)?);
            if self.is(idx, ']') {
                return Ok(idx + 1);
            }
            if !self.is(idx, ',') {
                return Err((idx, "Expecting ',' delimiter"));
            }
            let comma = idx;
            idx = self.skip_ws(idx + 1);
            if self.is(idx, ']') {
                return Err((comma, "Illegal trailing comma before end of array"));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::decode_error;

    #[test]
    fn matches_cpython() {
        // (input, CPython 3.14 `e.pos`, `e.msg`)
        let cases: &[(&str, usize, &str)] = &[
            ("{\"a\": 1,", 8, "Expecting property name enclosed in double quotes"),
            ("[1,]", 2, "Illegal trailing comma before end of array"),
            ("{\"a\":1,}", 6, "Illegal trailing comma before end of object"),
            ("[1 2]", 3, "Expecting ',' delimiter"),
            ("tru", 0, "Expecting value"),
            ("\"ab\\x\"", 3, "Invalid \\escape"),
            ("\"a\\u12\"", 3, "Invalid \\uXXXX escape"),
            ("-", 0, "Expecting value"),
            ("1.", 1, "Extra data"),
            ("{\"a\" 1}", 5, "Expecting ':' delimiter"),
            ("\"a\nb\"", 2, "Invalid control character at"),
            ("{} x", 3, "Extra data"),
            ("", 0, "Expecting value"),
            (" ", 1, "Expecting value"),
            ("\"\\u1234", 0, "Unterminated string starting at"),
            ("\"\\u123", 2, "Invalid \\uXXXX escape"),
            ("\"\\ud800\\u12", 8, "Invalid \\uXXXX escape"),
            ("[1, 2", 5, "Expecting ',' delimiter"),
            ("{\"a\": tru}", 6, "Expecting value"),
        ];
        for &(input, pos, msg) in cases {
            assert_eq!(decode_error(input), Some((pos, msg)), "{input:?}");
        }
        assert_eq!(decode_error("[NaN, -Infinity, {\"é\": \"\\ud83d\\ude00\"}]"), None);
    }
}
