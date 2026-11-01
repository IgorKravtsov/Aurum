import { describe, expect, it } from "vitest";
import {
  applyRowEdit,
  computeLineAmount,
  emptyReceiptItemRow,
  formatCents,
  itemsTotalCents,
  rowFromItem,
  toItemInputs,
  type ReceiptItemRow,
} from "@/lib/receiptItems";

function row(patch: Partial<ReceiptItemRow>): ReceiptItemRow {
  return { ...emptyReceiptItemRow(), ...patch };
}

describe("computeLineAmount", () => {
  it("multiplies quantity by unit price and subtracts the discount", () => {
    expect(computeLineAmount({ quantity: "2", unit_price: "3.50", discount: "" })).toBe("7.00");
    expect(computeLineAmount({ quantity: "1.25", unit_price: "4.00", discount: "0.50" })).toBe("4.50");
  });

  it("rounds weighed quantities to whole cents without float drift", () => {
    expect(computeLineAmount({ quantity: "0.355", unit_price: "12.99", discount: "" })).toBe("4.61");
  });

  it("returns null without a unit price", () => {
    expect(computeLineAmount({ quantity: "2", unit_price: "", discount: "1" })).toBeNull();
  });
});

describe("applyRowEdit", () => {
  it("recalculates the amount when quantity, price or discount change", () => {
    let current = applyRowEdit(row({}), { unit_price: "2.50" });
    expect(current.amount).toBe("2.50");
    current = applyRowEdit(current, { quantity: "3" });
    expect(current.amount).toBe("7.50");
    current = applyRowEdit(current, { discount: "1.00" });
    expect(current.amount).toBe("6.50");
  });

  it("keeps a manually typed amount until a calculation input changes", () => {
    const current = applyRowEdit(row({ unit_price: "2.50", amount: "2.50" }), { amount: "2.00" });
    expect(current.amount).toBe("2.00");
    expect(applyRowEdit(current, { name: "Bread" }).amount).toBe("2.00");
  });
});

describe("toItemInputs", () => {
  it("skips blank rows and maps filled ones to the API shape", () => {
    const result = toItemInputs([
      row({ name: " Milk 2% ", quantity: "2", unit: "szt", unit_price: "3.50", amount: "7.00" }),
      emptyReceiptItemRow(),
      row({ name: "Returnable packaging", amount: "-0.50", category_id: "4" }),
    ]);
    expect(result).toEqual({
      items: [
        { name: "Milk 2%", quantity: "2", unit: "szt", unit_price: "3.50", discount: "0", amount: "7.00", category_id: null },
        { name: "Returnable packaging", quantity: "1", unit: null, unit_price: null, discount: "0", amount: "-0.50", category_id: 4 },
      ],
    });
  });

  it("reports the first problem", () => {
    expect(toItemInputs([row({ amount: "1.00" })])).toEqual({ error: "missingName" });
    expect(toItemInputs([row({ name: "Bread" })])).toEqual({ error: "missingAmount" });
    expect(toItemInputs([row({ name: "Bread", amount: "1", quantity: "0" })])).toEqual({ error: "invalidQuantity" });
    expect(toItemInputs([row({ name: "Bread", amount: "1", discount: "-1" })])).toEqual({ error: "negativeValue" });
  });
});

describe("totals and conversion", () => {
  it("sums row amounts in cents, including negative lines", () => {
    expect(itemsTotalCents([row({ amount: "7.00" }), row({ amount: "4.50" }), row({ amount: "-0.50" })])).toBe(1100);
    expect(formatCents(-50)).toBe("-0.50");
    expect(formatCents(123456)).toBe("1234.56");
  });

  it("builds an editor row from a stored item", () => {
    const edited = rowFromItem({
      id: 9,
      name: "Apples",
      quantity: "1.250",
      unit: "kg",
      unit_price: "4.00",
      discount: "0.00",
      amount: "5.00",
      category_id: null,
      category: null,
      position: 0,
    });
    expect(edited).toMatchObject({ quantity: "1.25", unit: "kg", discount: "", amount: "5.00", category_id: "" });
  });
});
