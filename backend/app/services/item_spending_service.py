"""Per-product spending from receipt line items (models/transaction.py's
TransactionItem) — "what exactly is the money spent on", one level below the
category reports in reports_service.py.

Lines are grouped by a normalized product name, so "Milk 2%", " milk 2% "
and "MILK  2%" count as the same product. Only expense transactions are
included; every amount is converted into the app's display currency at the
transaction's date through the same FX seam the category reports use.
"""
from collections import Counter, defaultdict
from dataclasses import dataclass, field
from datetime import date as date_
from decimal import Decimal

from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.models.account import Account
from app.models.enums import TransactionType
from app.models.transaction import Transaction, TransactionItem
from app.schemas.reports import ItemSpendingProduct, ItemSpendingReport
from app.services.fx_service import MoneyAmount, convert_amounts
from app.services.settings_service import get_or_create_app_settings


def normalize_item_name(name: str) -> str:
    """Grouping key for a product name: trimmed, inner whitespace collapsed,
    case-insensitive."""
    return " ".join(name.split()).casefold()


@dataclass
class _ProductAccumulator:
    name: str = ""
    name_date: date_ | None = None
    total_amount: Decimal = Decimal("0")
    total_quantity: Decimal = Decimal("0")
    units: Counter = field(default_factory=Counter)
    transaction_ids: set[int] = field(default_factory=set)
    last_date: date_ | None = None

    def add(self, *, name: str, amount: Decimal, quantity: Decimal, unit: str | None, transaction_id: int, day: date_) -> None:
        self.total_amount += amount
        self.total_quantity += quantity
        if unit:
            self.units[unit] += 1
        self.transaction_ids.add(transaction_id)
        if self.name_date is None or day >= self.name_date:
            self.name, self.name_date = " ".join(name.split()), day
        if self.last_date is None or day > self.last_date:
            self.last_date = day

    def to_product(self) -> ItemSpendingProduct:
        avg = (self.total_amount / self.total_quantity).quantize(Decimal("0.01")) if self.total_quantity else None
        return ItemSpendingProduct(
            name=self.name,
            total_amount=self.total_amount,
            total_quantity=self.total_quantity,
            unit=self.units.most_common(1)[0][0] if self.units else None,
            purchase_count=len(self.transaction_ids),
            avg_unit_price=avg,
            last_date=self.last_date,
        )


async def get_item_spending_report(
    session: AsyncSession,
    *,
    start_date: date_ | None,
    end_date: date_ | None,
    search: str | None,
    account_id: int | None,
    limit: int,
) -> ItemSpendingReport:
    settings = await get_or_create_app_settings(session)
    search = search.strip() if search and search.strip() else None

    stmt = (
        select(
            TransactionItem.name,
            TransactionItem.quantity,
            TransactionItem.unit,
            TransactionItem.amount,
            Transaction.id.label("transaction_id"),
            Transaction.date,
            Account.currency,
        )
        .join(Transaction, Transaction.id == TransactionItem.transaction_id)
        .join(Account, Account.id == Transaction.account_id)
        .where(Transaction.type == TransactionType.EXPENSE)
    )
    if start_date is not None:
        stmt = stmt.where(Transaction.date >= start_date)
    if end_date is not None:
        stmt = stmt.where(Transaction.date <= end_date)
    if account_id is not None:
        stmt = stmt.where(Transaction.account_id == account_id)
    if search is not None:
        stmt = stmt.where(TransactionItem.name.ilike(f"%{search}%"))
    rows = (await session.execute(stmt)).all()

    converted = (
        await convert_amounts(
            session,
            [MoneyAmount(index, row.amount, row.currency, row.date) for index, row in enumerate(rows)],
            settings.currency,
        )
        if rows
        else {}
    )

    products: dict[str, _ProductAccumulator] = defaultdict(_ProductAccumulator)
    for index, row in enumerate(rows):
        key = normalize_item_name(row.name)
        if not key:
            continue
        products[key].add(
            name=row.name,
            amount=converted[index],
            quantity=row.quantity,
            unit=row.unit,
            transaction_id=row.transaction_id,
            day=row.date,
        )

    ranked = sorted((acc.to_product() for acc in products.values()), key=lambda p: (-p.total_amount, p.name.casefold()))
    return ItemSpendingReport(
        start_date=start_date,
        end_date=end_date,
        search=search,
        account_id=account_id,
        currency=settings.currency,
        total_amount=sum((p.total_amount for p in ranked), Decimal("0")),
        product_count=len(ranked),
        purchase_count=len({row.transaction_id for row in rows}),
        items=ranked[:limit],
    )
