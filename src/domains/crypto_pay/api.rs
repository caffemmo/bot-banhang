use std::collections::HashMap;
use std::sync::Arc;

use axum::{
    Json, Router,
    extract::{Path, Query, State},
    routing::{get, post},
};
use chrono::Utc;
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::core::responses::{Ack, ApiError, ApiResult, PaginatedResponse, ok};
use crate::domains::crypto_pay::models::CryptoPaymentRequest;
use crate::domains::crypto_pay::repo;
use crate::domains::orders::fulfillment::{PaymentSource, fulfill_paid_order};

#[derive(Debug, Serialize)]
pub struct CryptoPaymentDto {
    pub id: i64,
    pub order_id: Option<String>,
    pub purpose: String,
    pub wallet_topup_id: Option<i64>,
    pub user_id: i64,
    pub chat_id: i64,
    pub method: String,
    pub status: String,
    pub amount_vnd: i64,
    pub rate_vnd_per_usdt: String,
    pub amount_usdt_expected: String,
    pub amount_token_units: String,
    pub memo: String,
    pub address: Option<String>,
    pub binance_prepay_id: Option<String>,
    pub binance_checkout_url: Option<String>,
    pub tx_hash: Option<String>,
    pub confirmations: i64,
    pub failure_reason: Option<String>,
    pub created_at: String,
    pub expires_at: String,
    pub completed_at: Option<String>,
}

impl From<CryptoPaymentRequest> for CryptoPaymentDto {
    fn from(payment: CryptoPaymentRequest) -> Self {
        Self {
            id: payment.id,
            order_id: payment.order_id,
            purpose: payment.purpose,
            wallet_topup_id: payment.wallet_topup_id,
            user_id: payment.user_id,
            chat_id: payment.chat_id,
            method: payment.method.to_string(),
            status: payment.status.to_string(),
            amount_vnd: payment.amount_vnd,
            rate_vnd_per_usdt: payment.rate_vnd_per_usdt.to_string(),
            amount_usdt_expected: payment.amount_usdt_expected.to_string(),
            amount_token_units: payment.amount_token_units,
            memo: payment.memo,
            address: payment.address,
            binance_prepay_id: payment.binance_prepay_id,
            binance_checkout_url: payment.binance_checkout_url,
            tx_hash: payment.tx_hash,
            confirmations: payment.confirmations,
            failure_reason: payment.failure_reason,
            created_at: payment.created_at,
            expires_at: payment.expires_at,
            completed_at: payment.completed_at,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ManualCompletePayload {
    pub payment_ref: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ReasonPayload {
    pub reason: Option<String>,
}

pub async fn list_crypto_payments(
    State(ctx): State<Arc<AppContext>>,
    Query(params): Query<HashMap<String, String>>,
) -> ApiResult<PaginatedResponse<CryptoPaymentDto>> {
    let limit = params
        .get("limit")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(50)
        .clamp(1, 200);
    let offset = params
        .get("offset")
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0)
        .max(0);
    let items = repo::list_crypto_payments_admin(&ctx.pool, limit, offset)
        .await
        .map_err(|err| ApiError::internal(format!("list crypto payments failed: {err}")))?
        .into_iter()
        .map(Into::into)
        .collect();
    let total = repo::count_crypto_payments_admin(&ctx.pool)
        .await
        .map_err(|err| ApiError::internal(format!("count crypto payments failed: {err}")))?;
    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

pub async fn get_crypto_payment(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
) -> ApiResult<CryptoPaymentDto> {
    let payment = repo::find_crypto_payment_by_id(&ctx.pool, id)
        .await
        .map_err(|err| ApiError::internal(format!("get crypto payment failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("crypto payment not found"))?;
    Ok(ok(payment.into()))
}

pub async fn complete_crypto_payment_manual(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
    Json(payload): Json<ManualCompletePayload>,
) -> ApiResult<Ack> {
    let payment = repo::find_crypto_payment_by_id(&ctx.pool, id)
        .await
        .map_err(|err| ApiError::internal(format!("get crypto payment failed: {err}")))?
        .ok_or_else(|| ApiError::not_found("crypto payment not found"))?;
    let payment_ref = payload
        .payment_ref
        .unwrap_or_else(|| format!("admin-crypto-{id}"));
    let mut tx = ctx
        .pool
        .begin()
        .await
        .map_err(|err| ApiError::internal(format!("begin tx failed: {err}")))?;
    let completed = repo::complete_crypto_payment(&mut tx, id, Some(&payment_ref), 0)
        .await
        .map_err(|err| ApiError::internal(format!("complete crypto payment failed: {err}")))?;
    tx.commit()
        .await
        .map_err(|err| ApiError::internal(format!("commit tx failed: {err}")))?;
    if completed && let Some(order_id) = payment.order_id {
        fulfill_paid_order(
            ctx,
            &order_id,
            &payment_ref,
            Utc::now(),
            PaymentSource::AdminManual {
                admin_user_id: None,
            },
        )
        .await
        .map_err(|err| ApiError::internal(format!("fulfill order failed: {err}")))?;
    }
    Ok(ok(Ack { success: completed }))
}

pub async fn fail_crypto_payment(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
    Json(payload): Json<ReasonPayload>,
) -> ApiResult<Ack> {
    let failed = repo::fail_crypto_payment(
        &ctx.pool,
        id,
        payload.reason.as_deref().unwrap_or("failed by admin"),
    )
    .await
    .map_err(|err| ApiError::internal(format!("fail crypto payment failed: {err}")))?;
    Ok(ok(Ack { success: failed }))
}

pub async fn manual_review_crypto_payment(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
    Json(payload): Json<ReasonPayload>,
) -> ApiResult<Ack> {
    let updated = repo::mark_crypto_payment_manual_review(
        &ctx.pool,
        id,
        payload
            .reason
            .as_deref()
            .unwrap_or("manual review by admin"),
    )
    .await
    .map_err(|err| ApiError::internal(format!("manual review crypto payment failed: {err}")))?;
    Ok(ok(Ack { success: updated }))
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/admin/crypto-payments", get(list_crypto_payments))
        .route("/api/admin/crypto-payments/:id", get(get_crypto_payment))
        .route(
            "/api/admin/crypto-payments/:id/complete",
            post(complete_crypto_payment_manual),
        )
        .route(
            "/api/admin/crypto-payments/:id/fail",
            post(fail_crypto_payment),
        )
        .route(
            "/api/admin/crypto-payments/:id/manual-review",
            post(manual_review_crypto_payment),
        )
}
