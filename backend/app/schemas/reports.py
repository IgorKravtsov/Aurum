from datetime import date as date_
from decimal import Decimal

from pydantic import BaseModel, Field


class CategoryRankingChildItem(BaseModel):
    """One subcategory's (or the parent's own direct, un-subcategorized)
    share of a CategoryRankingItem's total — see
    services/category_rollup.py's CategoryRollupChildItem."""

    category_id: int
    name: str
    color: str
    icon: str | None
    amount: Decimal


class CategorySpendingPoint(BaseModel):
    year: int
    month: int
    amount: Decimal


class CategorySpendingReport(BaseModel):
    category_id: int
    category_name: str
    category_color: str
    category_icon: str | None
    start_date: date_ | None
    end_date: date_ | None
    total_amount: Decimal
    transaction_count: int
    average_per_month: Decimal
    series: list[CategorySpendingPoint]


class CategoryRankingItem(BaseModel):
    category_id: int
    name: str
    color: str
    icon: str | None
    amount: Decimal
    percent: float
    transaction_count: int
    # Populated only when this category's spend came from more than one
    # distinct category (subcategories, or a mix of itself and its
    # children) — e.g. a receipt split across "Groceries" subcategories.
    children: list[CategoryRankingChildItem] = Field(default_factory=list)


class CategoryRankingReport(BaseModel):
    start_date: date_ | None
    end_date: date_ | None
    total_amount: Decimal
    items: list[CategoryRankingItem]


class ItemSpendingProduct(BaseModel):
    """One product across every receipt it appeared on — receipt line items
    grouped by normalized name (case-insensitive, whitespace-trimmed)."""

    # Spelling from the most recent purchase, so a later-corrected name wins.
    name: str
    total_amount: Decimal
    total_quantity: Decimal
    # Most common unit among the lines, or null when none had one.
    unit: str | None
    # Distinct transactions (receipts) the product appeared on.
    purchase_count: int
    # total_amount / total_quantity — the effective price per unit actually
    # paid, after line discounts. Null when the quantities sum to zero.
    avg_unit_price: Decimal | None
    last_date: date_


class ItemSpendingReport(BaseModel):
    start_date: date_ | None
    end_date: date_ | None
    search: str | None
    account_id: int | None
    # Display currency every amount here is converted into (app settings).
    currency: str
    # Totals cover every matching product, not just the `limit` returned.
    total_amount: Decimal
    product_count: int
    purchase_count: int
    items: list[ItemSpendingProduct]
