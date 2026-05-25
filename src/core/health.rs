use std::sync::Arc;

use axum::extract::State;
use serde::Serialize;

use crate::app::AppContext;

use crate::core::responses::{ApiError, ApiResult, ok};

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: String,
    pub db: String,
    pub version: &'static str,
    pub git_sha: Option<&'static str>,
    pub artifact_sha256: Option<&'static str>,
}

pub async fn health(State(ctx): State<Arc<AppContext>>) -> ApiResult<HealthResponse> {
    sqlx::query_scalar::<_, i64>("SELECT 1")
        .fetch_one(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("db error: {e}")))?;

    Ok(ok(health_payload()))
}

fn health_payload() -> HealthResponse {
    HealthResponse {
        status: "up".to_string(),
        db: "ok".to_string(),
        version: env!("CARGO_PKG_VERSION"),
        git_sha: Some(option_env!("BOTBANHANG_GIT_SHA").unwrap_or("unknown")),
        artifact_sha256: Some(option_env!("BOTBANHANG_ARTIFACT_SHA256").unwrap_or("unknown")),
    }
}
use axum::Router;
use axum::routing::get;

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new().route("/api/health", get(health))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn health_payload_includes_build_version() {
        let payload = health_payload();

        assert_eq!(payload.version, env!("CARGO_PKG_VERSION"));
        assert!(payload.git_sha.is_some());
        assert!(payload.artifact_sha256.is_some());
    }
}
