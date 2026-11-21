//! Argument parsing, a port of `cli/src/args.js` with identical semantics:
//! every `--flag` becomes a JS-like value (string, `true`, number, array...),
//! repeated flags collect into arrays, `--no-x` means `x = false`.

use serde_json::{Map, Value};

use crate::error::{CliError, Result};
use crate::js::{self, json, number_value, string_to_number};

const GLOBAL_VALUE_FLAGS: [&str; 8] =
    ["url", "api_base_url", "user", "password", "password_file", "output", "timeout", "header"];
const GLOBAL_BOOLEAN_FLAGS: [&str; 4] = ["help", "version", "json", "compact"];
/// Flags whose value is kept as the raw string (never turned into a scalar).
const RAW_VALUE_FLAGS: [&str; 4] = ["data", "file", "field", "query"];

pub type Flags = Map<String, Value>;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedArguments {
    pub positionals: Vec<String>,
    pub flags: Flags,
    pub globals: Flags,
}

pub fn is_raw_value_flag(name: &str) -> bool {
    RAW_VALUE_FLAGS.contains(&name)
}

/// `--api-base-url` -> `api_base_url`.
pub fn flag_name(value: &str) -> String {
    value.strip_prefix("--").unwrap_or(value).replace('-', "_")
}

/// `--flag`, but not a negative number such as `--5` (a value, not a flag).
fn looks_like_flag(value: &str) -> bool {
    if !value.starts_with("--") {
        return false;
    }
    let dashes = value.bytes().take_while(|byte| *byte == b'-').count();
    let numeric = dashes <= 3 && value.as_bytes().get(dashes).is_some_and(u8::is_ascii_digit);
    !numeric
}

/// `-?(0|[1-9]\d*)(\.\d+)?` over the whole string.
fn is_plain_number(value: &str) -> bool {
    let unsigned = value.strip_prefix('-').unwrap_or(value);
    let (integer, fraction) = match unsigned.split_once('.') {
        Some((integer, fraction)) => (integer, Some(fraction)),
        None => (unsigned, None),
    };
    let integer_ok = integer == "0"
        || (!integer.starts_with('0') && !integer.is_empty() && integer.bytes().all(|b| b.is_ascii_digit()));
    let fraction_ok = fraction.is_none_or(|digits| !digits.is_empty() && digits.bytes().all(|b| b.is_ascii_digit()));
    integer_ok && fraction_ok
}

/// `true`/`false`/`null`, numbers and JSON arrays/objects are recognised;
/// everything else stays a string.
pub fn parse_scalar(value: &str) -> Result<Value> {
    match value {
        "true" => return Ok(Value::Bool(true)),
        "false" => return Ok(Value::Bool(false)),
        "null" => return Ok(Value::Null),
        _ => {}
    }
    if is_plain_number(value) {
        return Ok(number_value(string_to_number(value)));
    }
    if (value.starts_with('[') && value.ends_with(']')) || (value.starts_with('{') && value.ends_with('}')) {
        return json::parse(value).map_err(|_| CliError::new(format!("Некорректный JSON во флаге: {value}")));
    }
    Ok(Value::String(value.to_string()))
}

/// Repeated flags collect into an array (appending to an existing array).
fn assign_flag(target: &mut Flags, name: &str, value: Value) {
    match target.get_mut(name) {
        Some(Value::Array(items)) => items.push(value),
        Some(existing) => *existing = Value::Array(vec![existing.take(), value]),
        None => {
            target.insert(name.to_string(), value);
        }
    }
}

pub fn parse_arguments(argv: &[String]) -> Result<ParsedArguments> {
    let mut parsed = ParsedArguments::default();
    let mut passthrough = false;

    let mut index = 0;
    while index < argv.len() {
        let token = &argv[index];
        index += 1;
        if token == "--" {
            passthrough = true;
            continue;
        }
        if passthrough || !looks_like_flag(token) {
            parsed.positionals.push(token.clone());
            continue;
        }

        let equals_at = token.find('=');
        let raw_name = equals_at.map_or(token.as_str(), |at| &token[..at]);
        let name = flag_name(raw_name);
        if let Some(negated) = name.strip_prefix("no_") {
            assign_flag(&mut parsed.flags, negated, Value::Bool(false));
            continue;
        }
        if GLOBAL_BOOLEAN_FLAGS.contains(&name.as_str()) {
            parsed.globals.insert(name, Value::Bool(true));
            continue;
        }

        let value = match equals_at {
            Some(at) => Value::String(token[at + 1..].to_string()),
            None => match argv.get(index) {
                Some(next) if !looks_like_flag(next) => {
                    index += 1;
                    Value::String(next.clone())
                }
                _ => Value::Bool(true),
            },
        };

        if GLOBAL_VALUE_FLAGS.contains(&name.as_str()) {
            assign_flag(&mut parsed.globals, &name, value);
        } else if is_raw_value_flag(&name) {
            assign_flag(&mut parsed.flags, &name, value);
        } else {
            let scalar = parse_scalar(&js::to_js_string(&value))?;
            assign_flag(&mut parsed.flags, &name, scalar);
        }
    }

    Ok(parsed)
}

