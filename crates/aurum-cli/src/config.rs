//! Local CLI configuration, a port of `cli/src/config.js`. Config/secrets
//! file IO lives here (and in `transport`), never in `aurum-client`.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use aurum_client::Url;
use serde_json::{Map, Value, json};

use crate::args::Flags;
use crate::env::Env;
use crate::error::{CliError, Result};
use crate::js::{self, fs, json as js_json};

pub const DEFAULT_API_BASE_URL: &str = "http://127.0.0.1:3000/api";

/// POSIX `path.join` + `normalize`: collapses `//`, `.` and `..`, keeps a
/// leading `/`, and never touches the filesystem.
fn join_normalized(parts: &[&str]) -> String {
    let joined = parts.iter().filter(|part| !part.is_empty()).copied().collect::<Vec<_>>().join("/");
    if joined.is_empty() {
        return ".".to_string();
    }
    let absolute = joined.starts_with('/');
    let trailing_slash = joined.ends_with('/');
    let mut stack: Vec<&str> = Vec::new();
    for segment in joined.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if stack.last().is_some_and(|last| *last != "..") {
                    stack.pop();
                } else if !absolute {
                    stack.push("..");
                }
            }
            other => stack.push(other),
        }
    }
    let mut path = stack.join("/");
    if path.is_empty() && !absolute {
        path = ".".to_string();
    }
    if trailing_slash && !path.is_empty() && path != "." {
        path.push('/');
    }
    if absolute { format!("/{path}") } else { path }
}

fn home_dir() -> String {
    std::env::home_dir().map(|path| path.to_string_lossy().into_owned()).unwrap_or_default()
}

/// `AURUM_CONFIG_FILE`, else `${XDG_CONFIG_HOME:-~/.config}/aurum/config.json`.
pub fn config_path(env: &Env) -> String {
    if let Some(explicit) = env.get("AURUM_CONFIG_FILE").filter(|path| !path.is_empty()) {
        return explicit.to_string();
    }
    let config_home = match env.get("XDG_CONFIG_HOME").filter(|path| !path.is_empty()) {
        Some(home) => home.to_string(),
        None => join_normalized(&[&home_dir(), ".config"]),
    };
    join_normalized(&[&config_home, "aurum", "config.json"])
}

/// The saved config; a missing file is an empty config.
pub fn read_config(path: &str) -> Result<Map<String, Value>> {
    let failed = |reason: String| CliError::new(format!("Не удалось прочитать конфигурацию {path}: {reason}"));
    let text = match fs::read_to_string(Path::new(path)) {
        Ok(text) => text,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(Map::new()),
        Err(error) => return Err(failed(fs::read_error_message(&error, path))),
    };
    match js_json::parse(&text) {
        Ok(Value::Object(config)) => Ok(config),
        Ok(_) => Err(failed("корневое значение должно быть JSON-объектом".to_string())),
        Err(reason) => Err(failed(reason)),
    }
}

/// Validates and canonicalises an API base URL: http(s) only, no embedded
/// credentials, no query/fragment, no trailing slashes.
pub fn normalize_api_base_url(value: &str) -> Result<String> {
    let mut url = Url::parse(value).map_err(|_| CliError::new(format!("Некорректный api-base-url: {value}")))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(CliError::new("api-base-url должен использовать http:// или https://"));
    }
    if !url.username().is_empty() || url.password().is_some_and(|password| !password.is_empty()) {
        return Err(CliError::new("Не помещайте логин или пароль в api-base-url; используйте настройки Basic Auth"));
    }
    url.set_fragment(None);
    url.set_query(None);
    Ok(url.as_str().trim_end_matches('/').to_string())
}

