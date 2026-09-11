import { afterEach, describe, expect, it, vi } from "vitest";
import { createLocalId } from "@/lib/localId";

describe("createLocalId", () => {
  afterEach(() => vi.unstubAllGlobals());

  it("creates unique IDs without calling crypto.randomUUID", () => {
    const randomUUID = vi.fn(() => {
      throw new TypeError("crypto.randomUUID is not a function");
    });
    vi.stubGlobal("crypto", { randomUUID });

    const first = createLocalId("split");
    const second = createLocalId("split");

    expect(first).not.toBe(second);
    expect(first).toMatch(/^split-[a-z0-9]+-[a-z0-9]+$/);
    expect(second).toMatch(/^split-[a-z0-9]+-[a-z0-9]+$/);
    expect(randomUUID).not.toHaveBeenCalled();
  });
});
