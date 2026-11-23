//! Emulation of the JavaScript semantics the Node CLI relied on.
//!
//! The Rust CLI must behave byte-for-byte like the Node one, and much of
//! that behaviour is an accident of JS: `Number("0x10")`, `String([1, 2])`,
//! `JSON.stringify` printing `100.0` as `100` and listing integer-like keys
//! first, `encodeURIComponent`, Node's `fs` error texts. Instead of
//! scattering such quirks around, they live here, one function each, and the
//! rest of the crate works with `serde_json::Value` as the "JS value".

pub mod fs;
pub mod json;

use serde_json::{Map, Number, Value};

/// JS truthiness (`if (value)`).
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(flag) => *flag,
        Value::Number(number) => number.as_f64().is_some_and(|n| n != 0.0 && !n.is_nan()),
        Value::String(text) => !text.is_empty(),
        Value::Array(_) | Value::Object(_) => true,
    }
}

/// JS `String(value)`.
pub fn to_js_string(value: &Value) -> String {
    match value {
        Value::Null => "null".to_string(),
        Value::Bool(flag) => flag.to_string(),
        Value::Number(number) => number_to_string(number.as_f64().unwrap_or(f64::NAN)),
        Value::String(text) => text.clone(),
        // Array.prototype.toString: join(","), where null becomes an empty item.
        Value::Array(items) => items
            .iter()
            .map(|item| if item.is_null() { String::new() } else { to_js_string(item) })
            .collect::<Vec<_>>()
            .join(","),
        Value::Object(_) => "[object Object]".to_string(),
    }
}

/// JS `Number(value)` for the values the CLI can hold.
pub fn to_js_number(value: &Value) -> f64 {
    match value {
        Value::Null => 0.0,
        Value::Bool(flag) => f64::from(u8::from(*flag)),
        Value::Number(number) => number.as_f64().unwrap_or(f64::NAN),
        Value::String(text) => string_to_number(text),
        Value::Array(_) => string_to_number(&to_js_string(value)),
        Value::Object(_) => f64::NAN,
    }
}

/// JS whitespace as trimmed by `Number()`/`String.prototype.trim`.
fn is_js_whitespace(c: char) -> bool {
    c.is_whitespace() || c == '\u{FEFF}'
}

/// JS `Number(string)`: StringNumericLiteral grammar (decimal, `0x`/`0o`/`0b`,
/// `Infinity`), empty string is 0, anything else NaN.
pub fn string_to_number(text: &str) -> f64 {
    let text = text.trim_matches(is_js_whitespace);
    if text.is_empty() {
        return 0.0;
    }
    for (prefix, radix) in [("0x", 16), ("0X", 16), ("0o", 8), ("0O", 8), ("0b", 2), ("0B", 2)] {
        if let Some(digits) = text.strip_prefix(prefix) {
            if digits.is_empty() || !digits.chars().all(|c| c.is_digit(radix)) {
                return f64::NAN;
            }
            return digits
                .chars()
                .fold(0.0, |acc, c| acc * f64::from(radix) + f64::from(c.to_digit(radix).unwrap_or(0)));
        }
    }
    let (sign, unsigned) = match text.as_bytes()[0] {
        b'-' => (-1.0, &text[1..]),
        b'+' => (1.0, &text[1..]),
        _ => (1.0, text),
    };
    if unsigned == "Infinity" {
        return sign * f64::INFINITY;
    }
    if is_decimal_literal(unsigned) { unsigned.parse::<f64>().map_or(f64::NAN, |n| sign * n) } else { f64::NAN }
}

/// `digits [. digits?] [e[+-]digits]` or `. digits [e[+-]digits]`.
fn is_decimal_literal(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut i = 0;
    let digits = |i: &mut usize| {
        let start = *i;
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            *i += 1;
        }
        *i - start
    };
    let integer = digits(&mut i);
    let mut fraction = 0;
    if i < bytes.len() && bytes[i] == b'.' {
        i += 1;
        fraction = digits(&mut i);
    }
    if integer + fraction == 0 {
        return false;
    }
    if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
        i += 1;
        if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
            i += 1;
        }
        if digits(&mut i) == 0 {
            return false;
        }
    }
    i == bytes.len()
}

