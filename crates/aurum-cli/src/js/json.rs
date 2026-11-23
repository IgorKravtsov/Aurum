//! `JSON.parse` with V8's error messages.
//!
//! Parsing itself is delegated to `serde_json`. Only when the text is
//! invalid, a small scanner re-reads it the way V8's parser does to produce
//! the exact text of the `SyntaxError` (`Expected property name or '}' in
//! JSON at position 1 (line 1 column 2)`), because that text is part of the
//! CLI's user-visible error messages (`Некорректный JSON в --data: ...`).

use serde_json::Value;

/// `JSON.parse(text)`; the error is V8's `SyntaxError` message.
pub fn parse(text: &str) -> Result<Value, String> {
    match serde_json::from_str::<Value>(text) {
        Ok(value) => Ok(value),
        Err(error) => Err(v8_error_message(text).unwrap_or_else(|| error.to_string())),
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Token {
    Eos,
    Str,
    Num,
    LBrace,
    RBrace,
    LBrack,
    RBrack,
    Colon,
    Comma,
    True,
    False,
    Null,
    Illegal,
}

struct Scanner {
    src: Vec<u16>,
    pos: usize,
}

type Fail = String;

const MAX_CONTEXT: usize = 10;

impl Scanner {
    fn skip_whitespace(&mut self) {
        while matches!(self.src.get(self.pos), Some(0x20 | 0x09 | 0x0A | 0x0D)) {
            self.pos += 1;
        }
    }

    fn peek(&mut self) -> Token {
        self.skip_whitespace();
        match self.src.get(self.pos).copied() {
            None => Token::Eos,
            Some(0x22) => Token::Str,
            Some(0x2D | 0x30..=0x39) => Token::Num,
            Some(0x7B) => Token::LBrace,
            Some(0x7D) => Token::RBrace,
            Some(0x5B) => Token::LBrack,
            Some(0x5D) => Token::RBrack,
            Some(0x3A) => Token::Colon,
            Some(0x2C) => Token::Comma,
            Some(0x74) => Token::True,
            Some(0x66) => Token::False,
            Some(0x6E) => Token::Null,
            Some(_) => Token::Illegal,
        }
    }

    fn current(&self) -> Option<u16> {
        self.src.get(self.pos).copied()
    }

    /// 1-based line/column of the current position, columns in UTF-16 units.
    fn location(&self) -> (usize, usize) {
        let (mut line, mut column) = (1, 1);
        let mut index = 0;
        while index < self.pos.min(self.src.len()) {
            match self.src[index] {
                0x0A => {
                    line += 1;
                    column = 1;
                }
                0x0D => {
                    if self.src.get(index + 1) == Some(&0x0A) {
                        index += 1;
                    }
                    line += 1;
                    column = 1;
                }
                _ => column += 1,
            }
            index += 1;
        }
        (line, column)
    }

    /// `ReportUnexpectedToken(token, message)` at the current position.
    fn report(&mut self, token: Token, message: Option<&str>) -> Fail {
        let position = self.pos;
        let (line, column) = self.location();
        let suffix = format!("at position {position} (line {line} column {column})");
        if let Some(message) = message {
            // Only this template already ends with "JSON" itself.
            let preposition = if message.ends_with("after JSON") { "" } else { " in JSON" };
            return format!("{message}{preposition} {suffix}");
        }
        match token {
            Token::Eos => "Unexpected end of JSON input".to_string(),
            Token::Num => format!("Unexpected number in JSON {suffix}"),
            Token::Str => format!("Unexpected string in JSON {suffix}"),
            _ => {
                let whole = String::from_utf16_lossy(&self.src);
                if matches!(whole.as_str(), "NaN" | "Infinity" | "undefined" | "[object Object]") {
                    return format!("\"{whole}\" is not valid JSON");
                }
                let unit = String::from_utf16_lossy(&self.src[position..=position]);
                let length = self.src.len();
                if length < MAX_CONTEXT * 2 + 1 {
                    format!("Unexpected token '{unit}', \"{whole}\" is not valid JSON")
                } else if position < MAX_CONTEXT {
                    let part = String::from_utf16_lossy(&self.src[..position + MAX_CONTEXT]);
                    format!("Unexpected token '{unit}', \"{part}\"... is not valid JSON")
                } else if position < length - MAX_CONTEXT {
                    let part = String::from_utf16_lossy(&self.src[position - MAX_CONTEXT..position + MAX_CONTEXT]);
                    format!("Unexpected token '{unit}', ...\"{part}\"... is not valid JSON")
                } else {
                    let part = String::from_utf16_lossy(&self.src[position - MAX_CONTEXT..]);
                    format!("Unexpected token '{unit}', ...\"{part}\" is not valid JSON")
                }
            }
        }
    }

    fn expect_next(&mut self, token: Token, message: &str) -> Result<(), Fail> {
        let found = self.peek();
        if found == token {
            self.pos += 1;
            Ok(())
        } else {
            Err(self.report(found, Some(message)))
        }
    }

    fn string(&mut self) -> Result<(), Fail> {
        self.pos += 1; // opening quote
        loop {
            match self.current() {
                None => return Err(self.report(Token::Illegal, Some("Unterminated string"))),
                Some(0x22) => {
                    self.pos += 1;
                    return Ok(());
                }
                Some(0x5C) => {
                    self.pos += 1;
                    match self.current() {
                        Some(0x22 | 0x5C | 0x2F | 0x62 | 0x66 | 0x6E | 0x72 | 0x74) => self.pos += 1,
                        Some(0x75) => {
                            for _ in 0..4 {
                                self.pos += 1;
                                let hex = self
                                    .current()
                                    .and_then(|unit| char::from_u32(u32::from(unit)))
                                    .is_some_and(|c| c.is_ascii_hexdigit());
                                if !hex {
                                    return Err(self.report(Token::Illegal, Some("Bad Unicode escape")));
                                }
                            }
                            self.pos += 1;
                        }
                        None => return Err(self.report(Token::Eos, None)),
                        // Outside Latin-1 V8 gives up with a plain unexpected-token message.
                        Some(unit) if unit > 0xFF => return Err(self.report(Token::Illegal, None)),
                        _ => return Err(self.report(Token::Illegal, Some("Bad escaped character"))),
                    }
                }
                Some(unit) if unit < 0x20 => {
                    return Err(self.report(Token::Illegal, Some("Bad control character in string literal")));
                }
                Some(_) => self.pos += 1,
            }
        }
    }

    fn digits(&mut self) -> usize {
        let start = self.pos;
        while matches!(self.current(), Some(0x30..=0x39)) {
            self.pos += 1;
        }
        self.pos - start
    }

    fn number(&mut self) -> Result<(), Fail> {
        if self.current() == Some(0x2D) {
            self.pos += 1;
        }
        if self.current() == Some(0x30) {
            self.pos += 1;
            if matches!(self.current(), Some(0x30..=0x39)) {
                return Err(self.report(Token::Num, None));
            }
        } else if self.digits() == 0 {
            return Err(self.report(Token::Illegal, Some("No number after minus sign")));
        }
        if self.current() == Some(0x2E) {
            self.pos += 1;
            if self.digits() == 0 {
                return Err(self.report(Token::Illegal, Some("Unterminated fractional number")));
            }
        }
        if matches!(self.current(), Some(0x65 | 0x45)) {
            self.pos += 1;
            if matches!(self.current(), Some(0x2B | 0x2D)) {
                self.pos += 1;
            }
            if self.digits() == 0 {
                return Err(self.report(Token::Illegal, Some("Exponent part is missing a number")));
            }
        }
        Ok(())
    }

    fn literal(&mut self, word: &str) -> Result<(), Fail> {
        for expected in word.bytes() {
            match self.current() {
                Some(unit) if unit == u16::from(expected) => self.pos += 1,
                // The offending character is reported as the token it would be elsewhere.
                Some(0x22) => return Err(self.report(Token::Str, None)),
                Some(0x2D | 0x30..=0x39) => return Err(self.report(Token::Num, None)),
                Some(_) => return Err(self.report(Token::Illegal, None)),
                None => return Err(self.report(Token::Eos, None)),
            }
        }
        Ok(())
    }

    fn value(&mut self) -> Result<(), Fail> {
        match self.peek() {
            Token::Str => self.string(),
            Token::Num => self.number(),
            Token::True => self.literal("true"),
            Token::False => self.literal("false"),
            Token::Null => self.literal("null"),
            Token::LBrack => {
                self.pos += 1;
                if self.peek() == Token::RBrack {
                    self.pos += 1;
                    return Ok(());
                }
                loop {
                    self.value()?;
                    match self.peek() {
                        Token::Comma => self.pos += 1,
                        Token::RBrack => {
                            self.pos += 1;
                            return Ok(());
                        }
                        other => return Err(self.report(other, Some("Expected ',' or ']' after array element"))),
                    }
                }
            }
            Token::LBrace => {
                self.pos += 1;
                if self.peek() == Token::RBrace {
                    self.pos += 1;
                    return Ok(());
                }
                self.expect_next(Token::Str, "Expected property name or '}'")?;
                self.pos -= 1;
                loop {
                    self.string()?;
                    self.expect_next(Token::Colon, "Expected ':' after property name")?;
                    self.value()?;
                    match self.peek() {
                        Token::Comma => {
                            self.pos += 1;
                            self.expect_next(Token::Str, "Expected double-quoted property name")?;
                            self.pos -= 1;
                        }
                        Token::RBrace => {
                            self.pos += 1;
                            return Ok(());
                        }
                        other => return Err(self.report(other, Some("Expected ',' or '}' after property value"))),
                    }
                }
            }
            other => Err(self.report(other, None)),
        }
    }
}

/// V8's message for invalid JSON, or `None` when the scanner finds no error
/// (the caller then falls back to serde_json's own message).
fn v8_error_message(text: &str) -> Option<String> {
    let mut scanner = Scanner { src: text.encode_utf16().collect(), pos: 0 };
    let outcome = scanner.value().and_then(|()| match scanner.peek() {
        Token::Eos => Ok(()),
        other => Err(scanner.report(other, Some("Unexpected non-whitespace character after JSON"))),
    });
    outcome.err()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(text: &str) -> String {
        parse(text).expect_err("must be invalid")
    }

    #[test]
    fn matches_v8_messages() {
        assert_eq!(message(""), "Unexpected end of JSON input");
        assert_eq!(message("{"), "Expected property name or '}' in JSON at position 1 (line 1 column 2)");
        assert_eq!(message("{bad"), "Expected property name or '}' in JSON at position 1 (line 1 column 2)");
        assert_eq!(
            message("{\"a\":1"),
            "Expected ',' or '}' after property value in JSON at position 6 (line 1 column 7)"
        );
        assert_eq!(message("[1,"), "Unexpected end of JSON input");
        assert_eq!(message("nope"), "Unexpected token 'o', \"nope\" is not valid JSON");
        assert_eq!(message("undefined"), "\"undefined\" is not valid JSON");
        assert_eq!(message("1 2"), "Unexpected non-whitespace character after JSON at position 2 (line 1 column 3)");
        assert_eq!(message("\"abc"), "Unterminated string in JSON at position 4 (line 1 column 5)");
        assert_eq!(message("[01]"), "Unexpected number in JSON at position 2 (line 1 column 3)");
    }

    #[test]
    fn valid_json_parses() {
        assert_eq!(parse(" {\"a\": [1, 2.5, null]} ").unwrap()["a"][1], 2.5);
    }
}
