use std::sync::Arc;

use anyhow::Result;
use axum::{
    Json, Router,
    extract::{Path, State},
    http::HeaderMap,
    routing::{get, put},
};
use serde::{Deserialize, Serialize};
use sqlx::{FromRow, SqlitePool};

use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::products::repo;

#[derive(Debug, Serialize, FromRow)]
pub struct ProductUsageInstructions {
    pub product_id: i64,
    pub content: String,
    pub updated_at: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UsageInstructionsPayload {
    pub content: Option<String>,
}

pub async fn get_usage_instructions(
    pool: &SqlitePool,
    product_id: i64,
) -> Result<Option<String>, sqlx::Error> {
    sqlx::query_scalar::<_, String>(
        r#"SELECT content
        FROM product_usage_instructions
        WHERE product_id = ?"#,
    )
    .bind(product_id)
    .fetch_optional(pool)
    .await
}

async fn get_product_usage_instructions(
    State(ctx): State<Arc<AppContext>>,
    _headers: HeaderMap,
    Path(product_id): Path<i64>,
) -> ApiResult<Option<ProductUsageInstructions>> {
    let usage = sqlx::query_as::<_, ProductUsageInstructions>(
        r#"SELECT product_id, content, updated_at
        FROM product_usage_instructions
        WHERE product_id = ?"#,
    )
    .bind(product_id)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("get usage instructions failed: {e}")))?;

    Ok(ok(usage))
}

async fn save_product_usage_instructions(
    State(ctx): State<Arc<AppContext>>,
    _headers: HeaderMap,
    Path(product_id): Path<i64>,
    Json(payload): Json<UsageInstructionsPayload>,
) -> ApiResult<ProductUsageInstructions> {
    let Some(_) = repo::get_product(&ctx.pool, product_id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    let content = payload
        .content
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::validation("content cannot be empty"))?;

    if content.chars().count() > 4000 {
        return Err(ApiError::validation("content must be <= 4000 chars"));
    }

    sqlx::query(
        r#"INSERT INTO product_usage_instructions (product_id, content, updated_at)
        VALUES (?, ?, datetime('now'))
        ON CONFLICT(product_id) DO UPDATE SET
            content = excluded.content,
            updated_at = datetime('now')"#,
    )
    .bind(product_id)
    .bind(content)
    .execute(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("save usage instructions failed: {e}")))?;

    let usage = sqlx::query_as::<_, ProductUsageInstructions>(
        r#"SELECT product_id, content, updated_at
        FROM product_usage_instructions
        WHERE product_id = ?"#,
    )
    .bind(product_id)
    .fetch_one(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("load usage instructions failed: {e}")))?;

    Ok(ok(usage))
}

async fn delete_product_usage_instructions(
    State(ctx): State<Arc<AppContext>>,
    _headers: HeaderMap,
    Path(product_id): Path<i64>,
) -> ApiResult<Option<ProductUsageInstructions>> {
    let existing = sqlx::query_as::<_, ProductUsageInstructions>(
        r#"SELECT product_id, content, updated_at
        FROM product_usage_instructions
        WHERE product_id = ?"#,
    )
    .bind(product_id)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("load usage instructions failed: {e}")))?;

    sqlx::query("DELETE FROM product_usage_instructions WHERE product_id = ?")
        .bind(product_id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("delete usage instructions failed: {e}")))?;

    Ok(ok(existing))
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route(
        "/api/admin/products/:id/usage-instructions",
        get(get_product_usage_instructions)
            .put(save_product_usage_instructions)
            .delete(delete_product_usage_instructions),
    )
}
