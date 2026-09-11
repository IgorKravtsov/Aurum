import { readdir, readFile, stat } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const packageRoot = join(dirname(fileURLToPath(import.meta.url)), "..");
const skillsRoot = join(packageRoot, "skill-data");

function frontmatterValue(markdown, key) {
  const match = markdown.match(new RegExp(`^${key}:\\s*(.+)$`, "m"));
  return match?.[1]?.trim() ?? "";
}

async function availableSkills() {
  const entries = await readdir(skillsRoot, { withFileTypes: true });
  const skills = [];
  for (const entry of entries) {
    if (!entry.isDirectory()) continue;
    const path = join(skillsRoot, entry.name, "SKILL.md");
    const markdown = await readFile(path, "utf8");
    skills.push({ name: frontmatterValue(markdown, "name") || entry.name, description: frontmatterValue(markdown, "description"), path });
  }
  return skills.sort((left, right) => left.name.localeCompare(right.name));
}

async function referenceFiles(skillPath) {
  const referenceRoot = join(dirname(skillPath), "references");
  try {
    if (!(await stat(referenceRoot)).isDirectory()) return [];
  } catch {
    return [];
  }
  const entries = await readdir(referenceRoot, { withFileTypes: true });
  return entries.filter((entry) => entry.isFile() && entry.name.endsWith(".md")).map((entry) => join(referenceRoot, entry.name)).sort();
}

export async function runSkills(positionals, globals) {
  const subcommand = positionals[1] ?? "list";
  const skills = await availableSkills();
  if (subcommand === "list") {
    const data = skills.map(({ name, description }) => ({ name, description }));
    return globals.json ? { success: true, data } : data.map(({ name, description }) => `${name.padEnd(12)} ${description}`).join("\n");
  }
  if (subcommand === "path") {
    const name = positionals[2];
    if (!name) return skillsRoot;
    const skill = skills.find((candidate) => candidate.name === name);
    if (!skill) throw new Error(`Скилл не найден: ${name}`);
    return dirname(skill.path);
  }
  if (subcommand !== "get") throw new Error(`Неизвестная подкоманда skills: ${subcommand}`);

  const getAll = positionals.includes("--all") || globals.all;
  const full = positionals.includes("--full") || globals.full;
  const requested = getAll ? skills : skills.filter(({ name }) => positionals.slice(2).filter((value) => !value.startsWith("--")).includes(name));
  if (!getAll && requested.length === 0) throw new Error("Использование: aurum skills get <name> [--full]");
  const requestedNames = positionals.slice(2).filter((value) => !value.startsWith("--"));
  const missing = requestedNames.filter((name) => !skills.some((skill) => skill.name === name));
  if (missing.length) throw new Error(`Скилл не найден: ${missing.join(", ")}`);

  const documents = [];
  for (const skill of requested) {
    documents.push(await readFile(skill.path, "utf8"));
    if (full) {
      for (const path of await referenceFiles(skill.path)) {
        documents.push(`\n---\n\n<!-- ${path.slice(packageRoot.length + 1)} -->\n\n${await readFile(path, "utf8")}`);
      }
    }
  }
  const content = documents.join("\n\n");
  return globals.json ? { success: true, data: { content } } : content;
}
