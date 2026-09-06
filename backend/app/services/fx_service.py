"""Currency conversion behind one batch interface.

Callers provide native amounts and dates; this module alone knows how to
normalise ISO codes, resolve NBP business-day fallbacks, persist rates and
compute PLN-pivot cross rates. NBP only receives currency codes and dates —
never transaction amounts, account names, descriptions or other user data.
"""
from __future__ import annotations

from dataclasses import dataclass
from datetime import date, datetime, timedelta, timezone
from decimal import Decimal, InvalidOperation
from typing import Hashable, Sequence

import httpx
from fastapi import HTTPException
from sqlalchemy import func, or_, select
from sqlalchemy.dialects.postgresql import insert
from sqlalchemy.ext.asyncio import AsyncSession

from app.models.fx import FxRate

NBP_BASE_URL = "https://api.nbp.pl/api/exchangerates/rates"
NBP_EARLIEST_DATE = date(2002, 1, 2)
MAX_NBP_RANGE_DAYS = 93
# Table B is normally published weekly, so its holiday fallback window must
# be longer than table A's business-day window.
FALLBACK_DAYS = 14
PROVISIONAL_RATE_TTL = timedelta(hours=1)


@dataclass(frozen=True)
class MoneyAmount:
    key: Hashable
    amount: Decimal
    currency: str
    value_date: date


def _normalise_currency(currency: str) -> str:
    value = currency.strip().upper()
    if len(value) != 3 or not value.isalpha():
        raise HTTPException(status_code=422, detail=f"Unsupported currency code: {currency!r}")
    return value


def _date_chunks(requested_dates: set[date]) -> list[list[date]]:
    """Groups dates so each NBP range, including fallback lookback, stays
    within the provider's documented 93-day maximum."""
    chunks: list[list[date]] = []
    for requested in sorted(requested_dates):
        if not chunks or (requested - chunks[-1][0]).days + FALLBACK_DAYS >= MAX_NBP_RANGE_DAYS:
            chunks.append([requested])
        else:
            chunks[-1].append(requested)
    return chunks


async def _fetch_nbp_currency(currency: str, requested_dates: set[date]) -> dict[date, tuple[date, Decimal]]:
    today = date.today()
    future_dates = sorted(requested for requested in requested_dates if requested > today)
    if future_dates:
        raise HTTPException(
            status_code=422,
            detail=f"Cannot resolve a historical FX rate for future date {future_dates[0].isoformat()}",
        )
    resolved: dict[date, tuple[date, Decimal]] = {}

    async with httpx.AsyncClient(timeout=12, headers={"Accept": "application/json", "User-Agent": "Aurum/1.2"}) as client:
        for chunk in _date_chunks(requested_dates):
            first, last = chunk[0], chunk[-1]
            provider_end = min(last, today)
            provider_start = max(NBP_EARLIEST_DATE, min(first, provider_end) - timedelta(days=FALLBACK_DAYS))
            if provider_end < NBP_EARLIEST_DATE:
                raise HTTPException(
                    status_code=422,
                    detail=f"NBP has no {currency} rates before {NBP_EARLIEST_DATE.isoformat()}",
                )
            response: httpx.Response | None = None
            for table in ("a", "b"):
                url = (
                    f"{NBP_BASE_URL}/{table}/{currency.lower()}/"
                    f"{provider_start.isoformat()}/{provider_end.isoformat()}/"
                )
                try:
                    candidate = await client.get(url, params={"format": "json"})
                except httpx.HTTPError as exc:
                    raise HTTPException(status_code=503, detail="FX provider is temporarily unavailable") from exc
                if candidate.status_code != 404:
                    response = candidate
                    break
            if response is None:
                raise HTTPException(status_code=422, detail=f"Currency {currency} is not available from NBP")
            if not response.is_success:
                raise HTTPException(status_code=503, detail=f"FX provider returned HTTP {response.status_code}")

            try:
                publications = sorted(
                    (
                        date.fromisoformat(item["effectiveDate"]),
                        Decimal(str(item["mid"])),
                    )
                    for item in response.json()["rates"]
                )
            except (KeyError, TypeError, ValueError, InvalidOperation) as exc:
                raise HTTPException(status_code=503, detail="FX provider returned an invalid response") from exc

            for requested in chunk:
                lookup_date = min(requested, today)
                available = [item for item in publications if item[0] <= lookup_date]
                if not available:
                    raise HTTPException(
                        status_code=503,
                        detail=f"No {currency} rate available on or before {requested.isoformat()}",
                    )
                effective_date, rate = available[-1]
                if rate <= 0:
                    raise HTTPException(status_code=503, detail="FX provider returned a non-positive rate")
                resolved[requested] = (effective_date, rate)

    return resolved


