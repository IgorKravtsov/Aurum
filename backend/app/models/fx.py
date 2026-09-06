"""Locally cached official foreign-exchange rates.

Each row answers one deterministic question: what was one unit of `currency`
worth in PLN on `requested_date`? `effective_date` records the NBP publication
actually used, which can be earlier on weekends and Polish bank holidays.
Financial amounts and descriptions never leave Aurum; only currency codes and
date ranges are sent to NBP.
"""
from datetime import date as date_, datetime
from decimal import Decimal

from sqlalchemy import CheckConstraint, Date, DateTime, Numeric, String, UniqueConstraint, func
from sqlalchemy.orm import Mapped, mapped_column

from app.db.base import Base


class FxRate(Base):
    __tablename__ = "fx_rates"
    __table_args__ = (
        UniqueConstraint("currency", "requested_date", name="uq_fx_rates_currency_requested_date"),
        CheckConstraint("rate_to_pln > 0", name="ck_fx_rates_positive_rate"),
        CheckConstraint("effective_date <= requested_date", name="ck_fx_rates_effective_not_future"),
    )

    id: Mapped[int] = mapped_column(primary_key=True)
    currency: Mapped[str] = mapped_column(String(3), nullable=False)
    requested_date: Mapped[date_] = mapped_column(Date, nullable=False)
    effective_date: Mapped[date_] = mapped_column(Date, nullable=False)
    # PLN received for one unit of `currency`; PLN itself is the implicit
    # identity rate and is never persisted.
    rate_to_pln: Mapped[Decimal] = mapped_column(Numeric(30, 18), nullable=False)
    provider: Mapped[str] = mapped_column(String(32), nullable=False, default="nbp")
    fetched_at: Mapped[datetime] = mapped_column(DateTime(timezone=True), nullable=False, server_default=func.now())
