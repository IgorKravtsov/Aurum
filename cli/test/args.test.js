import assert from "node:assert/strict";
import test from "node:test";

import { parseArguments, parseScalar } from "../src/args.js";

test("parseScalar распознаёт JSON-типы и сохраняет обычные строки", () => {
  assert.equal(parseScalar("42.50"), 42.5);
  assert.equal(parseScalar("false"), false);
  assert.equal(parseScalar("null"), null);
  assert.deepEqual(parseScalar("[1,2]"), [1, 2]);
  assert.equal(parseScalar("2026-09-12"), "2026-09-12");
});

test("parseArguments отделяет глобальные параметры от полей API", () => {
  const parsed = parseArguments([
    "--api-base-url", "https://example.test/api",
    "transactions", "create",
    "--account-id", "3",
    "--tag-ids", "[1,2]",
    "--no-category-id",
  ]);
  assert.deepEqual(parsed.positionals, ["transactions", "create"]);
  assert.equal(parsed.globals.api_base_url, "https://example.test/api");
  assert.deepEqual(parsed.flags, { account_id: 3, tag_ids: [1, 2], category_id: false });
});

test("--data остаётся строкой до построения тела запроса", () => {
  const parsed = parseArguments(["accounts", "create", "--data", "{\"name\":\"Cash\"}"]);
  assert.equal(parsed.flags.data, "{\"name\":\"Cash\"}");
});
