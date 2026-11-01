import { api } from "@/api/client";
import type { CategoryKind, CategoryRankingReport, CategorySpendingReport, ItemSpendingReport } from "@/types";

export function fetchCategorySpendingReport(categoryId: number, startDate?: string, endDate?: string) {
  const params = new URLSearchParams({ category_id: String(categoryId) });
  if (startDate) params.set("start_date", startDate);
  if (endDate) params.set("end_date", endDate);
  return api.get<CategorySpendingReport>(`/reports/category-spending?${params.toString()}`);
}

export function fetchCategoryRanking(kind: CategoryKind, startDate?: string, endDate?: string) {
  const params = new URLSearchParams({ kind });
  if (startDate) params.set("start_date", startDate);
  if (endDate) params.set("end_date", endDate);
  return api.get<CategoryRankingReport>(`/reports/category-ranking?${params.toString()}`);
}

export interface ItemSpendingFilters {
  startDate?: string;
  endDate?: string;
  search?: string;
  accountId?: number;
  limit?: number;
}

export function fetchItemSpending({ startDate, endDate, search, accountId, limit }: ItemSpendingFilters) {
  const params = new URLSearchParams();
  if (startDate) params.set("start_date", startDate);
  if (endDate) params.set("end_date", endDate);
  if (search?.trim()) params.set("search", search.trim());
  if (accountId !== undefined) params.set("account_id", String(accountId));
  if (limit !== undefined) params.set("limit", String(limit));
  const query = params.toString();
  return api.get<ItemSpendingReport>(`/reports/item-spending${query ? `?${query}` : ""}`);
}
