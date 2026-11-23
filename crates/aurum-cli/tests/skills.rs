//! The discovery skill shipped in `skills/aurum` and its installer script.

use std::path::PathBuf;
use std::process::Command;

fn repository_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..").canonicalize().unwrap()
}

#[test]
fn install_skill_script_installs_the_same_discovery_skill_for_agents_and_claude() {
    let directory = tempfile::tempdir().unwrap();
    let agents = directory.path().join("agents");
    let claude = directory.path().join("claude");
    let result = Command::new("sh")
        .arg(repository_root().join("install-skill.sh"))
        .env("AGENTS_SKILLS_DIR", &agents)
        .env("CLAUDE_SKILLS_DIR", &claude)
        .output()
        .unwrap();
    assert!(result.status.success(), "{}", String::from_utf8_lossy(&result.stderr));

    let agents_skill = std::fs::read_to_string(agents.join("aurum/SKILL.md")).unwrap();
    let claude_skill = std::fs::read_to_string(claude.join("aurum/SKILL.md")).unwrap();
    assert_eq!(agents_skill, claude_skill);
    assert!(agents_skill.contains("aurum skills get core"));
    assert!(agents_skill.contains("сопоставьте с ними каждую входящую запись"));
}
