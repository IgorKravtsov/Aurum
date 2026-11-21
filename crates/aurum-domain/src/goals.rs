//! API contract types for `/api/goals` — port of `backend/app/schemas/goal.py`,
//! plus the pure half of `backend/app/services/goal_service.py`
//! ([`goal_read`]: progress derived from the contribution total).
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use chrono::NaiveDate;
use serde::{Deserialize, Serialize};

use crate::budgets::percent_of;
use crate::decimal::Decimal;
use crate::pyfmt::PyFloat;
use crate::validation::{FromInput, Patch, Val, ValidationErrors};

// ---------------------------------------------------------------------------
// Request DTOs
// ---------------------------------------------------------------------------

/// `GoalCreate` — body of `POST /api/goals`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GoalCreate {
    pub name: String,
    pub target_amount: Decimal,
    pub target_date: Option<NaiveDate>,
}

impl FromInput for GoalCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(150));
        let target_amount = o.required("target_amount", |v| v.decimal()?.gt("0"));
        let target_date = o.optional("target_date", |v| v.date());
        o.finish()?;
        Ok(Self { name: name.value(), target_amount: target_amount.value(), target_date: target_date.value() })
    }
}

/// `GoalUpdate` — partial update (`exclude_unset`). An explicit `null` for
/// a NOT NULL column is accepted here and fails in the database (500), like
/// the Python commit.
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GoalUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub name: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub target_amount: Patch<Decimal>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<NaiveDate>))]
    pub target_date: Patch<NaiveDate>,
}

impl FromInput for GoalUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.patch("name", |v| v.str()?.min_len(1)?.max_len(150));
        let target_amount = o.patch("target_amount", |v| v.decimal()?.gt("0"));
        let target_date = o.patch("target_date", |v| v.date());
        o.finish()?;
        Ok(Self { name: name.value(), target_amount: target_amount.value(), target_date: target_date.value() })
    }
}

/// `GoalContributionCreate` — body of `POST /api/goals/{goal_id}/contributions`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GoalContributionCreate {
    // Negative allowed — a withdrawal from the goal is still a contribution
    // to its running total, just in the other direction. Zero is pointless.
    pub amount: Decimal,
    pub date: NaiveDate,
    pub note: Option<String>,
}

impl FromInput for GoalContributionCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        // Has to be a validator: pydantic has no "not equal" constraint, so
        // the Field(ne=0) this replaces was parsed as an unknown keyword,
        // stored as schema metadata, and never actually checked anything.
        let amount = o.required("amount", |v| {
            v.decimal()?.check(|value| if value.is_zero() { Err("amount must not be zero".into()) } else { Ok(()) })
        });
        let date = o.required("date", |v| v.date());
        let note = o.optional("note", |v| v.str()?.max_len(200));
        o.finish()?;
        Ok(Self { amount: amount.value(), date: date.value(), note: note.value() })
    }
}

/// Path parameters of the `/api/goals/{goal_id}` routes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GoalPath {
    pub goal_id: i64,
}

impl FromInput for GoalPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let goal_id = o.required("goal_id", |v| v.int());
        o.finish()?;
        Ok(Self { goal_id: goal_id.value() })
    }
}

// ---------------------------------------------------------------------------
// Response DTOs
// ---------------------------------------------------------------------------

/// `GoalRead`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct GoalRead {
    pub id: i32,
    pub name: String,
    pub target_amount: Decimal,
    pub target_date: Option<NaiveDate>,
    pub current_amount: Decimal,
    pub remaining: Decimal,
    pub percent: PyFloat,
    pub is_reached: bool,
}

// ---------------------------------------------------------------------------
// Pure logic
// ---------------------------------------------------------------------------

/// `_to_read`: a goal plus its running total — the sum of all logged
/// contributions, computed on read rather than stored, so it's never out of
/// sync with the log.
pub fn goal_read(
    id: i32,
    name: String,
    target_amount: Decimal,
    target_date: Option<NaiveDate>,
    current_amount: Decimal,
) -> GoalRead {
    GoalRead {
        id,
        name,
        remaining: target_amount.sub(&current_amount),
        percent: percent_of(&current_amount, &target_amount),
        is_reached: current_amount >= target_amount,
        target_date,
        target_amount,
        current_amount,
    }
}

/// What `create_goal` answers: the request's own values (the session never
/// reloads them) with no contributions yet.
pub fn new_goal_read(id: i32, payload: GoalCreate) -> GoalRead {
    GoalRead {
        id,
        name: payload.name,
        remaining: payload.target_amount.clone(),
        target_amount: payload.target_amount,
        target_date: payload.target_date,
        current_amount: Decimal::zero(),
        percent: PyFloat(0.0),
        is_reached: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::validation::Source;
    use serde_json::json;

    fn d(s: &str) -> Decimal {
        s.parse().unwrap()
    }

    #[test]
    fn progress() {
        let goal = goal_read(1, "Trip".into(), d("1000.00"), None, d("250.00"));
        assert_eq!(goal.remaining.to_string(), "750.00");
        assert_eq!(serde_json::to_string(&goal.percent).unwrap(), "25.0");
        assert!(!goal.is_reached);
        let reached = goal_read(1, "Trip".into(), d("3.00"), None, d("3.00"));
        assert!(reached.is_reached);
        // A target stored as 0.00 (e.g. "0.001" rounded by the column).
        let zero = goal_read(1, "x".into(), d("0.00"), None, d("0"));
        assert_eq!(serde_json::to_string(&zero.percent).unwrap(), "0.0");
        assert!(zero.is_reached);
    }

    #[test]
    fn zero_contribution_is_rejected() {
        let errors =
            GoalContributionCreate::from_source(&json!({"amount": "0.00", "date": "2026-01-01"}), Source::Body)
                .unwrap_err();
        assert_eq!(errors.0[0].msg, "Value error, amount must not be zero");
    }
}
