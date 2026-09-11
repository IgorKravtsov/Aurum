import assert from "node:assert/strict";
import { mkdtemp, readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import test from "node:test";

import { runSkills } from "../src/skills.js";
import { VERSION } from "../src/main.js";

test("версия CLI берётся из package.json", async () => {
  const packageJson = JSON.parse(await readFile(join(import.meta.dirname, "..", "package.json"), "utf8"));
  assert.equal(VERSION, packageJson.version);
});

test("skills get выдаёт руководство установленной версии", async () => {
  const core = await runSkills(["skills", "get", "core"], {});
  assert.match(core, /# Aurum CLI/);
  assert.doesNotMatch(core, /# Полный справочник Aurum CLI/);

  const full = await runSkills(["skills", "get", "core"], { full: true });
  assert.match(full, /# Полный справочник Aurum CLI/);
});

test("install-skill.sh устанавливает одинаковый discovery-скилл для Agents и Claude", async () => {
  const directory = await mkdtemp(join(tmpdir(), "aurum-skill-"));
  const agents = join(directory, "agents");
  const claude = join(directory, "claude");
  const result = spawnSync("sh", [join(import.meta.dirname, "..", "..", "install-skill.sh")], {
    env: { ...process.env, AGENTS_SKILLS_DIR: agents, CLAUDE_SKILLS_DIR: claude },
    encoding: "utf8",
  });
  assert.equal(result.status, 0, result.stderr);
  const agentsSkill = await readFile(join(agents, "aurum", "SKILL.md"), "utf8");
  const claudeSkill = await readFile(join(claude, "aurum", "SKILL.md"), "utf8");
  assert.equal(agentsSkill, claudeSkill);
  assert.match(agentsSkill, /aurum skills get core/);
});
