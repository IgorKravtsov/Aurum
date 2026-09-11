import assert from "node:assert/strict";
import { mkdtemp, stat } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { readConfig, resolveApiBaseUrl, runConfig } from "../src/config.js";

test("config set сохраняет api-base-url и следующие запросы используют его", async () => {
  const directory = await mkdtemp(join(tmpdir(), "aurum-config-"));
  const path = join(directory, "config.json");
  const environment = { AURUM_CONFIG_FILE: path };

  const result = await runConfig(["config", "set", "api-base-url", "https://finance.example/api/"], environment);
  assert.equal(result.api_base_url, "https://finance.example/api");
  assert.deepEqual(await readConfig(path), { api_base_url: "https://finance.example/api" });
  assert.equal(await resolveApiBaseUrl({}, environment), "https://finance.example/api");
  assert.equal((await stat(path)).mode & 0o777, 0o600);
});

test("флаг и окружение имеют приоритет над сохранённой конфигурацией", async () => {
  const directory = await mkdtemp(join(tmpdir(), "aurum-config-priority-"));
  const path = join(directory, "config.json");
  await runConfig(["config", "set", "api-base-url", "https://saved.example/api"], { AURUM_CONFIG_FILE: path });

  assert.equal(
    await resolveApiBaseUrl({}, { AURUM_CONFIG_FILE: path, AURUM_API_BASE_URL: "https://environment.example/api" }),
    "https://environment.example/api",
  );
  assert.equal(
    await resolveApiBaseUrl(
      { api_base_url: "https://flag.example/api" },
      { AURUM_CONFIG_FILE: path, AURUM_API_BASE_URL: "https://environment.example/api" },
    ),
    "https://flag.example/api",
  );
});

test("api-base-url не принимает credentials и посторонние протоколы", async () => {
  await assert.rejects(
    runConfig(["config", "set", "api-base-url", "https://user:secret@example.test/api"], { AURUM_CONFIG_FILE: "/unused" }),
    /логин или пароль/,
  );
  await assert.rejects(
    runConfig(["config", "set", "api-base-url", "file:///tmp/api"], { AURUM_CONFIG_FILE: "/unused" }),
    /http:\/\/ или https:\/\//,
  );
});
