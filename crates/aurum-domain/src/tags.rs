//! API contract types for `/api/tags` — port of the former Python backend's `app/schemas/tag.py`.
//!
//! Reference implementation for the porting conventions (see
//! `crates/aurum-server/PORTING.md`): a read DTO that maps straight from a
//! row, and a request DTO validated through [`FromInput`].

use serde::{Deserialize, Serialize};

use crate::validation::{FromInput, Val, ValidationErrors};

/// `TagCreate` — body of `POST /api/tags`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
pub struct TagCreate {
    pub name: String,
}

impl FromInput for TagCreate {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        // name: str = Field(min_length=1, max_length=50)
        let name = o.required("name", |v| v.str()?.min_len(1)?.max_len(50));
        o.finish()?;
        Ok(Self { name: name.value() })
    }
}

/// `TagRead` — also embedded in `TransactionRead.tags`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "openapi", derive(utoipa::ToSchema))]
#[cfg_attr(feature = "sqlx", derive(sqlx::FromRow))]
pub struct TagRead {
    pub id: i32,
    pub name: String,
}

/// Path parameters of `DELETE /api/tags/{tag_id}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TagPath {
    pub tag_id: i64,
}

impl FromInput for TagPath {
    fn from_input(v: Val<'_>) -> Result<Self, ValidationErrors> {
        let mut o = v.object()?;
        let tag_id = o.required("tag_id", |v| v.int());
        o.finish()?;
        Ok(Self { tag_id: tag_id.value() })
    }
}
