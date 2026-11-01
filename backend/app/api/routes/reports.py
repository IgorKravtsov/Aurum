from datetime import date as date_

from fastapi import APIRouter, Depends, Query
from sqlalchemy.ext.asyncio import AsyncSession

from app.api.deps import get_session
from app.models.enums import CategoryKind
from app.schemas.reports import CategoryRankingReport, CategorySpendingReport, ItemSpendingReport
from app.services.item_spending_service import get_item_spending_report
from app.services.reports_service import get_category_ranking_report, get_category_spending_report

router = APIRouter(prefix="/reports", tags=["reports"])


@router.get("/category-spending", response_model=CategorySpendingReport)
async def read_category_spending_report(
    category_id: int,
    start_date: date_ | None = Query(default=None),
    end_date: date_ | None = Query(default=None),
    session: AsyncSession = Depends(get_session),
) -> CategorySpendingReport:
    return await get_category_spending_report(session, category_id, start_date, end_date)


@router.get("/category-ranking", response_model=CategoryRankingReport)
async def read_category_ranking_report(
    kind: CategoryKind = CategoryKind.EXPENSE,
    start_date: date_ | None = Query(default=None),
    end_date: date_ | None = Query(default=None),
    session: AsyncSession = Depends(get_session),
) -> CategoryRankingReport:
    return await get_category_ranking_report(session, kind, start_date, end_date)


@router.get("/item-spending", response_model=ItemSpendingReport)
async def read_item_spending_report(
    start_date: date_ | None = Query(default=None),
    end_date: date_ | None = Query(default=None),
    search: str | None = Query(default=None, max_length=200),
    account_id: int | None = Query(default=None),
    limit: int = Query(default=50, ge=1, le=500),
    session: AsyncSession = Depends(get_session),
) -> ItemSpendingReport:
    """Spending per product from receipt line items of expense transactions
    (see services/item_spending_service.py)."""
    return await get_item_spending_report(
        session, start_date=start_date, end_date=end_date, search=search, account_id=account_id, limit=limit
    )
