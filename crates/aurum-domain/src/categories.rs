//! API contract types for `/api/categories` — port of the former Python backend's `app/schemas/category.py`,
//! plus the pure subcategory rules of the former Python backend's `app/api/routes/categories.py`.
//!
//! Follows `crates/aurum-server/PORTING.md`: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.

use serde::{Deserialize, Serialize};

use crate::enums::CategoryKind;
use crate::validation::{FromInput, Patch, Val, ValidationErrors};

/// Hex color drawn from the dataviz skill's validated categorical palette.
pub const COLOR_PATTERN: &str = r"^#[0-9a-fA-F]{6}$";

/// `CategoryCreate` (= `CategoryBase`) — body of `POST /api/categories`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryCreate {
    pub name: String,
    pub kind: CategoryKind,
    pub icon: Option<String>,
    pub color: String,
    pub sort_order: i64,
    // Subcategory parent, one level deep only — the categories route rejects
    // a parent that itself already has a parent.
    pub parent_id: Option<i64>,
}

impl FromInput for CategoryCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(100));
        let kind = o.required("kind", |v| v.enum_());
        let icon = o.optional("icon", |v| v.str());
        let color = o.required("color", |v| v.str()?.pattern(COLOR_PATTERN));
        let sort_order = o.with_default("sort_order", || 0, |v| v.int());
        let parent_id = o.optional("parent_id", |v| v.int());
        o.finish()?;
        Ok(Self {
            name: name.value(),
            kind: kind.value(),
            icon: icon.value(),
            color: color.value(),
            sort_order: sort_order.value(),
            parent_id: parent_id.value(),
        })
    }
}

/// `CategoryUpdate` — partial update (`exclude_unset`).
#[derive(Debug, Clone, PartialEq, Default, Serialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct CategoryUpdate {
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub name: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub icon: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<String>))]
    pub color: Patch<String>,
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub sort_order: Patch<i64>,
    // The route uses exclude_unset=True, so sending parent_id explicitly as
    // null (as opposed to omitting the field) is what turns a subcategory
    // back into a top-level category.
    #[serde(skip_serializing_if = "Patch::is_unset")]
    #[cfg_attr(feature = "openapi", schema(value_type = Option<i64>))]
    pub parent_id: Patch<i64>,
}

impl FromInput for CategoryUpdate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let name = o.patch("name", |v| v.str()?.min_len(1)?.max_len(100));
        let icon = o.patch("icon", |v| v.str());
        let color = o.patch("color", |v| v.str()?.pattern(COLOR_PATTERN));
        let sort_order = o.patch("sort_order", |v| v.int());
        let parent_id = o.patch("parent_id", |v| v.int());
        o.finish()?;
        Ok(Self {
            name: name.value(),
            icon: icon.value(),
            color: color.value(),
            sort_order: sort_order.value(),
            parent_id: parent_id.value(),
        })
    }
}

/// `CategoryRead` — also embedded in `TransactionRead.category` and
/// `TransactionSplitRead.category`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct CategoryRead {
    pub name: String,
    pub kind: CategoryKind,
    // Lucide icon name rendered in the frontend.
    pub icon: Option<String>,
    pub color: String,
    // Fixed slot ordering keeps the donut chart's category order stable across renders.
    pub sort_order: i32,
    pub parent_id: Option<i32>,
    pub id: i32,
    pub is_default: bool,
}

/// Query of `GET /api/categories`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct CategoryListQuery {
    pub kind: Option<CategoryKind>,
}

impl FromInput for CategoryListQuery {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let kind = o.optional("kind", |v| v.enum_());
        o.finish()?;
        Ok(Self { kind: kind.value() })
    }
}

/// Path parameters of `PATCH/DELETE /api/categories/{category_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CategoryPath {
    pub category_id: i64,
}

impl FromInput for CategoryPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let category_id = o.required("category_id", |v| v.int());
        o.finish()?;
        Ok(Self { category_id: category_id.value() })
    }
}

/// Subcategories are one level deep only: a parent must itself be
/// top-level, and must share the child's kind (an expense category can't
/// nest under an income one, or vice versa). Port of `_validate_parent`;
/// `parent` is the looked-up parent row (`None` = not found). Returns the
/// 400 detail of the first rule broken.
pub fn parent_rule_violation(
    parent_id: i64,
    kind: CategoryKind,
    category_id: Option<i64>,
    parent: Option<&CategoryRead>,
) -> Option<&'static str> {
    if Some(parent_id) == category_id {
        return Some("A category cannot be its own parent");
    }
    let Some(parent) = parent else {
        return Some("Parent category not found");
    };
    if parent.parent_id.is_some() {
        return Some("Subcategories can only be one level deep");
    }
    if parent.kind != kind {
        return Some("A subcategory must have the same kind as its parent");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn category(id: i32, kind: CategoryKind, parent_id: Option<i32>) -> CategoryRead {
        CategoryRead {
            name: "x".into(),
            kind,
            icon: None,
            color: "#000000".into(),
            sort_order: 0,
            parent_id,
            id,
            is_default: false,
        }
    }

    #[test]
    fn parent_rules_in_python_order() {
        let top = category(1, CategoryKind::Expense, None);
        let child = category(2, CategoryKind::Expense, Some(1));
        assert_eq!(
            parent_rule_violation(3, CategoryKind::Expense, Some(3), None),
            Some("A category cannot be its own parent")
        );
        assert_eq!(parent_rule_violation(9, CategoryKind::Expense, None, None), Some("Parent category not found"));
        assert_eq!(
            parent_rule_violation(2, CategoryKind::Expense, None, Some(&child)),
            Some("Subcategories can only be one level deep")
        );
        assert_eq!(
            parent_rule_violation(1, CategoryKind::Income, None, Some(&top)),
            Some("A subcategory must have the same kind as its parent")
        );
        assert_eq!(parent_rule_violation(1, CategoryKind::Expense, None, Some(&top)), None);
    }
}
