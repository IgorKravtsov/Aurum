import assert from "node:assert/strict";
import { readdir, readFile } from "node:fs/promises";
import { join } from "node:path";
import test from "node:test";

import { COMMANDS } from "../src/commands.js";

test("каждый endpoint бэкенда имеет именованную CLI-команду", async () => {
  const routesDirectory = join(import.meta.dirname, "..", "..", "backend", "app", "api", "routes");
  const files = (await readdir(routesDirectory)).filter((name) => name.endsWith(".py") && name !== "__init__.py");
  const backend = ["GET /health"];
  for (const file of files) {
    const source = await readFile(join(routesDirectory, file), "utf8");
    const prefix = source.match(/router\s*=\s*APIRouter\(prefix="([^"]+)"/)?.[1];
    assert.ok(prefix, `prefix не найден в ${file}`);
    for (const match of source.matchAll(/@router\.(get|post|patch|delete)\("([^"]*)"/g)) {
      backend.push(`${match[1].toUpperCase()} ${prefix}${match[2]}`);
    }
  }
  const cli = COMMANDS.map(({ method, path }) => `${method} ${path}`);
  assert.deepEqual([...new Set(cli)].sort(), [...backend].sort());
  assert.equal(cli.length, new Set(cli).size, "у двух команд не должно быть одного endpoint");
});
