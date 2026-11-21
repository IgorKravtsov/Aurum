//! API contract types for `/api/dashboard` — port of `backend/app/schemas/dashboard.py`.
//!
//! Not ported yet. Follow `crates/aurum-server/PORTING.md` and use
//! `tags.rs` / `settings.rs` as the reference: response DTOs derive
//! `Serialize` (field order = pydantic field order, inherited fields first),
//! request DTOs implement `validation::FromInput`.
