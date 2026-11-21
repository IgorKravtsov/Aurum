//! Turning a command line into an HTTP request description, a port of
//! `buildRequest`/`buildGenericRequest` from `cli/src/client.js`. Nothing
//! here touches the network; sending lives in `transport`.

use std::path::Path;

use aurum_client::Url;
use serde_json::{Map, Value};

use crate::args::{Flags, is_raw_value_flag, split_key_value};
use crate::commands::{Command, path_parameters};
use crate::config::resolve_api_base_url;
use crate::env::Env;
use crate::error::{CliError, Result};
use crate::js::{self, fs, json, ordered_entries};

/// What to send: everything `aurum-client` needs for one `request` call.
#[derive(Debug, Clone, PartialEq)]
pub struct BuiltRequest {
    pub method: String,
    /// Normalised API root (no trailing slash), see `config::normalize_api_base_url`.
    pub base_url: String,
    /// Path with path parameters already substituted, always starting with `/`.
    pub path: String,
    pub query: Vec<(String, String)>,
    pub body: Option<Value>,
}

/// The parts of a command the builder needs; owned so that the generic
/// `aurum request` can synthesise one at runtime.
struct Spec {
    name: String,
    method: String,
    path: String,
    query: Vec<String>,
    body: bool,
}

impl From<&Command> for Spec {
    fn from(command: &Command) -> Self {
        Self {
            name: command.name.to_string(),
            method: command.method.to_string(),
            path: command.path.to_string(),
            query: command.query.iter().map(|name| (*name).to_string()).collect(),
            body: command.body,
        }
    }
}

/// JS `valuesOf`: `undefined` -> none, array -> its items, otherwise the value itself.
fn values_of(value: Option<&Value>) -> Vec<&Value> {
    match value {
        None => Vec::new(),
        Some(Value::Array(items)) => items.iter().collect(),
        Some(other) => vec![other],
    }
}

fn json_from_file(path: &str) -> Result<Value> {
    let failed = |reason: String| CliError::new(format!("Не удалось прочитать JSON из {path}: {reason}"));
    let text = fs::read_to_string(Path::new(path)).map_err(|error| failed(fs::read_error_message(&error, path)))?;
    json::parse(&text).map_err(failed)
}

fn append_query(query: &mut Vec<(String, String)>, name: &str, value: &Value) {
    for item in values_of(Some(value)) {
        let text = match item {
            Value::Null => continue,
            Value::Array(_) | Value::Object(_) => js::stringify(item, 0),
            other => js::to_js_string(other),
        };
        query.push((name.to_string(), text));
    }
}

/// The flag's single value, `None` when the flag is absent.
fn defined<'a>(flags: &'a Flags, name: &str) -> Option<&'a Value> {
    flags.get(name)
}

