"""merge crypto network and FX rate branches

Revision ID: e4b9c2d7a105
Revises: d1a6f4c8b729, d8a1f0c2b4e6
Create Date: 2026-09-13

"""
from typing import Sequence, Union


# revision identifiers, used by Alembic.
revision: str = "e4b9c2d7a105"
down_revision: Union[str, Sequence[str], None] = ("d1a6f4c8b729", "d8a1f0c2b4e6")
branch_labels: Union[str, Sequence[str], None] = None
depends_on: Union[str, Sequence[str], None] = None


def upgrade() -> None:
    # Both parent migrations already contain the schema changes; this revision
    # only joins their histories so startup can safely target a single head.
    pass


def downgrade() -> None:
    # Downgrading a merge revision separates the history back into both parent
    # branches without reverting either branch's schema changes.
    pass
