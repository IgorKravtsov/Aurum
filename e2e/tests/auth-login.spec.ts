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
  const passwordInput = page.locator("#auth-password");
  await passwordInput.fill(password);

  const showPasswordButton = page.getByRole("button", { name: "Показать пароль" });
  await expect(passwordInput).toHaveAttribute("type", "password");
  await showPasswordButton.click();
  await expect(passwordInput).toHaveAttribute("type", "text");
  await page.getByRole("button", { name: "Скрыть пароль" }).click();
  await expect(passwordInput).toHaveAttribute("type", "password");

  const successfulCredentialCheck = page.waitForResponse(
    (response) => response.url().endsWith("/api/auth/check") && response.status() === 204,
  );
  await page.locator('button[type="submit"]').click();
  await successfulCredentialCheck;

  await expect(usernameInput).not.toBeVisible();
  await expect(page.locator("main")).toBeVisible();

  // The action stays at the bottom of both navigation variants: visible in
  // the desktop footer, and reachable from the mobile off-canvas drawer.
  await expect(page.getByRole("button", { name: "Выйти" })).toBeVisible();
  await page.setViewportSize({ width: 390, height: 844 });
  await page.getByRole("button", { name: "Открыть меню" }).click();
  const mobileLogout = page.getByRole("button", { name: "Выйти" });
  await expect(mobileLogout).toBeVisible();
  await mobileLogout.click();

  await expect(usernameInput).toBeVisible();
  await expect(page.locator("main")).not.toBeVisible();
  await expect.poll(() => page.evaluate(() => sessionStorage.getItem("aurum:basicAuth"))).toBeNull();
});
