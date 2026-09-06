from fastapi import APIRouter, Depends, HTTPException
from sqlalchemy import select
from sqlalchemy.ext.asyncio import AsyncSession

from app.api.deps import get_session
from app.models.asset import Asset
from app.models.budget import Budget
from app.models.goal import Goal
from app.models.settings import AppSettings
from app.models.transaction import Transaction
from app.schemas.settings import AppSettingsRead, AppSettingsUpdate
from app.services.settings_service import get_or_create_app_settings

router = APIRouter(prefix="/settings", tags=["settings"])


# Both routes still return the ORM row as-is: AppSettingsRead.app_version
# isn't a stored column, it defaults to APP_VERSION when FastAPI validates
# the response (see schemas/settings.py).
@router.get("", response_model=AppSettingsRead)
async def read_settings(session: AsyncSession = Depends(get_session)) -> AppSettings:
    return await get_or_create_app_settings(session)


@router.patch("", response_model=AppSettingsRead)
async def update_settings(payload: AppSettingsUpdate, session: AsyncSession = Depends(get_session)) -> AppSettings:
    settings = await get_or_create_app_settings(session)
    if payload.currency is not None and payload.currency != settings.currency:
        # Budgets/goals are denominated in the base currency, while crypto
        # valuations are fetched in it. Re-labelling those stored numbers
        # would silently corrupt their meaning, so changing the base remains
        # safe only until the first dependent row is created.
        dependent_models = (Transaction, Budget, Goal, Asset)
        for model in dependent_models:
            if (await session.execute(select(model.id).limit(1))).scalar_one_or_none() is not None:
                raise HTTPException(
                    status_code=409,
                    detail="Base currency cannot be changed after transactions, budgets, goals, or assets have been created",
                )
    for field, value in payload.model_dump(exclude_unset=True).items():
        setattr(settings, field, value)
    await session.commit()
    await session.refresh(settings)
    return settings
