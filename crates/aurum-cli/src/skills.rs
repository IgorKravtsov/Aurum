//! `aurum skills ...`, a port of `cli/src/skills.js`. The skill texts are
//! embedded into the binary (`skill-data/`), so they always match the
//! installed version and no data files have to be shipped next to it.

use std::path::{Path, PathBuf};

use include_dir::{Dir, include_dir};
use serde_json::{Value, json};

use crate::args::Flags;
use crate::env::Env;
use crate::error::{CliError, Result};
use crate::js::{self, fs};

static SKILL_DATA: Dir<'static> = include_dir!("$CARGO_MANIFEST_DIR/skill-data");

/// Where reference files are labelled inside `skills get --full` output.
const SKILL_DATA_DIR_NAME: &str = "skill-data";

struct Skill {
    /// Directory name inside `skill-data/`.
    directory: &'static str,
    name: String,
    description: String,
    markdown: &'static str,
    /// `(file name, content)` of `references/*.md`, sorted by name.
    references: Vec<(&'static str, &'static str)>,
}

/// Value of `key: ...` in the frontmatter (first matching line), trimmed.
fn frontmatter_value(markdown: &str, key: &str) -> String {
    let prefix = format!("{key}:");
    markdown.lines().find_map(|line| line.strip_prefix(&prefix)).map(|rest| rest.trim().to_string()).unwrap_or_default()
}

fn available_skills() -> Vec<Skill> {
    let mut skills: Vec<Skill> = SKILL_DATA
        .dirs()
        .filter_map(|directory| {
            let markdown = directory.get_file(directory.path().join("SKILL.md"))?.contents_utf8()?;
            let directory_name = directory.path().file_name()?.to_str()?;
            let name = frontmatter_value(markdown, "name");
            let mut references: Vec<(&'static str, &'static str)> = directory
                .get_dir(directory.path().join("references"))
                .map(|references| {
                    references
                        .files()
                        .filter_map(|file| Some((file.path().file_name()?.to_str()?, file.contents_utf8()?)))
                        .filter(|(file_name, _)| file_name.ends_with(".md"))
                        .collect()
                })
                .unwrap_or_default();
            references.sort_unstable_by_key(|(file_name, _)| *file_name);
            Some(Skill {
                directory: directory_name,
                name: if name.is_empty() { directory_name.to_string() } else { name },
                description: frontmatter_value(markdown, "description"),
                markdown,
                references,
            })
        })
        .collect();
    skills.sort_by(|left, right| left.name.cmp(&right.name));
    skills
}

/// Skills only exist inside the binary, so `skills path` writes them to a
/// cache directory on demand and prints that location (for tools that want
/// to read the files from disk).
fn materialize(env: &Env) -> Result<PathBuf> {
    let cache_home = match env.get("XDG_CACHE_HOME").filter(|path| !path.is_empty()) {
        Some(home) => PathBuf::from(home),
        None => std::env::home_dir().unwrap_or_default().join(".cache"),
    };
    let root = cache_home.join("aurum").join(SKILL_DATA_DIR_NAME);
    let failed = |error: std::io::Error, at: &Path| {
        CliError::new(fs::error_message(&error, "write", Some(&at.to_string_lossy())))
    };
    for entry in walk_files(&SKILL_DATA) {
        let target = root.join(entry.path());
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|error| failed(error, parent))?;
        }
        std::fs::write(&target, entry.contents()).map_err(|error| failed(error, &target))?;
    }
    Ok(root)
}

fn walk_files<'a>(directory: &'a Dir<'static>) -> Vec<&'a include_dir::File<'static>> {
    let mut files: Vec<_> = directory.files().collect();
    for child in directory.dirs() {
        files.extend(walk_files(child));
    }
    files
}

/// `globals` is the CLI's globals merged with its flags (flags win).
pub fn run_skills(positionals: &[String], globals: &Flags, env: &Env) -> Result<Value> {
    let subcommand = positionals.get(1).map_or("list", String::as_str);
    let skills = available_skills();
    let flag = |name: &str| globals.get(name).is_some_and(js::truthy);
    let json_output = flag("json");

    match subcommand {
        "list" => {
            if json_output {
                let data: Vec<Value> = skills
                    .iter()
                    .map(|skill| json!({ "name": skill.name, "description": skill.description }))
                    .collect();
                return Ok(json!({ "success": true, "data": data }));
            }
            let lines: Vec<String> =
                skills.iter().map(|skill| format!("{:<12} {}", skill.name, skill.description)).collect();
            Ok(Value::String(lines.join("\n")))
        }
        "path" => {
            let root = match positionals.get(2).filter(|name| !name.is_empty()) {
                None => materialize(env)?,
                Some(name) => {
                    let skill = skills
                        .iter()
                        .find(|candidate| &candidate.name == name)
                        .ok_or_else(|| CliError::new(format!("Скилл не найден: {name}")))?;
                    materialize(env)?.join(skill.directory)
                }
            };
            Ok(Value::String(root.to_string_lossy().into_owned()))
        }
        "get" => {
            let get_all = positionals.iter().any(|value| value == "--all") || flag("all");
            let full = positionals.iter().any(|value| value == "--full") || flag("full");
            let requested_names: Vec<&String> =
                positionals.iter().skip(2).filter(|value| !value.starts_with("--")).collect();
            let requested: Vec<&Skill> = if get_all {
                skills.iter().collect()
            } else {
                skills.iter().filter(|skill| requested_names.contains(&&skill.name)).collect()
            };
            if !get_all && requested.is_empty() {
                return Err(CliError::new("Использование: aurum skills get <name> [--full]"));
            }
            let missing: Vec<&str> = requested_names
                .iter()
                .filter(|name| !skills.iter().any(|skill| &&skill.name == *name))
                .map(|name| name.as_str())
                .collect();
            if !missing.is_empty() {
                return Err(CliError::new(format!("Скилл не найден: {}", missing.join(", "))));
            }

            let mut documents: Vec<String> = Vec::new();
            for skill in requested {
                documents.push(skill.markdown.to_string());
                if full {
                    for (file_name, content) in &skill.references {
                        let label = format!("{SKILL_DATA_DIR_NAME}/{}/references/{file_name}", skill.directory);
                        documents.push(format!("\n---\n\n<!-- {label} -->\n\n{content}"));
                    }
                }
            }
            let content = documents.join("\n\n");
            Ok(if json_output {
                json!({ "success": true, "data": { "content": content } })
            } else {
                Value::String(content)
            })
        }
        other => Err(CliError::new(format!("Неизвестная подкоманда skills: {other}"))),
    }
}
