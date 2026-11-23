//! Black-box tests of the `aurum` binary: arguments in, stdout/stderr/exit code out.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::{MockServer, Reply};
use tempfile::TempDir;

struct Sandbox {
    home: TempDir,
}

impl Sandbox {
    fn new() -> Self {
        Self { home: tempfile::tempdir().unwrap() }
    }

    fn path(&self, name: &str) -> std::path::PathBuf {
        self.home.path().join(name)
    }

    /// Runs `aurum` with an otherwise empty environment, so nothing from the
    /// developer's machine (AURUM_* variables, a real config) leaks in.
    fn run(&self, args: &[&str], env: &[(&str, &str)]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_aurum"))
            .args(args)
            .current_dir(self.home.path())
            .env_clear()
            .env("HOME", self.home.path())
            .envs(env.iter().copied())
            .output()
            .expect("run aurum")
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8(bytes.to_vec()).unwrap()
}

fn echo_server() -> MockServer {
    MockServer::start(|request| match request.path_and_query.as_str() {
        "/api/health" => Reply::json(200, r#"{"status":"ok"}"#),
        "/api/accounts" if request.method == "GET" => {
            Reply::json(200, r#"[{"id": 1, "balance": 100.0, "name": "Cash"}]"#)
        }
        "/api/missing" => Reply::json(404, r#"{"detail":"Not found"}"#),
        _ => Reply::json(200, "{}"),
    })
}

#[test]
fn version_and_help() {
    let sandbox = Sandbox::new();
    let version = sandbox.run(&["--version"], &[]);
    assert_eq!(text(&version.stdout), format!("{}\n", env!("CARGO_PKG_VERSION")));
    assert!(version.status.success());

    for args in [&["--help"][..], &[][..]] {
        let help = sandbox.run(args, &[]);
        assert!(help.status.success());
        let stdout = text(&help.stdout);
        assert!(stdout.starts_with(&format!("Aurum CLI {}\n\nИспользование:\n", env!("CARGO_PKG_VERSION"))));
        // Descriptions are aligned in one column, so only the ends of the line are fixed.
        let line = stdout
            .lines()
            .find(|line| line.starts_with("  accounts update <account_id>"))
            .expect("accounts update listed");
        assert!(line.ends_with("  Изменить счёт"));
        assert!(stdout.ends_with("aurum skills get core --full\n"));
    }
}

#[test]
fn successful_request_prints_pretty_json_in_js_number_format() {
    let server = echo_server();
    let sandbox = Sandbox::new();
    let url = server.api_url();

    let output = sandbox.run(&["--api-base-url", &url, "accounts", "list"], &[]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(text(&output.stdout), "[\n  {\n    \"id\": 1,\n    \"balance\": 100,\n    \"name\": \"Cash\"\n  }\n]\n");

    let compact = sandbox.run(&["--url", &url, "accounts", "list", "--compact"], &[]);
    assert_eq!(text(&compact.stdout), "[{\"id\":1,\"balance\":100,\"name\":\"Cash\"}]\n");
}

#[test]
fn api_errors_go_to_stderr_with_exit_code_one() {
    let server = echo_server();
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["request", "GET", "/missing"], &[("AURUM_API_BASE_URL", &server.api_url())]);

    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stdout), "");
    assert_eq!(text(&output.stderr), "Aurum API вернул 404 Not Found\n{\n  \"detail\": \"Not found\"\n}\n");
}

#[test]
fn usage_errors_exit_with_one() {
    let sandbox = Sandbox::new();
    let output = sandbox.run(&["nope", "command"], &[]);
    assert_eq!(output.status.code(), Some(1));
    assert_eq!(text(&output.stderr), "Неизвестная команда: nope command\nЗапустите aurum --help\n");

    let output = sandbox.run(&["accounts", "update"], &[]);
    assert_eq!(text(&output.stderr), "Команда accounts update ожидает: <account_id>\n");
}

#[test]
fn body_flags_reach_the_server_as_json() {
    let server = echo_server();
    let sandbox = Sandbox::new();
    let url = server.api_url();
    let output = sandbox.run(
        &[
            "--api-base-url",
            &url,
            "transactions",
            "create",
            "--account-id",
            "3",
            "--amount",
            "42.50",
            "--tag-ids",
            "[1,2]",
            "--field",
            "note=hi",
            "--no-flag",
        ],
        &[],
    );
    assert!(output.status.success(), "{}", text(&output.stderr));
    let sent = &server.requests()[0];
    assert_eq!((sent.method.as_str(), sent.path_and_query.as_str()), ("POST", "/api/transactions"));
    assert_eq!(sent.body, r#"{"account_id":3,"amount":42.5,"tag_ids":[1,2],"flag":false,"note":"hi"}"#);
}

#[test]
fn output_flag_writes_a_private_file_and_prints_nothing() {
    let server = echo_server();
    let sandbox = Sandbox::new();
    let url = server.api_url();
    let output = sandbox.run(&["--api-base-url", &url, "health", "--output", "out.json"], &[]);

    assert!(output.status.success());
    assert_eq!(text(&output.stdout), "");
    assert_eq!(std::fs::read_to_string(sandbox.path("out.json")).unwrap(), "{\n  \"status\": \"ok\"\n}\n");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(sandbox.path("out.json")).unwrap().permissions().mode() & 0o777, 0o600);
    }
}

#[test]
fn saved_config_is_used_by_later_commands() {
    let server = echo_server();
    let sandbox = Sandbox::new();
    let config = sandbox.path("cfg").join("config.json");
    let env = [("AURUM_CONFIG_FILE", config.to_str().unwrap())];

    let set = sandbox.run(&["config", "set", "api-base-url", &server.api_url()], &env);
    assert!(set.status.success(), "{}", text(&set.stderr));
    assert!(text(&set.stdout).contains("\"source\": \"config\""));
    let get = sandbox.run(&["config", "get", "api-base-url"], &env);
    assert_eq!(text(&get.stdout), format!("{}\n", server.api_url()));

    let health = sandbox.run(&["health", "--compact"], &env);
    assert_eq!(text(&health.stdout), "{\"status\":\"ok\"}\n");

    let path = sandbox.run(&["config", "path"], &env);
    assert_eq!(text(&path.stdout), format!("{}\n", config.display()));
}

#[test]
fn basic_auth_comes_from_the_environment_and_a_password_file() {
    let server = echo_server();
    let sandbox = Sandbox::new();
    std::fs::write(sandbox.path("pw"), "secret\n").unwrap();
    let url = server.api_url();
    let output = sandbox
        .run(&["health"], &[("AURUM_API_BASE_URL", &url), ("AURUM_USER", "alice"), ("AURUM_PASSWORD_FILE", "pw")]);

    assert!(output.status.success(), "{}", text(&output.stderr));
    assert_eq!(server.requests()[0].headers["authorization"], "Basic YWxpY2U6c2VjcmV0");
}

#[test]
fn skills_commands_print_the_embedded_guide() {
    let sandbox = Sandbox::new();
    let list = sandbox.run(&["skills", "list", "--json", "--compact"], &[]);
    let listed: serde_json::Value = serde_json::from_slice(&list.stdout).unwrap();
    assert_eq!(listed["data"][0]["name"], "core");

    let core = text(&sandbox.run(&["skills", "get", "core"], &[]).stdout);
    assert!(core.contains("# Aurum CLI"));
    assert!(core.contains("Обязательное сопоставление перед внесением операций"));
    assert!(core.contains("Ничего не записывайте без явного подтверждения"));
    assert!(!core.contains("# Полный справочник Aurum CLI"));

    let full = text(&sandbox.run(&["skills", "get", "core", "--full"], &[]).stdout);
    assert!(full.contains("# Полный справочник Aurum CLI"));
    assert!(full.contains("<!-- skill-data/core/references/commands.md -->"));

    let json = sandbox.run(&["skills", "get", "core", "--json", "--compact"], &[]);
    let value: serde_json::Value = serde_json::from_slice(&json.stdout).unwrap();
    assert_eq!(value["success"], true);
    assert!(value["data"]["content"].as_str().unwrap().contains("# Aurum CLI"));

    let missing = sandbox.run(&["skills", "get", "nope"], &[]);
    assert_eq!(missing.status.code(), Some(1));
    assert_eq!(text(&missing.stderr), "Использование: aurum skills get <name> [--full]\n");
}

#[test]
fn skills_path_materialises_the_embedded_files() {
    let sandbox = Sandbox::new();
    let cache = sandbox.path("cache");
    let output = sandbox.run(&["skills", "path", "core"], &[("XDG_CACHE_HOME", cache.to_str().unwrap())]);
    assert!(output.status.success(), "{}", text(&output.stderr));
    let directory = text(&output.stdout);
    let directory = Path::new(directory.trim_end());
    assert!(directory.join("SKILL.md").is_file());
    assert!(directory.join("references/commands.md").is_file());
}
