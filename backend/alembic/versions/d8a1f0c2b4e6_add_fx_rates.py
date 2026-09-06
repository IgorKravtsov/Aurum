"""add cached FX rates

Revision ID: d8a1f0c2b4e6
Revises: c4e8f61a9d23
Create Date: 2026-09-06
"""
from typing import Sequence, Union

import sqlalchemy as sa
from alembic import op

revision: str = "d8a1f0c2b4e6"
down_revision: Union[str, None] = "c4e8f61a9d23"
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.add_column("transactions", sa.Column("destination_amount", sa.Numeric(14, 2), nullable=True))
    op.add_column("recurring_transactions", sa.Column("destination_amount", sa.Numeric(14, 2), nullable=True))
    # Existing transfers were necessarily same-amount transfers under the old
    # schema, so retaining that fact makes the migration lossless.
    op.execute("UPDATE transactions SET destination_amount = amount WHERE type = 'TRANSFER'")
    op.execute("UPDATE recurring_transactions SET destination_amount = amount WHERE type = 'TRANSFER'")

    op.create_table(
        "fx_rates",
        sa.Column("id", sa.Integer(), nullable=False),
        sa.Column("currency", sa.String(length=3), nullable=False),
        sa.Column("requested_date", sa.Date(), nullable=False),
        sa.Column("effective_date", sa.Date(), nullable=False),
        sa.Column("rate_to_pln", sa.Numeric(precision=30, scale=18), nullable=False),
        sa.Column("provider", sa.String(length=32), nullable=False, server_default="nbp"),
        sa.Column("fetched_at", sa.DateTime(timezone=True), nullable=False, server_default=sa.func.now()),
        sa.CheckConstraint("effective_date <= requested_date", name="ck_fx_rates_effective_not_future"),
        sa.CheckConstraint("rate_to_pln > 0", name="ck_fx_rates_positive_rate"),
        sa.PrimaryKeyConstraint("id"),
        sa.UniqueConstraint("currency", "requested_date", name="uq_fx_rates_currency_requested_date"),
    )
    op.create_index("ix_fx_rates_requested_date", "fx_rates", ["requested_date"], unique=False)


def downgrade() -> None:
    op.drop_index("ix_fx_rates_requested_date", table_name="fx_rates")
    op.drop_table("fx_rates")
    op.drop_column("recurring_transactions", "destination_amount")
    op.drop_column("transactions", "destination_amount")
