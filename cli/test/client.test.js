import assert from "node:assert/strict";
import { mkdtemp, readFile, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import test from "node:test";

import { parseArguments } from "../src/args.js";
import { buildRequest, executeRequest } from "../src/client.js";
import { findCommand } from "../src/commands.js";

test("buildRequest собирает path, query и JSON-тело из флагов", async () => {
  const command = findCommand(["crypto", "holding-update", "17"]);
  const request = await buildRequest(
    command,
    ["crypto", "holding-update", "17"],
    { network: "Ethereum" },
    { api_base_url: "https://finance.example/api/" },
  );
  assert.equal(request.method, "PATCH");
  assert.equal(request.url.href, "https://finance.example/api/crypto/holdings/17");
  assert.deepEqual(request.body, { network: "Ethereum" });
});

test("--file и прямые флаги объединяются, причём флаги имеют приоритет", async () => {
  const directory = await mkdtemp(join(tmpdir(), "aurum-cli-"));
  const input = join(directory, "account.json");
  await writeFile(input, JSON.stringify({ name: "Old", type: "cash" }));
  const command = findCommand(["accounts", "create"]);
  const request = await buildRequest(command, ["accounts", "create"], { file: input, name: "New" }, {});
  assert.deepEqual(request.body, { name: "New", type: "cash" });
});

test("executeRequest добавляет Basic Auth и возвращает JSON", async () => {
  let captured;
  const fakeFetch = async (url, options) => {
    captured = { url, options };
    return new Response(JSON.stringify({ id: 1 }), {
      status: 201,
      headers: { "Content-Type": "application/json" },
    });
  };
  const result = await executeRequest(
    { method: "POST", url: new URL("https://finance.example/api/accounts"), body: { name: "Cash" } },
    { user: "alice", password: "secret" },
    fakeFetch,
  );
  assert.deepEqual(result, { id: 1 });
  assert.equal(captured.options.headers.Authorization, `Basic ${Buffer.from("alice:secret").toString("base64")}`);
  assert.equal(captured.options.body, '{"name":"Cash"}');
});

test("пароль читается из файла без завершающего перевода строки", async () => {
  const directory = await mkdtemp(join(tmpdir(), "aurum-cli-password-"));
  const passwordFile = join(directory, "password");
  await writeFile(passwordFile, "local-secret\n");
  let authorization;
  await executeRequest(
    { method: "GET", url: new URL("https://finance.example/api/settings") },
    { user: "alice", password_file: passwordFile },
    async (_url, options) => {
      authorization = options.headers.Authorization;
      return new Response("{}", { status: 200 });
    },
  );
  assert.equal(authorization, `Basic ${Buffer.from("alice:local-secret").toString("base64")}`);
  assert.equal(await readFile(passwordFile, "utf8"), "local-secret\n");
});

test("reports item-spending передаёт фильтры как query-параметры", async () => {
  const positionals = ["reports", "item-spending"];
  const command = findCommand(positionals);
  const request = await buildRequest(
    command,
    positionals,
    { start_date: "2026-01-01", end_date: "2026-03-31", search: "milk", account_id: 2, limit: 10 },
    { api_base_url: "https://finance.example/api" },
  );
  assert.equal(request.method, "GET");
  assert.equal(request.url.pathname, "/api/reports/item-spending");
  assert.deepEqual(Object.fromEntries(request.url.searchParams), {
    start_date: "2026-01-01",
    end_date: "2026-03-31",
    search: "milk",
    account_id: "2",
    limit: "10",
  });
  assert.equal(request.body, undefined);
});

test("--items передаёт позиции чека JSON-массивом в теле операции", async () => {
  const argv = [
    "transactions", "create",
    "--account-id", "1", "--type", "expense", "--amount", "7.00",
    "--items", '[{"name":"Milk 2%","quantity":2,"unit":"szt","unit_price":3.5,"amount":7}]',
  ];
  const parsed = parseArguments(argv);
  const request = await buildRequest(findCommand(parsed.positionals), parsed.positionals, parsed.flags, {});
  assert.deepEqual(request.body.items, [{ name: "Milk 2%", quantity: 2, unit: "szt", unit_price: 3.5, amount: 7 }]);
});
