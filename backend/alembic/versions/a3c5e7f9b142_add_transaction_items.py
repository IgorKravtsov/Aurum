"""add transaction items (receipt line items)

Revision ID: a3c5e7f9b142
Revises: e4b9c2d7a105
Create Date: 2026-11-01 12:00:00.000000

"""
from typing import Sequence, Union

from alembic import op
import sqlalchemy as sa


# revision identifiers, used by Alembic.
revision: str = 'a3c5e7f9b142'
down_revision: Union[str, None] = 'e4b9c2d7a105'
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    op.create_table(
        'transaction_items',
        sa.Column('id', sa.Integer(), nullable=False),
        sa.Column('transaction_id', sa.Integer(), nullable=False),
        sa.Column('name', sa.String(length=200), nullable=False),
        sa.Column('quantity', sa.Numeric(precision=12, scale=3), server_default='1', nullable=False),
        sa.Column('unit', sa.String(length=16), nullable=True),
        sa.Column('unit_price', sa.Numeric(precision=14, scale=2), nullable=True),
        sa.Column('discount', sa.Numeric(precision=14, scale=2), server_default='0', nullable=False),
        sa.Column('amount', sa.Numeric(precision=14, scale=2), nullable=False),
        sa.Column('category_id', sa.Integer(), nullable=True),
        sa.Column('position', sa.Integer(), server_default='0', nullable=False),
        sa.ForeignKeyConstraint(['transaction_id'], ['transactions.id'], ondelete='CASCADE'),
        sa.ForeignKeyConstraint(['category_id'], ['categories.id'], ondelete='SET NULL'),
        sa.PrimaryKeyConstraint('id'),
    )
    op.create_index('ix_transaction_items_transaction_id', 'transaction_items', ['transaction_id'])


def downgrade() -> None:
    op.drop_index('ix_transaction_items_transaction_id', table_name='transaction_items')
    op.drop_table('transaction_items')