/// JS `Number::toString` (radix 10): shortest round-trip digits, positional
/// notation for 1e-7 < |x| < 1e21, exponent form otherwise.
pub fn number_to_string(x: f64) -> String {
    if x.is_nan() {
        return "NaN".to_string();
    }
    if x == 0.0 {
        return "0".to_string();
    }
    if x.is_infinite() {
        return if x < 0.0 { "-Infinity" } else { "Infinity" }.to_string();
    }
    let sign = if x < 0.0 { "-" } else { "" };
    // `{:e}` yields the shortest round-trip digits: "1.2345e-7".
    let scientific = format!("{:e}", x.abs());
    let (mantissa, exponent) = scientific.split_once('e').unwrap_or((&scientific, "0"));
    let digits: String = mantissa.chars().filter(|c| *c != '.').collect();
    let k = digits.len() as i32;
    let n = exponent.parse::<i32>().unwrap_or(0) + 1;
    let body = if k <= n && n <= 21 {
        format!("{digits}{}", "0".repeat((n - k) as usize))
    } else if 0 < n && n <= 21 {
        format!("{}.{}", &digits[..n as usize], &digits[n as usize..])
    } else if -6 < n && n <= 0 {
        format!("0.{}{digits}", "0".repeat((-n) as usize))
    } else {
        let sign_e = if n - 1 < 0 { '-' } else { '+' };
        let exp = (n - 1).abs();
        if k == 1 {
            format!("{digits}e{sign_e}{exp}")
        } else {
            format!("{}.{}e{sign_e}{exp}", &digits[..1], &digits[1..])
        }
    };
    format!("{sign}{body}")
}

/// The `Value` a JS number would become: integral values within the safe
/// range are stored as integers (so `42.0` prints as `42` and `-0` as `0`).
pub fn number_value(x: f64) -> Value {
    if x.is_finite() && x.fract() == 0.0 && x.abs() < 9_007_199_254_740_992.0 {
        return Value::from(x as i64);
    }
    Number::from_f64(x).map_or(Value::Null, Value::Number)
}

/// JS `encodeURIComponent`.
pub fn encode_uri_component(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        if byte.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&byte) {
            out.push(byte as char);
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
    }
    out
}

/// `true` when `key` is a canonical array index (`"0"`, `"12"`, not `"012"`),
/// the keys JS enumerates first and in ascending order.
fn array_index(key: &str) -> Option<u32> {
    let canonical = key == "0" || (!key.starts_with('0') && !key.is_empty() && key.bytes().all(|b| b.is_ascii_digit()));
    if !canonical {
        return None;
    }
    key.parse::<u32>().ok().filter(|index| *index != u32::MAX)
}

/// Keys of an object in JS enumeration order: integer-like keys ascending,
/// then the rest in insertion order.
pub fn ordered_entries(map: &Map<String, Value>) -> Vec<(&String, &Value)> {
    let mut indexed: Vec<(u32, &String, &Value)> = Vec::new();
    let mut named: Vec<(&String, &Value)> = Vec::new();
    for (key, value) in map {
        match array_index(key) {
            Some(index) => indexed.push((index, key, value)),
            None => named.push((key, value)),
        }
    }
    indexed.sort_by_key(|(index, ..)| *index);
    indexed.into_iter().map(|(_, key, value)| (key, value)).chain(named).collect()
}

/// JS `JSON.stringify(value, null, indent)`; `indent` of 0 means compact.
pub fn stringify(value: &Value, indent: usize) -> String {
    let mut out = String::new();
    write_value(value, indent, 0, &mut out);
    out
}