fn build(spec: &Spec, positionals: &[String], flags: &Flags, globals: &Flags, env: &Env) -> Result<BuiltRequest> {
    let command_words = spec.name.split(' ').count();
    let arguments: &[String] = positionals.get(command_words..).unwrap_or(&[]);
    let mut path = spec.path.clone();
    let parameters = path_parameters(&path);
    if arguments.len() < parameters.len() {
        let expected = parameters.iter().map(|name| format!("<{name}>")).collect::<Vec<_>>().join(" ");
        return Err(CliError::new(format!("Команда {} ожидает: {expected}", spec.name)));
    }
    for (name, argument) in parameters.iter().zip(arguments) {
        path = path.replacen(&format!("{{{name}}}"), &js::encode_uri_component(argument), 1);
    }
    if arguments.len() > parameters.len() {
        return Err(CliError::new(format!("Лишние аргументы: {}", arguments[parameters.len()..].join(" "))));
    }

    let base_url = resolve_api_base_url(globals, env)?;
    let path = if path.starts_with('/') { path } else { format!("/{path}") };
    // Validated here (not only at send time) so a bad URL is reported before
    // any flag or body problem, exactly like `new URL(...)` in the Node CLI.
    Url::parse(&format!("{base_url}{path}")).map_err(|_| CliError::new("Invalid URL"))?;

    let mut query: Vec<(String, String)> = Vec::new();
    let mut body_fields = Map::new();
    for (name, value) in ordered_entries(flags) {
        if is_raw_value_flag(name) {
            continue;
        }
        if !spec.body || spec.query.contains(name) {
            append_query(&mut query, name, value);
        } else {
            body_fields.insert(name.clone(), value.clone());
        }
    }
    for value in values_of(flags.get("query")) {
        let (name, parsed) = split_key_value(value, "--query")?;
        append_query(&mut query, &name, &parsed);
    }

    let mut body = None;
    if spec.body {
        let (data, file) = (defined(flags, "data"), defined(flags, "file"));
        if data.is_some() && file.is_some() {
            return Err(CliError::new("Используйте только один из флагов --data и --file"));
        }
        if matches!(data, Some(Value::Array(_))) || matches!(file, Some(Value::Array(_))) {
            return Err(CliError::new("Флаги --data и --file нельзя повторять"));
        }
        let mut document = Value::Object(Map::new());
        if let Some(file) = file {
            document = json_from_file(&js::to_js_string(file))?;
        }
        if let Some(data) = data {
            document = json::parse(&js::to_js_string(data))
                .map_err(|reason| CliError::new(format!("Некорректный JSON в --data: {reason}")))?;
        }
        let Value::Object(mut merged) = document else {
            return Err(CliError::new("Тело запроса должно быть JSON-объектом"));
        };
        for value in values_of(flags.get("field")) {
            let (name, parsed) = split_key_value(value, "--field")?;
            body_fields.insert(name, parsed);
        }
        for (name, value) in ordered_entries(&body_fields) {
            merged.insert(name.clone(), value.clone());
        }
        body = Some(Value::Object(merged));
    } else if flags.contains_key("data") || flags.contains_key("file") || flags.contains_key("field") {
        return Err(CliError::new(format!("Команда {} не принимает тело запроса", spec.name)));
    }

    Ok(BuiltRequest { method: spec.method.clone(), base_url, path, query, body })
}

pub fn build_request(
    command: &Command,
    positionals: &[String],
    flags: &Flags,
    globals: &Flags,
    env: &Env,
) -> Result<BuiltRequest> {
    build(&Spec::from(command), positionals, flags, globals, env)
}