/// Atomically replaces the config file (write to a temp file, then rename)
/// with owner-only permissions: the config may sit next to credentials.
pub fn write_config(config: &Map<String, Value>, path: &str) -> Result<()> {
    let target = PathBuf::from(path);
    let io_failure =
        |error: std::io::Error, syscall: &str, at: &str| CliError::new(fs::error_message(&error, syscall, Some(at)));
    if let Some(directory) = target.parent().filter(|directory| !directory.as_os_str().is_empty()) {
        fs::create_private_dir_all(directory)
            .map_err(|error| io_failure(error, "mkdir", &directory.to_string_lossy()))?;
    }
    let temporary = format!("{path}.{}.tmp", std::process::id());
    let content = format!("{}\n", js::stringify(&Value::Object(config.clone()), 2));
    fs::write_private(Path::new(&temporary), content.as_bytes())
        .map_err(|error| io_failure(error, "open", &temporary))?;
    // Node prints both paths of a rename: `rename 'from' -> 'to'`.
    std::fs::rename(&temporary, &target).map_err(|error| {
        CliError::new(fs::error_message(&error, "rename", Some(&format!("{temporary}' -> '{path}"))))
    })?;
    // Applied again after the rename in case the path already existed with other rights.
    fs::set_private_mode(&target).map_err(|error| io_failure(error, "chmod", path))?;
    Ok(())
}

/// Flag, then environment, then saved config, then the local default.
pub fn resolve_api_base_url(globals: &Flags, env: &Env) -> Result<String> {
    let explicit = globals.get("api_base_url").or_else(|| globals.get("url")).cloned().or_else(|| {
        env.get("AURUM_API_BASE_URL").or_else(|| env.get("AURUM_URL")).map(|value| Value::String(value.to_string()))
    });
    if let Some(explicit) = explicit.filter(js::truthy) {
        return normalize_api_base_url(&js::to_js_string(&explicit));
    }
    let config = read_config(&config_path(env))?;
    normalize_api_base_url(&configured_url(&config))
}

/// `config.api_base_url ?? default`, as a string.
fn configured_url(config: &Map<String, Value>) -> String {
    match config.get("api_base_url") {
        None | Some(Value::Null) => DEFAULT_API_BASE_URL.to_string(),
        Some(value) => js::to_js_string(value),
    }
}

const USAGE: &str = "Использование: aurum config <get|set|unset> api-base-url [URL]\n              aurum config path";