async def _rates_to_pln(
    session: AsyncSession, currencies_to_dates: dict[str, set[date]]
) -> dict[tuple[str, date], Decimal]:
    currencies = set(currencies_to_dates) - {"PLN"}
    if not currencies:
        return {}
    requested_dates = set().union(*(currencies_to_dates[currency] for currency in currencies))
    earliest = min(requested_dates) - timedelta(days=FALLBACK_DAYS)
    latest = max(requested_dates)
    cached_rows = (
        await session.execute(
            select(FxRate).where(
                FxRate.currency.in_(currencies),
                or_(
                    FxRate.requested_date.in_(requested_dates),
                    FxRate.effective_date.between(earliest, latest),
                ),
            )
        )
    ).scalars().all()

    today = date.today()
    refresh_before = datetime.now(timezone.utc) - PROVISIONAL_RATE_TTL
    rates: dict[tuple[str, date], Decimal] = {}
    for row in cached_rows:
        is_expired_provisional = (
            row.requested_date == today
            and row.effective_date < row.requested_date
            and row.fetched_at < refresh_before
        )
        if not is_expired_provisional:
            rates[(row.currency, row.requested_date)] = row.rate_to_pln

    def cached_fallback(currency: str, requested: date) -> Decimal | None:
        candidates = [
            row
            for row in cached_rows
            if row.currency == currency
            and requested - timedelta(days=FALLBACK_DAYS) <= row.effective_date <= requested
        ]
        if not candidates:
            return None
        return max(candidates, key=lambda row: (row.effective_date, row.fetched_at)).rate_to_pln

    new_rows: list[dict] = []
    for currency in sorted(currencies):
        missing_dates = {
            requested for requested in currencies_to_dates[currency] if (currency, requested) not in rates
        }
        if not missing_dates:
            continue
        try:
            fetched = await _fetch_nbp_currency(currency, missing_dates)
        except HTTPException as exc:
            # A publication already cached for a nearby earlier date is a
            # safe outage fallback. It is intentionally not persisted under
            # the requested date, so today's preliminary value can still be
            # refreshed once NBP is available again.
            if exc.status_code != 503:
                raise
            unresolved: list[date] = []
            for requested in missing_dates:
                fallback = cached_fallback(currency, requested)
                if fallback is None:
                    unresolved.append(requested)
                else:
                    rates[(currency, requested)] = fallback
            if unresolved:
                raise
            continue
        for requested, (effective, rate) in fetched.items():
            rates[(currency, requested)] = rate
            new_rows.append(
                {
                    "currency": currency,
                    "requested_date": requested,
                    "effective_date": effective,
                    "rate_to_pln": rate,
                    "provider": "nbp",
                }
            )

    if new_rows:
        statement = insert(FxRate).values(new_rows)
        statement = statement.on_conflict_do_update(
            constraint="uq_fx_rates_currency_requested_date",
            set_={
                "effective_date": statement.excluded.effective_date,
                "rate_to_pln": statement.excluded.rate_to_pln,
                "provider": statement.excluded.provider,
                "fetched_at": func.now(),
            },
        )
        await session.execute(statement)
        await session.commit()
    return rates


async def convert_amounts(
    session: AsyncSession,
    amounts: Sequence[MoneyAmount],
    target_currency: str,
) -> dict[Hashable, Decimal]:
    """Converts every item atomically into `target_currency`.

    The result is keyed exactly like the input. A missing rate fails the whole
    batch; callers never receive a partially converted financial total.
    """
    target = _normalise_currency(target_currency)
    normalised = [
        MoneyAmount(item.key, Decimal(item.amount), _normalise_currency(item.currency), item.value_date)
        for item in amounts
    ]
    currencies_to_dates: dict[str, set[date]] = {}
    for item in normalised:
        if item.currency != "PLN":
            currencies_to_dates.setdefault(item.currency, set()).add(item.value_date)
        if target != "PLN":
            currencies_to_dates.setdefault(target, set()).add(item.value_date)

    rates = await _rates_to_pln(session, currencies_to_dates)
    converted: dict[Hashable, Decimal] = {}
    for item in normalised:
        if item.currency == target:
            converted[item.key] = item.amount
            continue
        source_to_pln = Decimal("1") if item.currency == "PLN" else rates[(item.currency, item.value_date)]
        target_to_pln = Decimal("1") if target == "PLN" else rates[(target, item.value_date)]
        converted[item.key] = item.amount * source_to_pln / target_to_pln
    return converted
