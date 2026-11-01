import { Dialog } from "@/components/ui/Dialog";
import { formatCurrencyCents, trimTrailingZeros } from "@/lib/format";
import { useTranslation } from "@/lib/i18n";
import { translateCategoryName } from "@/lib/categoryLabels";
import { toCents } from "@/lib/receiptItems";
import type { Transaction } from "@/types";

interface ReceiptItemsDialogProps {
  transaction: Transaction | null;
  onClose: () => void;
}

/** Read-only view of a transaction's receipt line items, opened from the
 * item-count badge in the transactions list. One stacked row per item so it
 * reads the same on a phone as in the desktop dialog. */
export function ReceiptItemsDialog({ transaction, onClose }: ReceiptItemsDialogProps) {
  const { t } = useTranslation();
  const currency = transaction?.account.currency;
  const differenceCents = transaction ? toCents(transaction.amount) - toCents(transaction.items_total ?? "0") : 0;

  return (
    <Dialog
      open={transaction !== null}
      onClose={onClose}
      title={t("transactions.items.dialogTitle", { description: transaction?.description ?? "" })}
    >
      {transaction && (
        <div className="space-y-3">
          <ul className="divide-y divide-gridline">
            {transaction.items.map((item) => {
              const discount = Number(item.discount);
              return (
                <li key={item.id} className="flex items-start justify-between gap-3 py-2">
                  <span className="min-w-0">
                    <span className="block break-words text-sm text-text-primary">{item.name}</span>
                    <span className="block text-xs text-text-muted">
                      {trimTrailingZeros(item.quantity)}
                      {item.unit ? ` ${item.unit}` : ""}
                      {item.unit_price ? ` × ${formatCurrencyCents(item.unit_price, currency)}` : ""}
                      {discount > 0
                        ? ` · ${t("transactions.items.discountShort", { amount: formatCurrencyCents(discount, currency) })}`
                        : ""}
                      {item.category ? ` · ${translateCategoryName(item.category.name)}` : ""}
                    </span>
                  </span>
                  <span className="shrink-0 text-sm font-medium tabular-nums text-text-primary">
                    {formatCurrencyCents(item.amount, currency)}
                  </span>
                </li>
              );
            })}
          </ul>
          <div className="space-y-1 border-t border-border pt-2 text-xs tabular-nums">
            <p className="flex justify-between text-text-secondary">
              <span>{t("transactions.items.itemsTotalShort")}</span>
              <span>{formatCurrencyCents(transaction.items_total ?? "0", currency)}</span>
            </p>
            <p className="flex justify-between text-text-secondary">
              <span>{t("transactions.items.transactionAmountShort")}</span>
              <span>{formatCurrencyCents(transaction.amount, currency)}</span>
            </p>
            <p className={`flex justify-between ${differenceCents === 0 ? "text-success" : "text-text-muted"}`}>
              <span>{t("transactions.items.differenceShort")}</span>
              <span>{formatCurrencyCents(differenceCents / 100, currency)}</span>
            </p>
          </div>
        </div>
      )}
    </Dialog>
  );
}
