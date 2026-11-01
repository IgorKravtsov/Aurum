from datetime import date as date_
from decimal import Decimal

from pydantic import BaseModel, ConfigDict, Field, computed_field, field_validator, model_validator

from app.core.text import capitalize_first_letter
from app.models.enums import TransactionType
from app.schemas.account import AccountRead
from app.schemas.category import CategoryRead
from app.schemas.tag import TagRead


def transfer_rule_violation(
    *,
    type: TransactionType,
    account_id: int,
    transfer_account_id: int | None,
    category_id: int | None,
    destination_amount: Decimal | None,
) -> str | None:
    """The transfer invariants in one place, returning the problem as text or
    None if the combination is valid.

    Both halves of the write path go through this: the create schema below
    (as a pydantic validator) and the PATCH route (see
    routes/transactions.py). Enforcing it on create only used to let an edit
    produce a row that create would have rejected — most damagingly a
    transfer with no transfer_account_id, which TransactionRead itself can't
    serialize, so the row broke every later read of the transactions list.
    """
    if type == TransactionType.TRANSFER:
        if not transfer_account_id:
            return "transfer_account_id is required for transfer transactions"
        if transfer_account_id == account_id:
            return "transfer_account_id must differ from account_id"
        if category_id:
            return "category_id is not valid for transfer transactions"
    elif transfer_account_id:
        return "transfer_account_id is only valid for transfer transactions"
    elif destination_amount is not None:
        return "destination_amount is only valid for transfer transactions"
    return None


def split_rule_violation(
    *,
    type: TransactionType,
    amount: Decimal,
    category_id: int | None,
    split_count: int,
    split_total: Decimal | None,
) -> str | None:
    """The split invariants in one place, same shape as transfer_rule_violation
    above — checked on create, and against the row *as it would look after
    a patch* on update (see routes/transactions.py), so an edit can't leave
    a transaction whose splits no longer add up to its own amount.

    Takes counts/totals rather than the split objects themselves: the
    update path's "splits weren't touched by this patch" case has to check
    the existing ORM rows, whose category_id can be None (the category was
    since deleted) — nothing here needs a live category to check the sum.
    """
    if split_count == 0:
        return None
    if type == TransactionType.TRANSFER:
        return "splits are not valid for transfer transactions"
    if category_id is not None:
        return "category_id must be omitted when splitting a transaction across categories"
    if split_count < 2:
        return "splitting a transaction needs at least 2 categories"
    if split_total != amount:
        return f"split amounts ({split_total}) must add up to the transaction amount ({amount})"
    return None


# Upper bound per transaction — generous for even a long hypermarket
# receipt, while keeping a single request from carrying an unbounded list.
MAX_ITEMS_PER_TRANSACTION = 500


def item_rule_violation(*, type: TransactionType, item_count: int) -> str | None:
    """The receipt line item invariants, same shape as split_rule_violation
    above — checked on create, and on update against the row as it would
    look after the patch (so switching a transaction with items to a
    transfer is refused too, not just sending items with a transfer).

    Deliberately *no* sum check: unlike splits, items describe a purchase
    rather than divide it, and a receipt may be entered only partially.
    TransactionRead.items_total lets clients surface any discrepancy.
    """
    if item_count == 0:
        return None
    if type == TransactionType.TRANSFER:
        return "items are not valid for transfer transactions"
    return None


class TransactionFields(BaseModel):
    """A transaction's shape, without the rules that only make sense while
    writing one. Reading goes through this: rows already in the database can
    stop satisfying a write-time rule through no fault of their own — the
    transfer_account_id FK is ON DELETE SET NULL, so deleting an account
    leaves the transfers that pointed at it with no destination — and
    refusing to serialize such a row would take the whole transactions list
    down with it, leaving no way in the UI to find and delete the row."""

    account_id: int
    category_id: int | None = None
    transfer_account_id: int | None = None
    type: TransactionType
    amount: Decimal = Field(gt=0, max_digits=14, decimal_places=2)
    destination_amount: Decimal | None = Field(default=None, gt=0, max_digits=14, decimal_places=2)
    description: str = Field(min_length=1, max_length=255)
    merchant: str | None = Field(default=None, max_length=150)
    notes: str | None = Field(default=None, max_length=2000)
    date: date_

    # Auto-capitalizes "траты на продукты" -> "Траты на продукты" so mixed
    # casing from quick manual entry doesn't need fixing by hand later.
    @field_validator("description")
    @classmethod
    def _capitalize_description(cls, value: str) -> str:
        return capitalize_first_letter(value)


class TransactionBase(TransactionFields):
    """The write-side shape: the fields plus the invariants a new or edited
    row has to satisfy."""

    @model_validator(mode="after")
    def _validate_type_specific_fields(self) -> "TransactionBase":
        violation = transfer_rule_violation(
            type=self.type,
            account_id=self.account_id,
            transfer_account_id=self.transfer_account_id,
            category_id=self.category_id,
            destination_amount=self.destination_amount,
        )
        if violation:
            raise ValueError(violation)
        return self


class TransactionSplitInput(BaseModel):
    category_id: int
    amount: Decimal = Field(gt=0, max_digits=14, decimal_places=2)
    note: str | None = Field(default=None, max_length=200)


