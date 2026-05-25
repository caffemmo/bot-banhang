use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::{get, post},
};
use serde::Deserialize;

use super::models::{AdjustPayload, AdminWalletUser, ManualTopupPayload, Wallet, WalletDetail};
use super::repo;
use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, PaginatedResponse, ok};

#[derive(Deserialize)]
pub struct ListQuery {
    #[serde(default = "default_limit")]
    pub limit: i64,
    #[serde(default)]
    pub offset: i64,
    pub query: Option<String>,
}
fn default_limit() -> i64 {
    20
}

/// GET /api/admin/wallets
pub async fn list_wallets(
    State(ctx): State<Arc<AppContext>>,
    Query(q): Query<ListQuery>,
) -> ApiResult<PaginatedResponse<AdminWalletUser>> {
    let limit = q.limit.clamp(1, 100);
    let offset = q.offset.max(0);
    let total = repo::count_admin_wallet_users(&ctx.pool, q.query.as_deref())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let items = repo::list_admin_wallet_users(&ctx.pool, limit, offset, q.query.as_deref())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

/// GET /api/admin/wallets/:user_id
pub async fn get_wallet(
    State(ctx): State<Arc<AppContext>>,
    Path(user_id): Path<i64>,
) -> ApiResult<WalletDetail> {
    let wallet = repo::get_or_create_wallet(&ctx.pool, user_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;
    let transactions = repo::list_transactions(&ctx.pool, user_id, 20)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(ok(WalletDetail {
        wallet,
        transactions,
    }))
}

/// POST /api/admin/wallets/:user_id/adjust
pub async fn adjust_wallet(
    State(ctx): State<Arc<AppContext>>,
    Path(user_id): Path<i64>,
    Json(payload): Json<AdjustPayload>,
) -> ApiResult<Wallet> {
    validate_setup_code(&ctx, payload.setup_code.as_deref().unwrap_or(""))?;
    if payload.amount == 0 {
        return Err(ApiError::validation("amount cannot be zero"));
    }
    validate_note(payload.note.as_deref())?;

    repo::admin_adjust_wallet(&ctx.pool, user_id, payload.amount, payload.note.as_deref())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let wallet = repo::get_or_create_wallet(&ctx.pool, user_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(ok(wallet))
}

/// POST /api/admin/wallets/:user_id/topup
pub async fn topup_wallet(
    State(ctx): State<Arc<AppContext>>,
    Path(user_id): Path<i64>,
    Json(payload): Json<ManualTopupPayload>,
) -> ApiResult<Wallet> {
    validate_setup_code(&ctx, &payload.setup_code)?;
    if payload.amount <= 0 {
        return Err(ApiError::validation("amount must be greater than zero"));
    }
    validate_note(payload.note.as_deref())?;

    repo::admin_manual_topup_wallet(&ctx.pool, user_id, payload.amount, payload.note.as_deref())
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    let wallet = repo::get_or_create_wallet(&ctx.pool, user_id)
        .await
        .map_err(|e| ApiError::internal(e.to_string()))?;

    Ok(ok(wallet))
}

fn validate_setup_code(ctx: &AppContext, setup_code: &str) -> Result<(), ApiError> {
    if setup_code.trim() != ctx.config.admin_setup_code {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INVALID_SETUP_CODE",
            "invalid setup code",
        ));
    }
    Ok(())
}

fn validate_note(note: Option<&str>) -> Result<(), ApiError> {
    if let Some(note) = note
        && note.len() > 500
    {
        return Err(ApiError::validation("note must be <= 500 chars"));
    }
    Ok(())
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/admin/wallets", get(list_wallets))
        .route("/api/admin/wallets/:user_id", get(get_wallet))
        .route("/api/admin/wallets/:user_id/topup", post(topup_wallet))
        .route("/api/admin/wallets/:user_id/adjust", post(adjust_wallet))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use serde_json::json;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use std::sync::{Arc, RwLock};
    use teloxide::Bot;
    use tower::ServiceExt;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_config() -> crate::config::Config {
        crate::config::Config {
            telegram_token: "TEST".into(),
            database_url: "sqlite::memory:".into(),
            bank_name: "VCB".into(),
            bank_account: Some("0000".into()),
            bank_account_name: None,
            webhook_secret: "webhook".into(),
            admin_jwt_secret: "test-secret-that-is-long-enough-for-hmac".into(),
            admin_setup_code: "SETUP-123".into(),
            admin_cookie_secure: false,
            base_url: None,
            i18n_dir: "i18n".to_string(),
            port: 0,
            crypto: crate::config::CryptoConfig::default(),
        }
    }

    async fn test_context(pool: SqlitePool) -> Arc<crate::app::AppContext> {
        Arc::new(crate::app::AppContext {
            bot: Bot::new("TEST"),
            pool,
            config: test_config(),
            configs: Arc::new(RwLock::new(std::collections::HashMap::new())),
            texts: Arc::new(RwLock::new(crate::bot::texts::BotTexts::default())),
            plugins: Arc::new(vec![]),
            usdt_rate_cache: Arc::new(tokio::sync::RwLock::new(None)),
        })
    }

    #[tokio::test]
    async fn manual_topup_requires_setup_code() {
        let ctx = test_context(test_pool().await).await;
        let app = router().with_state(ctx);

        let wrong_code = app
            .clone()
            .oneshot(
                Request::post("/api/admin/wallets/42/topup")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "amount": 50000,
                            "setup_code": "WRONG-CODE",
                            "note": "manual test"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(wrong_code.status(), StatusCode::FORBIDDEN);

        let right_code = app
            .oneshot(
                Request::post("/api/admin/wallets/42/topup")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "amount": 50000,
                            "setup_code": "SETUP-123",
                            "note": "manual test"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(right_code.status(), StatusCode::OK);
    }
}
