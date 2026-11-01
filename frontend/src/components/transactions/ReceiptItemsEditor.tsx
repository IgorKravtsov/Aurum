import type { ReactNode } from "react";
import { ChevronDown, ChevronRight, Plus, ReceiptText, X } from "lucide-react";
import { Input, Select } from "@/components/ui/Input";
import { formatCurrencyCents } from "@/lib/format";
import { useTranslation } from "@/lib/i18n";
import { translateCategoryName } from "@/lib/categoryLabels";
import { applyRowEdit, emptyReceiptItemRow, itemsTotalCents, toCents, type ReceiptItemRow } from "@/lib/receiptItems";

interface CategoryOption {
  id: number;
  name: string;
  indented?: boolean;
}

interface ReceiptItemsEditorProps {
  rows: ReceiptItemRow[];
  onChange: (rows: ReceiptItemRow[]) => void;
  expanded: boolean;
  onToggle: () => void;
  /** The transaction's own amount, to show how far the items are from it. */
  transactionAmount: string;
  currency?: string;
  categories: CategoryOption[];
}

/** The transaction form's optional, collapsible "receipt line items"
 * section. Rows stack into a two-column grid on phones and a four-column one
 * from `sm` up; the amount recalculates from quantity × unit price − discount
 * while those are edited, but stays directly editable. Unlike splits, the
 * items don't have to add up to the transaction amount — the footer just
 * shows the difference. */
export function ReceiptItemsEditor({
  rows,
  onChange,
  expanded,
  onToggle,
  transactionAmount,
  currency,
  categories,
}: ReceiptItemsEditorProps) {
  const { t } = useTranslation();
  const filledCount = rows.filter((row) => row.name.trim() || row.amount.trim()).length;
  const totalCents = itemsTotalCents(rows);
  const differenceCents = toCents(transactionAmount) - totalCents;

  function updateRow(key: string, patch: Partial<ReceiptItemRow>) {
    onChange(rows.map((row) => (row.key === key ? applyRowEdit(row, patch) : row)));
  }

  function addRow() {
    onChange([...rows, emptyReceiptItemRow()]);
  }

  function removeRow(key: string) {
    onChange(rows.filter((row) => row.key !== key));
  }

  return (
    <div className="rounded-lg border border-border">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={expanded}
        className="flex w-full items-center gap-2 px-3 py-2 text-left text-xs font-medium text-text-secondary hover:text-text-primary"
      >
        {expanded ? <ChevronDown size={14} /> : <ChevronRight size={14} />}
        <ReceiptText size={14} />
        <span className="flex-1">{t("transactions.items.sectionTitle")}</span>
        {filledCount > 0 && (
          <span className="rounded-full bg-surface-2 px-2 py-0.5 text-[11px] tabular-nums text-text-secondary">
            {filledCount}
          </span>
        )}
      </button>

      {expanded && (
        <div className="space-y-2 border-t border-border p-2">
          <p className="text-xs text-text-muted">{t("transactions.items.hint")}</p>

          {rows.map((row, index) => (
            <div key={row.key} className="space-y-1.5 rounded-lg border border-border bg-surface-1 p-2">
              <div className="flex items-center gap-1.5">
                <Input
                  aria-label={t("transactions.items.nameLabel")}
                  placeholder={`${index + 1}. ${t("transactions.items.nameLabel")}`}
                  maxLength={200}
                  value={row.name}
                  onChange={(event) => updateRow(row.key, { name: event.target.value })}
                />
                <button
                  type="button"
                  aria-label={t("transactions.items.removeRow")}
                  onClick={() => removeRow(row.key)}
                  className="shrink-0 rounded-md p-1.5 text-text-muted hover:bg-surface-2 hover:text-danger"
                >
                  <X size={15} />
                </button>
              </div>

              <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-4">
                <ItemField label={t("transactions.items.quantityLabel")}>
                  <Input
                    type="number"
                    inputMode="decimal"
                    step="0.001"
                    min="0.001"
                    value={row.quantity}
                    onChange={(event) => updateRow(row.key, { quantity: event.target.value })}
                  />
                </ItemField>
                <ItemField label={t("transactions.items.unitLabel")}>
                  <Input
                    maxLength={16}
                    placeholder={t("transactions.items.unitPlaceholder")}
                    value={row.unit}
                    onChange={(event) => updateRow(row.key, { unit: event.target.value })}
                  />
                </ItemField>
                <ItemField label={t("transactions.items.unitPriceLabel")}>
                  <Input
                    type="number"
                    inputMode="decimal"
                    step="0.01"
                    min="0"
                    value={row.unit_price}
                    onChange={(event) => updateRow(row.key, { unit_price: event.target.value })}
                  />
                </ItemField>
                <ItemField label={t("transactions.items.discountLabel")}>
                  <Input
                    type="number"
                    inputMode="decimal"
                    step="0.01"
                    min="0"
                    value={row.discount}
                    onChange={(event) => updateRow(row.key, { discount: event.target.value })}
                  />
                </ItemField>
              </div>

              <div className="grid grid-cols-2 gap-1.5 sm:grid-cols-4">
                <ItemField label={t("transactions.items.amountLabel")}>
                  {/* No min: a deposit-return line is legitimately negative. */}
                  <Input
                    type="number"
                    inputMode="decimal"
                    step="0.01"
                    value={row.amount}
                    onChange={(event) => updateRow(row.key, { amount: event.target.value })}
                  />
                </ItemField>
                <div className="sm:col-span-3">
                  <ItemField label={t("transactions.items.categoryLabel")}>
                    <Select
                      value={row.category_id}
                      onChange={(event) => updateRow(row.key, { category_id: event.target.value })}
                    >
                      <option value="">{t("transactions.form.noCategory")}</option>
                      {categories.map((category) => (
                        <option key={category.id} value={category.id}>
                          {category.indented ? `    ↳ ` : ""}
                          {translateCategoryName(category.name)}
                        </option>
                      ))}
                    </Select>
                  </ItemField>
                </div>
              </div>
            </div>
          ))}

          <div className="flex flex-col gap-1.5 sm:flex-row sm:items-center sm:justify-between">
            <button
              type="button"
              onClick={addRow}
              className="flex items-center gap-1 self-start rounded-md py-1 text-xs text-series-1 hover:underline"
            >
              <Plus size={14} />
              {t("transactions.items.addRow")}
            </button>
            {rows.length > 0 && (
              <p className="text-xs tabular-nums text-text-muted">
                {t("transactions.items.totalLabel", { amount: formatCurrencyCents(totalCents / 100, currency) })}
                {" · "}
                <span className={differenceCents === 0 ? "text-success" : "text-text-secondary"}>
                  {differenceCents === 0
                    ? t("transactions.items.matchesAmount")
                    : t("transactions.items.differenceLabel", {
                        amount: formatCurrencyCents(differenceCents / 100, currency),
                      })}
                </span>
              </p>
            )}
          </div>
        </div>
      )}
    </div>
  );
}

function ItemField({ label, children }: { label: string; children: ReactNode }) {
  return (
    <label className="block min-w-0">
      <span className="mb-0.5 block text-[11px] text-text-muted">{label}</span>
      {children}
    </label>
  );
}
