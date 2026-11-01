import { useEffect, useState } from "react";
import { Search } from "lucide-react";
import { Card, CardContent, CardHeader, CardTitle } from "@/components/ui/Card";
import { Input } from "@/components/ui/Input";
import { useItemSpending } from "@/hooks/useReports";
import { formatCurrencyCents, formatTransactionDate, trimTrailingZeros } from "@/lib/format";
import { useTranslation } from "@/lib/i18n";
import type { ItemSpendingProduct } from "@/types";

const SEARCH_DEBOUNCE_MS = 300;
const PRODUCT_LIMIT = 100;

interface ItemSpendingCardProps {
  startDate?: string;
  endDate?: string;
}

/** "What exactly is the money spent on" — receipt line items of expense
 * transactions grouped per product over the selected period. A table from
 * `sm` up; stacked cards on phones, where six columns wouldn't fit. */
export function ItemSpendingCard({ startDate, endDate }: ItemSpendingCardProps) {
  const { t } = useTranslation();
  const [searchInput, setSearchInput] = useState("");
  const [search, setSearch] = useState("");

  // Debounced so typing a product name doesn't fire a request per keystroke.
  useEffect(() => {
    const timer = window.setTimeout(() => setSearch(searchInput.trim()), SEARCH_DEBOUNCE_MS);
    return () => window.clearTimeout(timer);
  }, [searchInput]);

  const { data: report, isLoading } = useItemSpending({
    startDate,
    endDate,
    search: search || undefined,
    limit: PRODUCT_LIMIT,
  });
  const currency = report?.currency;

  return (
    <Card>
      <CardHeader className="flex-col items-stretch sm:flex-row sm:items-center">
        <CardTitle>{t("reports.items.title")}</CardTitle>
        <div className="relative sm:w-64">
          <Search size={14} className="pointer-events-none absolute left-3 top-1/2 -translate-y-1/2 text-text-muted" />
          <Input
            type="search"
            aria-label={t("reports.items.searchPlaceholder")}
            placeholder={t("reports.items.searchPlaceholder")}
            className="pl-8"
            value={searchInput}
            onChange={(event) => setSearchInput(event.target.value)}
          />
        </div>
      </CardHeader>
      <CardContent>
        {isLoading ? (
          <p className="py-10 text-center text-sm text-text-muted">{t("common.loading")}</p>
        ) : !report || report.items.length === 0 ? (
          <p className="py-10 text-center text-sm text-text-muted">
            {search ? t("reports.items.noMatches") : t("reports.items.empty")}
          </p>
        ) : (
          <>
            <div className="mb-3 grid grid-cols-3 gap-2 text-center">
              <Summary label={t("reports.items.totalSpent")} value={formatCurrencyCents(report.total_amount, currency)} />
              <Summary label={t("reports.items.productCount")} value={String(report.product_count)} />
              <Summary label={t("reports.items.purchaseCount")} value={String(report.purchase_count)} />
            </div>

            <table className="hidden w-full text-sm sm:table">
              <thead>
                <tr className="border-b border-gridline text-left text-xs text-text-muted">
                  <th className="w-2/5 py-2 pr-2 font-medium">{t("reports.items.colName")}</th>
                  <th className="px-2 py-2 text-right font-medium">{t("reports.items.colQuantity")}</th>
                  <th className="px-2 py-2 text-right font-medium">{t("reports.items.colPurchases")}</th>
                  <th className="px-2 py-2 text-right font-medium">{t("reports.items.colAvgPrice")}</th>
                  <th className="px-2 py-2 text-right font-medium">{t("reports.items.colLastDate")}</th>
                  <th className="py-2 pl-2 text-right font-medium">{t("reports.items.colTotal")}</th>
                </tr>
              </thead>
              <tbody className="divide-y divide-gridline">
                {report.items.map((product) => (
                  <tr key={product.name.toLowerCase()}>
                    <td className="max-w-0 truncate py-2 pr-2 text-text-primary" title={product.name}>
                      {product.name}
                    </td>
                    <td className="px-2 py-2 text-right tabular-nums text-text-secondary">{formatQuantity(product)}</td>
                    <td className="px-2 py-2 text-right tabular-nums text-text-secondary">{product.purchase_count}</td>
                    <td className="px-2 py-2 text-right tabular-nums text-text-secondary">
                      {product.avg_unit_price ? formatCurrencyCents(product.avg_unit_price, currency) : "—"}
                    </td>
                    <td className="px-2 py-2 text-right tabular-nums text-text-secondary">
                      {formatTransactionDate(product.last_date, true)}
                    </td>
                    <td className="py-2 pl-2 text-right font-medium tabular-nums text-text-primary">
                      {formatCurrencyCents(product.total_amount, currency)}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>

            <ul className="divide-y divide-gridline sm:hidden">
              {report.items.map((product) => (
                <li key={product.name.toLowerCase()} className="py-2.5">
                  <div className="flex items-start justify-between gap-3">
                    <span className="min-w-0 break-words text-sm text-text-primary">{product.name}</span>
                    <span className="shrink-0 text-sm font-medium tabular-nums text-text-primary">
                      {formatCurrencyCents(product.total_amount, currency)}
                    </span>
                  </div>
                  <p className="mt-0.5 text-xs text-text-muted">
                    {formatQuantity(product)}
                    {" · "}
                    {t("reports.items.purchasesShort", { count: product.purchase_count })}
                    {product.avg_unit_price
                      ? ` · ${t("reports.items.avgShort", { amount: formatCurrencyCents(product.avg_unit_price, currency) })}`
                      : ""}
                    {" · "}
                    {formatTransactionDate(product.last_date, true)}
                  </p>
                </li>
              ))}
            </ul>

            {report.product_count > report.items.length && (
              <p className="mt-3 text-center text-xs text-text-muted">
                {t("reports.items.limitedHint", { shown: report.items.length, total: report.product_count })}
              </p>
            )}
          </>
        )}
      </CardContent>
    </Card>
  );
}

function formatQuantity(product: ItemSpendingProduct): string {
  const quantity = trimTrailingZeros(product.total_quantity);
  return product.unit ? `${quantity} ${product.unit}` : quantity;
}

function Summary({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0 rounded-lg bg-surface-2 px-2 py-2">
      <p className="truncate text-[11px] text-text-muted">{label}</p>
      <p className="truncate text-sm font-semibold tabular-nums text-text-primary">{value}</p>
    </div>
  );
}
