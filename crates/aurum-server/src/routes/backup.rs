//! `/api/backup` — port of `backend/app/api/routes/backup.py`.

use aurum_domain::backup::{BackupPayload, ImportResult};
use axum::Json;
use axum::extract::State;
use utoipa_axum::router::OpenApiRouter;
use utoipa_axum::routes;

use crate::audit::log_destructive;
use crate::error::ApiResult;
use crate::extract::ApiJson;
use crate::services;
use crate::state::AppState;

#[utoipa::path(get, path = "/api/backup/export", tag = "backup", responses((status = 200, body = BackupPayload)))]
async fn export_backup(State(state): State<AppState>) -> ApiResult<Json<BackupPayload>> {
    Ok(Json(services::backup::export(&state.pool).await?))
}

#[utoipa::path(
    post,
    path = "/api/backup/import",
    tag = "backup",
    request_body = BackupPayload,
    responses(
        (status = 200, body = ImportResult),
        (status = 400, description = "Unsupported version, dangling reference or failed restore"),
        (status = 422, description = "Validation Error")
    )
)]
async fn import_backup(
    State(state): State<AppState>,
    ApiJson(payload): ApiJson<BackupPayload>,
) -> ApiResult<Json<ImportResult>> {
    // Logged before the fact as well as after: if the restore dies partway
    // (it rolls back, but the process could also be killed outright), the
    // "started" line is the only evidence it was ever attempted.
    log_destructive(
        "backup.restore.started",
        &[
            ("exported_at", &payload.exported_at),
            ("source_app_version", &payload.app_version),
            ("accounts", &payload.accounts.len()),
            ("transactions", &payload.transactions.len()),
        ],
    );
    services::backup::restore(&state.pool, &payload).await?;
    log_destructive("backup.restore.completed", &[("transactions", &payload.transactions.len())]);
    Ok(Json(ImportResult::ok()))
}

pub fn router() -> OpenApiRouter<AppState> {
    OpenApiRouter::new().routes(routes!(export_backup)).routes(routes!(import_backup))
}