class TransactionItemInput(BaseModel):
    """One receipt line as sent by a client. Ordering on the receipt is the
    order of the list itself — the server assigns `position` from it."""

    name: str = Field(min_length=1, max_length=200)
    quantity: Decimal = Field(default=Decimal("1"), gt=0, max_digits=12, decimal_places=3)
    unit: str | None = Field(default=None, max_length=16)
    unit_price: Decimal | None = Field(default=None, ge=0, max_digits=14, decimal_places=2)
    # Positive amount knocked off this line; already reflected in `amount`.
    discount: Decimal = Field(default=Decimal("0"), ge=0, max_digits=14, decimal_places=2)
    # Final line total actually paid. Zero and negative values are valid —
    # a deposit-return line reduces the receipt total.
    amount: Decimal = Field(max_digits=14, decimal_places=2)
    category_id: int | None = None

    @field_validator("name")
    @classmethod
    def _strip_name(cls, value: str) -> str:
        value = value.strip()
        if not value:
            raise ValueError("item name must not be blank")
        return value

    @field_validator("unit")
    @classmethod
    def _strip_unit(cls, value: str | None) -> str | None:
        if value is None:
            return None
        return value.strip() or None


class TransactionCreate(TransactionBase):
    tag_ids: list[int] = Field(default_factory=list)
    # None/omitted -> a normal single-category transaction, unchanged from
    # before. 2+ entries -> the amount is divided across categories instead
    # of using category_id (which must then be omitted — see
    # split_rule_violation). A single entry isn't accepted: that's just
    # category_id with extra steps.
    splits: list[TransactionSplitInput] | None = None
    # Optional receipt line items (see models/transaction.py's
    # TransactionItem) — independent of splits and of category_id.
    items: list[TransactionItemInput] | None = Field(default=None, max_length=MAX_ITEMS_PER_TRANSACTION)

    @model_validator(mode="after")
    def _validate_items(self) -> "TransactionCreate":
        violation = item_rule_violation(type=self.type, item_count=len(self.items or []))
        if violation:
            raise ValueError(violation)
        return self

    @model_validator(mode="after")
    def _validate_splits(self) -> "TransactionCreate":
        splits = self.splits or []
        violation = split_rule_violation(
            type=self.type,
            amount=self.amount,
            category_id=self.category_id,
            split_count=len(splits),
            split_total=sum((s.amount for s in splits), Decimal("0")) if splits else None,
        )
        if violation:
            raise ValueError(violation)
        return self


class TransactionUpdate(BaseModel):
    account_id: int | None = None
    category_id: int | None = None
    transfer_account_id: int | None = None
    type: TransactionType | None = None
    amount: Decimal | None = Field(default=None, gt=0, max_digits=14, decimal_places=2)
    destination_amount: Decimal | None = Field(default=None, gt=0, max_digits=14, decimal_places=2)
    description: str | None = Field(default=None, min_length=1, max_length=255)
    merchant: str | None = Field(default=None, max_length=150)
    notes: str | None = Field(default=None, max_length=2000)
    date: date_ | None = None
    # Omitted -> tags untouched; sent (even as []) -> replaces the full tag set.
    tag_ids: list[int] | None = None
    # Omitted -> splits untouched; sent (even as []) -> replaces the full
    # split set (send [] together with a category_id to turn a split
    # transaction back into a normal single-category one).
    splits: list[TransactionSplitInput] | None = None
    # Same semantics as splits: omitted -> items untouched; sent (even as
    # []) -> replaces the full set of receipt line items ([] clears them).
    items: list[TransactionItemInput] | None = Field(default=None, max_length=MAX_ITEMS_PER_TRANSACTION)

    @field_validator("description")
    @classmethod
    def _capitalize_description(cls, value: str | None) -> str | None:
        return capitalize_first_letter(value) if value is not None else None


class TransactionSplitRead(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    category_id: int | None
    category: CategoryRead | None = None
    amount: Decimal
    note: str | None


class TransactionItemRead(BaseModel):
    model_config = ConfigDict(from_attributes=True)

    id: int
    name: str
    quantity: Decimal
    unit: str | None
    unit_price: Decimal | None
    discount: Decimal
    amount: Decimal
    category_id: int | None
    category: CategoryRead | None = None
    position: int


class TransactionRead(TransactionFields):
    model_config = ConfigDict(from_attributes=True)

    id: int
    account: AccountRead
    category: CategoryRead | None = None
    tags: list[TagRead] = Field(default_factory=list)
    splits: list[TransactionSplitRead] = Field(default_factory=list)
    items: list[TransactionItemRead] = Field(default_factory=list)

    # Sum of the receipt line amounts, or null when there are none. Not
    # required to equal `amount` (receipts may be entered partially) —
    # clients compare the two to show the difference.
    @computed_field
    @property
    def items_total(self) -> Decimal | None:
        if not self.items:
            return None
        return sum((item.amount for item in self.items), Decimal("0"))


class TransactionPage(BaseModel):
    items: list[TransactionRead]
    total: int
    page: int
    page_size: int


class TransactionBulkCreate(BaseModel):
    """CSV import (see routes/transactions.py's /bulk): the frontend parses
    the file and maps its columns client-side, then sends already-shaped
    rows here. All-or-nothing — same failure semantics as backup restore, so
    a single bad row never leaves a partial import behind."""

    items: list[TransactionCreate] = Field(min_length=1, max_length=5000)


class TransactionBulkCreateResult(BaseModel):
    created: int