/// `aurum request <METHOD> <PATH>`: any endpoint, with or without a named command.
pub fn build_generic_request(
    positionals: &[String],
    flags: &Flags,
    globals: &Flags,
    env: &Env,
) -> Result<BuiltRequest> {
    let method_value = positionals.get(1).filter(|value| !value.is_empty());
    let path = positionals.get(2).filter(|value| !value.is_empty());
    let (Some(method_value), Some(path)) = (method_value, path) else {
        return Err(CliError::new("Использование: aurum request <METHOD> <PATH> [флаги]"));
    };
    let method = method_value.to_uppercase();
    if method.is_empty() || !method.bytes().all(|byte| byte.is_ascii_uppercase()) {
        return Err(CliError::new(format!("Некорректный HTTP-метод: {method_value}")));
    }
    let spec = Spec {
        name: "request".to_string(),
        body: !matches!(method.as_str(), "GET" | "HEAD" | "DELETE"),
        method,
        path: path.clone(),
        query: Vec::new(),
    };
    build(&spec, &["request".to_string()], flags, globals, env)
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::args::parse_arguments;
    use crate::commands::find_command;

    fn words(text: &str) -> Vec<String> {
        text.split(' ').map(str::to_string).collect()
    }

    /// Builds a request the way `run` does: `argv` is parsed first.
    fn build_from(argv: &str, env: &Env) -> Result<BuiltRequest> {
        let parsed = parse_arguments(&shell_split(argv))?;
        if parsed.positionals[0] == "request" {
            return build_generic_request(&parsed.positionals, &parsed.flags, &parsed.globals, env);
        }
        let command = find_command(&parsed.positionals).expect("known command");
        build_request(command, &parsed.positionals, &parsed.flags, &parsed.globals, env)
    }

    /// Whitespace split that keeps single-quoted chunks together.
    fn shell_split(text: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut current = String::new();
        let mut quoted = false;
        for c in text.chars() {
            match c {
                '\'' => quoted = !quoted,
                ' ' if !quoted => parts.push(std::mem::take(&mut current)),
                other => current.push(other),
            }
        }
        parts.push(current);
        parts.retain(|part| !part.is_empty());
        parts
    }

    fn env_with_url() -> Env {
        Env::from_pairs([
            ("AURUM_CONFIG_FILE", "/nonexistent/aurum.json"),
            ("AURUM_API_BASE_URL", "https://finance.example/api/"),
        ])
    }

    #[test]
    fn build_request_assembles_path_query_and_json_body_from_flags() {
        let globals = parse_arguments(&words("--api-base-url https://finance.example/api/")).unwrap().globals;
        let flags = serde_json::from_value(json!({"network": "Ethereum"})).unwrap();
        let command = find_command(&words("crypto holding-update 17")).unwrap();
        let request =
            build_request(command, &words("crypto holding-update 17"), &flags, &globals, &Env::default()).unwrap();
        assert_eq!(request.method, "PATCH");
        assert_eq!(request.base_url, "https://finance.example/api");
        assert_eq!(request.path, "/crypto/holdings/17");
        assert_eq!(request.body, Some(json!({"network": "Ethereum"})));
    }

    #[test]
    fn file_and_direct_flags_are_merged_and_flags_win() {
        let directory = tempfile::tempdir().unwrap();
        let input = directory.path().join("account.json");
        std::fs::write(&input, r#"{"name":"Old","type":"cash"}"#).unwrap();
        let request =
            build_from(&format!("accounts create --file {} --name New", input.display()), &env_with_url()).unwrap();
        assert_eq!(request.body, Some(json!({"name": "New", "type": "cash"})));
    }

    #[test]
    fn flags_go_to_the_query_on_commands_without_a_body_and_for_declared_names() {
        let request = build_from(
            "transactions list --year 2026 --tag-id 1 --tag-id 2 --query b-c=[1,2] --no-search",
            &env_with_url(),
        )
        .unwrap();
        let expected =
            [("year", "2026"), ("tag_id", "1"), ("tag_id", "2"), ("search", "false"), ("b_c", "1"), ("b_c", "2")];
        assert_eq!(request.query, expected.map(|(k, v)| (k.to_string(), v.to_string())));
        assert_eq!(request.body, None);

        let request = build_from("crypto refresh --portfolio-id 3", &env_with_url()).unwrap();
        assert_eq!(request.query, [("portfolio_id".to_string(), "3".to_string())]);
        assert_eq!(request.body, None);
    }

    #[test]
    fn path_parameters_are_required_exact_and_encoded() {
        let env = env_with_url();
        assert_eq!(build_from("accounts update", &env).unwrap_err().0, "Команда accounts update ожидает: <account_id>");
        assert_eq!(build_from("accounts delete 1 2 3", &env).unwrap_err().0, "Лишние аргументы: 2 3");
        assert_eq!(build_from("accounts delete a/b", &env).unwrap().path, "/accounts/a%2Fb");
    }

    #[test]
    fn body_flags_are_validated() {
        let env = env_with_url();
        let message = |argv: &str| build_from(argv, &env).unwrap_err().0;
        assert_eq!(message("accounts list --data {}"), "Команда accounts list не принимает тело запроса");
        assert_eq!(message("accounts create --data {} --file x"), "Используйте только один из флагов --data и --file");
        assert_eq!(message("accounts create --data {} --data {}"), "Флаги --data и --file нельзя повторять");
        assert_eq!(message("accounts create --data [1]"), "Тело запроса должно быть JSON-объектом");
        assert_eq!(
            message("accounts create --data {oops"),
            "Некорректный JSON в --data: Expected property name or '}' in JSON at position 1 (line 1 column 2)"
        );
        assert_eq!(message("accounts create --field nokey"), "--field ожидает значение в формате key=value");
        assert!(
            message("accounts create --file /nonexistent/x.json").starts_with(
                "Не удалось прочитать JSON из /nonexistent/x.json: ENOENT: no such file or directory, open"
            )
        );
    }

    #[test]
    fn generic_request_uses_the_method_to_decide_about_a_body() {
        let env = env_with_url();
        let get = build_from("request get /future-endpoint --query page=2", &env).unwrap();
        assert_eq!((get.method.as_str(), get.path.as_str(), get.body.clone()), ("GET", "/future-endpoint", None));
        assert_eq!(get.query, [("page".to_string(), "2".to_string())]);

        let post = build_from("request POST future --enabled true", &env).unwrap();
        assert_eq!(post.path, "/future");
        assert_eq!(post.body, Some(json!({"enabled": true})));

        assert_eq!(build_from("request", &env).unwrap_err().0, "Использование: aurum request <METHOD> <PATH> [флаги]");
        assert_eq!(build_from("request G3T /x", &env).unwrap_err().0, "Некорректный HTTP-метод: G3T");
    }
}