/// `aurum config ...`; returns the value to print.
pub fn run_config(positionals: &[String], env: &Env) -> Result<Value> {
    let action = positionals.get(1).map(String::as_str);
    let key = positionals.get(2).map(String::as_str);
    let path = config_path(env);
    if action == Some("path") && key.is_none() {
        return Ok(Value::String(path));
    }
    if !matches!(action, Some("get" | "set" | "unset")) || key != Some("api-base-url") {
        return Err(CliError::new(USAGE));
    }

    let mut config = read_config(&path)?;
    match action {
        Some("get") => {
            if positionals.len() > 3 {
                return Err(CliError::new("Команда config get не принимает значение"));
            }
            Ok(match config.get("api_base_url") {
                None | Some(Value::Null) => Value::String(DEFAULT_API_BASE_URL.to_string()),
                Some(value) => value.clone(),
            })
        }
        Some("unset") => {
            if positionals.len() > 3 {
                return Err(CliError::new("Команда config unset не принимает значение"));
            }
            config.shift_remove("api_base_url");
            write_config(&config, &path)?;
            Ok(json!({ "api_base_url": DEFAULT_API_BASE_URL, "source": "default", "path": path }))
        }
        _ => {
            let value = positionals.get(3).filter(|value| !value.is_empty());
            let Some(value) = value.filter(|_| positionals.len() <= 4) else {
                return Err(CliError::new("Использование: aurum config set api-base-url <URL>"));
            };
            let normalized = normalize_api_base_url(value)?;
            config.insert("api_base_url".to_string(), Value::String(normalized.clone()));
            write_config(&config, &path)?;
            Ok(json!({ "api_base_url": normalized, "source": "config", "path": path }))
        }
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn words(text: &str) -> Vec<String> {
        text.split(' ').map(str::to_string).collect()
    }

    fn env_for(path: &Path) -> Env {
        Env::from_pairs([("AURUM_CONFIG_FILE", path.to_str().unwrap())])
    }

    #[test]
    fn config_set_saves_api_base_url_and_later_requests_use_it() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("nested").join("config.json");
        let env = env_for(&path);

        let result = run_config(&words("config set api-base-url https://finance.example/api/"), &env).unwrap();
        assert_eq!(result["api_base_url"], "https://finance.example/api");
        assert_eq!(result["source"], "config");
        assert_eq!(
            Value::Object(read_config(path.to_str().unwrap()).unwrap()),
            json!({"api_base_url": "https://finance.example/api"})
        );
        assert_eq!(resolve_api_base_url(&Flags::new(), &env).unwrap(), "https://finance.example/api");
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "{\n  \"api_base_url\": \"https://finance.example/api\"\n}\n"
        );

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(std::fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
            assert_eq!(std::fs::metadata(path.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        }
    }

    #[test]
    fn flag_and_environment_take_priority_over_the_saved_config() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let path_text = path.to_str().unwrap();
        run_config(&words("config set api-base-url https://saved.example/api"), &env_for(&path)).unwrap();

        let environment = Env::from_pairs([
            ("AURUM_CONFIG_FILE", path_text),
            ("AURUM_API_BASE_URL", "https://environment.example/api"),
        ]);
        assert_eq!(resolve_api_base_url(&Flags::new(), &environment).unwrap(), "https://environment.example/api");

        let mut flags = Flags::new();
        flags.insert("api_base_url".to_string(), json!("https://flag.example/api"));
        assert_eq!(resolve_api_base_url(&flags, &environment).unwrap(), "https://flag.example/api");

        // `AURUM_URL` is the short alias, used only when the long variable is unset.
        let alias = Env::from_pairs([("AURUM_CONFIG_FILE", path_text), ("AURUM_URL", "https://alias.example/api")]);
        assert_eq!(resolve_api_base_url(&Flags::new(), &alias).unwrap(), "https://alias.example/api");
    }

    #[test]
    fn api_base_url_rejects_credentials_and_foreign_schemes() {
        let env = Env::from_pairs([("AURUM_CONFIG_FILE", "/unused")]);
        let error =
            run_config(&words("config set api-base-url https://user:secret@example.test/api"), &env).unwrap_err();
        assert!(error.0.contains("логин или пароль"));
        let error = run_config(&words("config set api-base-url file:///tmp/api"), &env).unwrap_err();
        assert!(error.0.contains("http:// или https://"));
        assert_eq!(normalize_api_base_url("nope").unwrap_err().0, "Некорректный api-base-url: nope");
        assert_eq!(normalize_api_base_url("http://h:99/x/?q=1#f").unwrap(), "http://h:99/x");
    }

    #[test]
    fn get_unset_and_path_report_the_effective_state() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        let env = env_for(&path);

        assert_eq!(run_config(&words("config path"), &env).unwrap(), json!(path.to_str().unwrap()));
        assert_eq!(run_config(&words("config get api-base-url"), &env).unwrap(), json!(DEFAULT_API_BASE_URL));
        run_config(&words("config set api-base-url https://a.example/api"), &env).unwrap();
        assert_eq!(run_config(&words("config get api-base-url"), &env).unwrap(), json!("https://a.example/api"));
        let unset = run_config(&words("config unset api-base-url"), &env).unwrap();
        assert_eq!(unset["source"], "default");
        assert_eq!(run_config(&words("config get api-base-url"), &env).unwrap(), json!(DEFAULT_API_BASE_URL));

        assert!(run_config(&words("config get api-base-url extra"), &env).is_err());
        assert!(run_config(&words("config set api-base-url"), &env).is_err());
        assert!(run_config(&words("config bogus"), &env).unwrap_err().0.starts_with("Использование: aurum config"));
    }

    #[test]
    fn unreadable_config_is_reported_with_its_path() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("config.json");
        std::fs::write(&path, "[1]").unwrap();
        let error = read_config(path.to_str().unwrap()).unwrap_err();
        assert_eq!(
            error.0,
            format!(
                "Не удалось прочитать конфигурацию {}: корневое значение должно быть JSON-объектом",
                path.display()
            )
        );
        std::fs::write(&path, "{oops").unwrap();
        assert!(read_config(path.to_str().unwrap()).unwrap_err().0.contains("Expected property name or '}'"));
    }

    #[test]
    fn default_path_follows_xdg_and_normalises() {
        let xdg = Env::from_pairs([("XDG_CONFIG_HOME", "/tmp//x/../y/")]);
        assert_eq!(config_path(&xdg), "/tmp/y/aurum/config.json");
        let explicit = Env::from_pairs([("AURUM_CONFIG_FILE", "./relative.json"), ("XDG_CONFIG_HOME", "/ignored")]);
        assert_eq!(config_path(&explicit), "./relative.json");
    }
}
