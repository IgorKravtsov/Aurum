// Pure logic behind the transaction form's "receipt line items" section —
// kept out of the component so the arithmetic and validation can be unit
// tested without rendering anything (see receiptItems.test.ts).
import { trimTrailingZeros } from "@/lib/format";
import { createLocalId } from "@/lib/localId";
import type { TransactionItem, TransactionItemInput } from "@/types";

export interface ReceiptItemRow {
  /** Local React list key only — never sent to the backend. */
  key: string;
  name: string;
  quantity: string;
  unit: string;
  unit_price: string;
  discount: string;
  amount: string;
  category_id: string;
}

export function emptyReceiptItemRow(): ReceiptItemRow {
  return {
    key: createLocalId("item"),
    name: "",
    quantity: "1",
    unit: "",
    unit_price: "",
    discount: "",
    amount: "",
    category_id: "",
  };
}

export function rowFromItem(item: TransactionItem): ReceiptItemRow {
  return {
    key: `item-${item.id}`,
    name: item.name,
    // Strips the server's fixed-scale padding ("2.000" -> "2") so an edited
    // row shows what a person would have typed.
    quantity: trimTrailingZeros(item.quantity),
    unit: item.unit ?? "",
    unit_price: item.unit_price ?? "",
    discount: Number(item.discount) === 0 ? "" : item.discount,
    amount: item.amount,
    category_id: item.category_id ? String(item.category_id) : "",
  };
}

// Integer arithmetic throughout: quantity in thousandths (the backend keeps
// 3 decimals), money in cents — float multiplication of "1.255" * "3.99"
// style values would otherwise drift by fractions of a cent.
function toScaledInteger(value: string, scale: number): number | null {
  const trimmed = value.trim();
  if (trimmed === "") return null;
  const parsed = Number(trimmed);
  return Number.isFinite(parsed) ? Math.round(parsed * scale) : null;
}

export function toCents(value: string): number {
  return toScaledInteger(value, 100) ?? 0;
}

export function formatCents(cents: number): string {
  const sign = cents < 0 ? "-" : "";
  const absolute = Math.abs(cents);
  return `${sign}${Math.floor(absolute / 100)}.${String(absolute % 100).padStart(2, "0")}`;
}

/** quantity * unit_price - discount, as a 2-decimal string — or null when
 * there's no unit price to compute from (the user then types the amount). */
export function computeLineAmount(row: Pick<ReceiptItemRow, "quantity" | "unit_price" | "discount">): string | null {
  const priceCents = toScaledInteger(row.unit_price, 100);
  if (priceCents === null) return null;
  const quantityMilli = toScaledInteger(row.quantity, 1000) ?? 1000;
  const discountCents = toScaledInteger(row.discount, 100) ?? 0;
  return formatCents(Math.round((quantityMilli * priceCents) / 1000) - discountCents);
}

const RECALCULATING_FIELDS = new Set<keyof ReceiptItemRow>(["quantity", "unit_price", "discount"]);

/** Applies one field edit. Editing quantity, unit price or discount
 * recalculates the amount (when a unit price is set); editing the amount
 * itself just stores what was typed, so it stays overridable. */
export function applyRowEdit(row: ReceiptItemRow, patch: Partial<ReceiptItemRow>): ReceiptItemRow {
  const next = { ...row, ...patch };
  const touchesCalculation = Object.keys(patch).some((field) => RECALCULATING_FIELDS.has(field as keyof ReceiptItemRow));
  if (touchesCalculation && !("amount" in patch)) {
    const computed = computeLineAmount(next);
    if (computed !== null) next.amount = computed;
  }
  return next;
}

function isBlankRow(row: ReceiptItemRow): boolean {
  return !row.name.trim() && !row.amount.trim() && !row.unit_price.trim();
}

export function itemsTotalCents(rows: ReceiptItemRow[]): number {
  return rows.reduce((sum, row) => sum + toCents(row.amount), 0);
}

export type ReceiptItemsError = "missingName" | "missingAmount" | "invalidQuantity" | "negativeValue";

/** Converts the editor rows into the API payload, skipping fully blank rows
 * (an untouched trailing row shouldn't block saving). Returns the first
 * problem found instead, so the form can show one translated message. */
export function toItemInputs(rows: ReceiptItemRow[]): { items: TransactionItemInput[] } | { error: ReceiptItemsError } {
  const items: TransactionItemInput[] = [];
  for (const row of rows) {
    if (isBlankRow(row)) continue;
    if (!row.name.trim()) return { error: "missingName" };
    if (!row.amount.trim() || !Number.isFinite(Number(row.amount))) return { error: "missingAmount" };
    const quantity = row.quantity.trim() || "1";
    if (!(Number(quantity) > 0)) return { error: "invalidQuantity" };
    if (Number(row.unit_price || "0") < 0 || Number(row.discount || "0") < 0) return { error: "negativeValue" };
    items.push({
      name: row.name.trim(),
      quantity,
      unit: row.unit.trim() || null,
      unit_price: row.unit_price.trim() || null,
      discount: row.discount.trim() || "0",
      amount: row.amount.trim(),
      category_id: row.category_id ? Number(row.category_id) : null,
    });
  }
  return { items };
}
