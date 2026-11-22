import { defineConfig } from "@playwright/test";

// Points at the throwaway stack run.sh brings up (see README) — never the
// user's real self-hosted instance, so tests can freely create/delete data.
const baseURL = process.env.AURUM_E2E_BASE_URL ?? "http://localhost:3100";

// The auth-free stack (run.sh) makes the frontend show a blocking "auth not
// configured" modal on first visit, which intercepts every click. The app
// persists the acknowledgement in localStorage (NoAuthModal.tsx), so tests
// start with it already acknowledged. Harmless on the Basic Auth stack.
const noAuthAcknowledged = {
  cookies: [],
  origins: [{ origin: new URL(baseURL).origin, localStorage: [{ name: "aurum:noAuthAcknowledged", value: "1" }] }],
};

// auth-login.spec.ts needs a stack with Basic Auth enabled; run-auth.sh sets
// AURUM_E2E_AUTH_USER for it. On the auth-free stack from run.sh it can only fail.
const authStackOnly = process.env.AURUM_E2E_AUTH_USER ? [] : ["**/auth-login.spec.ts"];

export default defineConfig({
  testDir: "./tests",
  testIgnore: authStackOnly,
  fullyParallel: false, // tests share one backend/DB and assert on exact totals — no isolation between them
  workers: 1,
  retries: 0,
  reporter: [["list"]],
  use: {
    baseURL,
    storageState: noAuthAcknowledged,
    viewport: { width: 1440, height: 900 },
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
});