/// `key=value` -> (`key` with `-` turned into `_`, parsed value).
pub fn split_key_value(value: &Value, flag: &str) -> Result<(String, Value)> {
    let text = js::to_js_string(value);
    // `separator < 1` in the JS: no `=` at all, or an empty key.
    match text.find('=') {
        Some(separator) if separator >= 1 => {
            Ok((text[..separator].replace('-', "_"), parse_scalar(&text[separator + 1..])?))
        }
        _ => Err(CliError::new(format!("{flag} ожидает значение в формате key=value"))),
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|item| (*item).to_string()).collect()
    }

    #[test]
    fn parse_scalar_recognises_json_types_and_keeps_plain_strings() {
        assert_eq!(parse_scalar("42.50").unwrap(), json!(42.5));
        assert_eq!(parse_scalar("false").unwrap(), json!(false));
        assert_eq!(parse_scalar("null").unwrap(), Value::Null);
        assert_eq!(parse_scalar("[1,2]").unwrap(), json!([1, 2]));
        assert_eq!(parse_scalar("2026-09-12").unwrap(), json!("2026-09-12"));
        // Not plain numbers: leading zero, trailing dot, exponent.
        assert_eq!(parse_scalar("007").unwrap(), json!("007"));
        assert_eq!(parse_scalar("1.").unwrap(), json!("1."));
        assert_eq!(parse_scalar("1e3").unwrap(), json!("1e3"));
        assert_eq!(parse_scalar("[oops").unwrap(), json!("[oops"));
        assert_eq!(parse_scalar("[oops]").unwrap_err().0, "Некорректный JSON во флаге: [oops]");
    }

    #[test]
    fn parse_arguments_separates_global_options_from_api_fields() {
        let parsed = parse_arguments(&args(&[
            "--api-base-url",
            "https://example.test/api",
            "transactions",
            "create",
            "--account-id",
            "3",
            "--tag-ids",
            "[1,2]",
            "--no-category-id",
        ]))
        .unwrap();
        assert_eq!(parsed.positionals, ["transactions", "create"]);
        assert_eq!(parsed.globals["api_base_url"], "https://example.test/api");
        assert_eq!(Value::Object(parsed.flags), json!({"account_id": 3, "tag_ids": [1, 2], "category_id": false}));
    }

    #[test]
    fn data_stays_a_string_until_the_request_body_is_built() {
        let parsed = parse_arguments(&args(&["accounts", "create", "--data", "{\"name\":\"Cash\"}"])).unwrap();
        assert_eq!(parsed.flags["data"], "{\"name\":\"Cash\"}");
    }

    #[test]
    fn repeated_flags_collect_and_edge_cases_follow_the_js_rules() {
        let parsed = parse_arguments(&args(&[
            "--header",
            "A: 1",
            "--header=B: 2",
            "--user",
            "--compact",
            "x",
            "--",
            "--y",
            "-5",
            "--5",
        ]))
        .unwrap();
        assert_eq!(parsed.globals["header"], json!(["A: 1", "B: 2"]));
        assert_eq!(parsed.globals["user"], json!(true));
        assert_eq!(parsed.globals["compact"], json!(true));
        // `--5` is numeric-looking, so it is a value, not a flag.
        assert_eq!(parsed.positionals, ["x", "--y", "-5", "--5"]);

        let parsed = parse_arguments(&args(&["--n", "1", "--n", "2", "--n", "3", "--flag=a=b"])).unwrap();
        assert_eq!(parsed.flags["n"], json!([1, 2, 3]));
        assert_eq!(parsed.flags["flag"], json!("a=b"));
    }

    #[test]
    fn split_key_value_normalises_the_key() {
        assert_eq!(
            split_key_value(&json!("tag-ids=[1,2]"), "--field").unwrap(),
            ("tag_ids".to_string(), json!([1, 2]))
        );
        assert!(split_key_value(&json!("=x"), "--field").is_err());
        assert_eq!(
            split_key_value(&json!(true), "--query").unwrap_err().0,
            "--query ожидает значение в формате key=value"
        );
    }
}