fn write_value(value: &Value, indent: usize, depth: usize, out: &mut String) {
    let newline = |out: &mut String, depth: usize| {
        if indent > 0 {
            out.push('\n');
            out.push_str(&" ".repeat(indent * depth));
        }
    };
    match value {
        Value::Null => out.push_str("null"),
        Value::Bool(flag) => out.push_str(if *flag { "true" } else { "false" }),
        Value::Number(number) => {
            let x = number.as_f64().unwrap_or(f64::NAN);
            out.push_str(&if x.is_finite() { number_to_string(x) } else { "null".to_string() });
        }
        // serde_json escapes exactly the characters JSON.stringify does.
        Value::String(text) => out.push_str(&serde_json::to_string(text).unwrap_or_default()),
        Value::Array(items) if items.is_empty() => out.push_str("[]"),
        Value::Array(items) => {
            out.push('[');
            for (index, item) in items.iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                write_value(item, indent, depth + 1, out);
            }
            newline(out, depth);
            out.push(']');
        }
        Value::Object(map) if map.is_empty() => out.push_str("{}"),
        Value::Object(map) => {
            out.push('{');
            for (index, (key, item)) in ordered_entries(map).into_iter().enumerate() {
                if index > 0 {
                    out.push(',');
                }
                newline(out, depth + 1);
                out.push_str(&serde_json::to_string(key).unwrap_or_default());
                out.push_str(if indent > 0 { ": " } else { ":" });
                write_value(item, indent, depth + 1, out);
            }
            newline(out, depth);
            out.push('}');
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn number_formatting_matches_js() {
        for (input, expected) in [
            (42.5, "42.5"),
            (100.0, "100"),
            (-0.0, "0"),
            (0.000001, "0.000001"),
            (1e-7, "1e-7"),
            (1.5e-7, "1.5e-7"),
            (1e21, "1e+21"),
            (123456789012345680000.0, "123456789012345680000"),
            (1.2345678901234567e19, "12345678901234567000"),
            (0.1 + 0.2, "0.30000000000000004"),
        ] {
            assert_eq!(number_to_string(input), expected, "{input}");
        }
    }

    #[test]
    fn string_to_number_follows_js_grammar() {
        assert_eq!(string_to_number(" 42 "), 42.0);
        assert_eq!(string_to_number(""), 0.0);
        assert_eq!(string_to_number("0x10"), 16.0);
        assert_eq!(string_to_number("1e3"), 1000.0);
        assert_eq!(string_to_number(".5"), 0.5);
        assert_eq!(string_to_number("5."), 5.0);
        assert!(string_to_number("abc").is_nan());
        assert!(string_to_number("1_000").is_nan());
        assert!(string_to_number("inf").is_nan());
        assert!(string_to_number("-").is_nan());
        assert_eq!(string_to_number("-Infinity"), f64::NEG_INFINITY);
    }

    #[test]
    fn stringify_matches_json_stringify() {
        let value = json!({"b": 1.0, "2": [], "a": {"x": null, "y": [1, "t\n"]}, "1": {}});
        assert_eq!(
            stringify(&value, 2),
            "{\n  \"1\": {},\n  \"2\": [],\n  \"b\": 1,\n  \"a\": {\n    \"x\": null,\n    \"y\": [\n      1,\n      \"t\\n\"\n    ]\n  }\n}"
        );
        assert_eq!(stringify(&value, 0), r#"{"1":{},"2":[],"b":1,"a":{"x":null,"y":[1,"t\n"]}}"#);
    }

    #[test]
    fn js_strings_and_truthiness() {
        assert_eq!(to_js_string(&json!(["a", null, 2])), "a,,2");
        assert_eq!(to_js_string(&json!(true)), "true");
        assert!(truthy(&json!([])));
        assert!(!truthy(&json!("")));
        assert!(!truthy(&json!(0)));
    }

    #[test]
    fn uri_component_encoding() {
        assert_eq!(encode_uri_component("a b/ё$"), "a%20b%2F%D1%91%24");
        assert_eq!(encode_uri_component("A-z_0.!~*'()"), "A-z_0.!~*'()");
    }
}
