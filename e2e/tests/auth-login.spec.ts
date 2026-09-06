import { expect, test } from "@playwright/test";

const username = process.env.AURUM_E2E_AUTH_USER ?? "aurum-e2e";
const password = process.env.AURUM_E2E_AUTH_PASSWORD ?? "auth-e2e-only";

test("branded login screen accepts valid Basic Auth credentials", async ({ page }) => {
  // The auth-required probe must use the challenge-free endpoint: a regular
  // 401 + WWW-Authenticate can make Chrome wait for its native auth dialog
  // instead of resolving fetch() and rendering Aurum's branded login form.
  const authProbePromise = page.waitForResponse((response) => response.url().endsWith("/api/auth/check"));
  await page.goto("/");
  const authProbe = await authProbePromise;
  expect(authProbe.status()).toBe(403);
  expect(authProbe.headers()["www-authenticate"]).toBeUndefined();

  const usernameInput = page.locator("#auth-username");
  await expect(usernameInput).toBeVisible();

  await usernameInput.fill(username);
  await page.locator("#auth-password").fill(password);

  const successfulCredentialCheck = page.waitForResponse(
    (response) => response.url().endsWith("/api/auth/check") && response.status() === 204,
  );
  await page.locator('button[type="submit"]').click();
  await successfulCredentialCheck;

  await expect(usernameInput).not.toBeVisible();
  await expect(page.locator("main")).toBeVisible();
});
