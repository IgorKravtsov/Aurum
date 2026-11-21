//! Every API endpoint must have a named CLI command (and no two commands
//! may share an endpoint).

use std::collections::BTreeSet;
use std::path::PathBuf;

use aurum_cli::commands::COMMANDS;

/// `(METHOD, path)` pairs declared by a FastAPI router source file:
/// `router = APIRouter(prefix="/x")` plus `@router.get("/y")` decorators.
fn routes_of(source: &str, file: &str) -> Vec<String> {
    let after_prefix =
        source.split("router = APIRouter(prefix=\"").nth(1).unwrap_or_else(|| panic!("prefix не найден в {file}"));
    let prefix = after_prefix.split('"').next().unwrap();
    let mut routes = Vec::new();
    for chunk in source.split("@router.").skip(1) {
        let Some((method, rest)) = chunk.split_once("(\"") else { continue };
        if !["get", "post", "patch", "delete"].contains(&method) {
            continue;
        }
        let path = rest.split('"').next().unwrap();
        routes.push(format!("{} {prefix}{path}", method.to_uppercase()));
    }
    routes
}

#[test]
fn every_backend_endpoint_has_a_named_cli_command() {
    let routes_directory = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../backend/app/api/routes");
    // The FastAPI sources are the reference until the Rust server replaces
    // them; once they are gone this check has nothing left to compare with.
    let Ok(entries) = std::fs::read_dir(&routes_directory) else {
        eprintln!("backend/app/api/routes не найден — проверка покрытия пропущена");
        return;
    };

    let mut backend: BTreeSet<String> = BTreeSet::from(["GET /health".to_string()]);
    for entry in entries {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".py") || name == "__init__.py" {
            continue;
        }
        backend.extend(routes_of(&std::fs::read_to_string(&path).unwrap(), &name));
    }

    let cli: Vec<String> = COMMANDS.iter().map(|command| format!("{} {}", command.method, command.path)).collect();
    let unique: BTreeSet<String> = cli.iter().cloned().collect();
    assert_eq!(unique, backend);
    assert_eq!(cli.len(), unique.len(), "у двух команд не должно быть одного endpoint");
}
