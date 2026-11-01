import { test, expect, type Page } from "@playwright/test";
import { getDefaultAccountId } from "./helpers";

// Receipt line items end to end: entering them in the transaction form (with
// the amount auto-calculated from quantity × unit price), seeing the item
// badge in the list, and the per-product report — on a phone-sized viewport
// too, where the form rows stack and the report switches to cards.
// Product names are neutral examples, not real receipts.

async function acknowledgeNoAuth(page: Page) {
  await page.addInitScript(() => localStorage.setItem("aurum:noAuthAcknowledged", "1"));
}

test("receipt items can be entered in the form and viewed from the list", async ({ page }) => {
  await acknowledgeNoAuth(page);
  await page.goto("/transactions");
  await page.getByRole("button", { name: "Добавить", exact: true }).click();

  await page.locator("#amount").fill("12.00");
  const description = `e2e-receipt-items-form-${Date.now()}`;
  await page.locator("#description").fill(description);

  await page.getByRole("button", { name: "Позиции чека", exact: true }).click();
  await page.getByRole("button", { name: "Добавить позицию" }).click();
  const firstName = page.getByRole("textbox", { name: "Название" }).first();
  await firstName.fill("Test yogurt");
  await page.getByLabel("Кол-во").first().fill("3");
  await page.getByLabel("Цена за ед.").first().fill("2.50");
  // quantity × unit price, recalculated while typing.
  await expect(page.getByLabel("Сумма", { exact: true }).first()).toHaveValue("7.50");

  await page.getByRole("button", { name: "Добавить позицию" }).click();
  await page.getByRole("textbox", { name: "Название" }).nth(1).fill("Test bread");
  await page.getByLabel("Сумма", { exact: true }).nth(1).fill("3.00");
  await expect(page.getByText("разница")).toBeVisible();

  await page.getByRole("button", { name: "Сохранить" }).click();

  const row = page.locator("li", { hasText: description });
  const badge = row.getByRole("button", { name: "Позиции чека: 2" });
  await expect(badge).toBeVisible();
  await badge.click();
  const dialog = page.locator("div.max-h-\\[90vh\\]");
  await expect(dialog.getByText("Test yogurt")).toBeVisible();
  await expect(dialog.getByText("Test bread")).toBeVisible();
});

test("products report works on a phone-sized screen", async ({ page, request }) => {
  const accountId = await getDefaultAccountId(request);
  const resp = await request.post("/api/transactions", {
    data: {
      account_id: accountId,
      type: "expense",
      amount: "9.00",
      description: "e2e-receipt-items-report",
      date: new Date().toISOString().slice(0, 10),
      items: [
        { name: "E2E sparkling water", quantity: 2, unit: "szt", unit_price: 1.5, amount: 3.0 },
        { name: "E2E oat flakes", amount: 6.0 },
      ],
    },
  });
  expect(resp.ok()).toBeTruthy();

  await acknowledgeNoAuth(page);
  await page.setViewportSize({ width: 390, height: 844 });
  await page.goto("/reports");
  await page.getByRole("button", { name: "Товары", exact: true }).click();
  await page.getByRole("searchbox", { name: "Поиск товара" }).fill("e2e sparkling");

  const card = page.locator("ul.sm\\:hidden");
  await expect(card.getByText("E2E sparkling water")).toBeVisible();
  await expect(card.getByText("E2E oat flakes")).toHaveCount(0);
  await expect(page.locator("table")).toBeHidden();

  // Nothing on the page may force horizontal scrolling on a phone.
  const overflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth);
  expect(overflow).toBeLessThanOrEqual(0);
});
