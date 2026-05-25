use std::sync::Arc;

use anyhow::{Result, anyhow};
use sha2::{Digest, Sha256};

use crate::app::AppContext;
use crate::domains::crypto_pay::binance_pay_history::{
    BinancePayHistoryClient, BinancePayTransaction,
};
use crate::domains::crypto_pay::models::CryptoPaymentRequest;
use crate::domains::crypto_pay::repo::{self as crypto_repo, BinancePayTransactionAudit};
use crate::domains::orders::fulfillment::{PaymentSource, fulfill_paid_order};
use crate::domains::wallet::repo as wallet_repo;
use chrono::{DateTime, NaiveDateTime, Utc};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinancePayMatchDecision {
    Match {
        payment_id: i64,
    },
    Ignore {
        reason: String,
    },
    ManualReview {
        payment_id: Option<i64>,
        reason: String,
    },
}

pub async fn run_binance_pay_tick(ctx: Arc<AppContext>) -> Result<()> {
    if !ctx.binance_pay_enabled() {
        return Ok(());
    }
    let client = BinancePayHistoryClient::from_context(&ctx)?;
    let now = Utc::now();
    let start = now - chrono::Duration::minutes(ctx.binance_pay_history_lookback_minutes());
    let txs = client
        .list_transactions(start.timestamp_millis(), now.timestamp_millis(), 100)
        .await?;
    process_binance_pay_transactions(ctx.clone(), &txs).await?;
    crypto_repo::set_worker_state(
        &ctx.pool,
        "binance_pay_history_last_success_at",
        &now.to_rfc3339(),
    )
    .await?;
    crypto_repo::set_worker_state(
        &ctx.pool,
        "binance_pay_history_last_fetch_count",
        &txs.len().to_string(),
    )
    .await?;
    Ok(())
}

pub async fn process_binance_pay_transactions(
    ctx: Arc<AppContext>,
    txs: &[BinancePayTransaction],
) -> Result<()> {
    for tx in txs {
        let provider_ref = provider_ref(tx)?;
        crypto_repo::upsert_binance_pay_transaction(
            &ctx.pool,
            &BinancePayTransactionAudit {
                provider_tx_id: tx.provider_tx_id.clone(),
                provider_order_id: tx.provider_order_id.clone(),
                provider_raw_id: if tx.provider_tx_id.is_none() {
                    Some(provider_ref.clone())
                } else {
                    None
                },
                note: tx.note.clone(),
                amount_usdt: tx.amount.to_string(),
                currency: tx.currency.clone(),
                transaction_time_ms: tx.transaction_time_ms,
                status: tx.status.clone(),
                direction: tx.direction.clone(),
                raw_json: tx.raw_json.clone(),
            },
        )
        .await?;

        let candidates = if let Some(note) = tx.note.as_deref() {
            crypto_repo::list_active_binance_pay_candidates_by_memo(&ctx.pool, note).await?
        } else {
            Vec::new()
        };
        let candidate_id = candidates.first().map(|payment| payment.id);
        let duplicate =
            crypto_repo::provider_tx_already_completed(&ctx.pool, &provider_ref, candidate_id)
                .await?;
        match classify_binance_pay_transaction(
            tx,
            &candidates,
            duplicate,
            Utc::now().timestamp_millis(),
            ctx.binance_pay_match_grace_minutes(),
        ) {
            BinancePayMatchDecision::Match { payment_id } => {
                let payment = candidates
                    .iter()
                    .find(|payment| payment.id == payment_id)
                    .ok_or_else(|| anyhow!("matched Binance Pay payment candidate not found"))?;
                complete_payment(ctx.clone(), payment, &provider_ref, tx).await?;
                crypto_repo::mark_binance_pay_transaction_match(
                    &ctx.pool,
                    &provider_ref,
                    Some(payment_id),
                    "matched",
                    None,
                )
                .await?;
            }
            BinancePayMatchDecision::Ignore { reason } => {
                crypto_repo::mark_binance_pay_transaction_match(
                    &ctx.pool,
                    &provider_ref,
                    None,
                    "ignored",
                    Some(&reason),
                )
                .await?;
            }
            BinancePayMatchDecision::ManualReview { payment_id, reason } => {
                if let Some(payment_id) = payment_id {
                    crypto_repo::mark_crypto_payment_manual_review(&ctx.pool, payment_id, &reason)
                        .await?;
                }
                crypto_repo::mark_binance_pay_transaction_match(
                    &ctx.pool,
                    &provider_ref,
                    payment_id,
                    "manual_review",
                    Some(&reason),
                )
                .await?;
            }
        }
    }
    Ok(())
}

pub fn classify_binance_pay_transaction(
    tx: &BinancePayTransaction,
    candidates: &[CryptoPaymentRequest],
    provider_tx_already_completed: bool,
    _now_ms: i64,
    grace_minutes: i64,
) -> BinancePayMatchDecision {
    let Some(note) = tx.note.as_deref().filter(|v| !v.trim().is_empty()) else {
        return ignore("missing_note");
    };
    let matching = candidates
        .iter()
        .filter(|payment| payment.memo == note)
        .collect::<Vec<_>>();
    if matching.is_empty() {
        return ignore("unknown_note");
    }
    if matching.len() > 1 {
        return manual(None, "duplicate_active_note");
    }
    let payment = matching[0];

    if provider_tx_already_completed {
        return ignore("duplicate_provider_tx");
    }
    if !tx.currency.eq_ignore_ascii_case("USDT") {
        return manual(Some(payment.id), "currency_mismatch");
    }
    if tx.amount != payment.amount_usdt_expected {
        return manual(Some(payment.id), "amount_mismatch");
    }

    if let Some(created_ms) = parse_time_ms(&payment.created_at)
        && tx.transaction_time_ms < created_ms - 5 * 60 * 1000
    {
        return manual(Some(payment.id), "tx_before_request");
    }
    if let Some(expires_ms) = parse_time_ms(&payment.expires_at)
        && tx.transaction_time_ms > expires_ms + grace_minutes * 60 * 1000
    {
        return manual(Some(payment.id), "late_payment");
    }

    BinancePayMatchDecision::Match {
        payment_id: payment.id,
    }
}

fn ignore(reason: &str) -> BinancePayMatchDecision {
    BinancePayMatchDecision::Ignore {
        reason: reason.to_string(),
    }
}

fn manual(payment_id: Option<i64>, reason: &str) -> BinancePayMatchDecision {
    BinancePayMatchDecision::ManualReview {
        payment_id,
        reason: reason.to_string(),
    }
}

fn parse_time_ms(value: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(value)
        .map(|dt| dt.timestamp_millis())
        .or_else(|_| {
            NaiveDateTime::parse_from_str(value, "%Y-%m-%d %H:%M:%S")
                .map(|dt| DateTime::<Utc>::from_naive_utc_and_offset(dt, Utc).timestamp_millis())
        })
        .ok()
}

fn provider_ref(tx: &BinancePayTransaction) -> Result<String> {
    if let Some(id) = tx.provider_tx_id.as_deref().filter(|v| !v.is_empty()) {
        return Ok(id.to_string());
    }
    if let Some(id) = tx.provider_order_id.as_deref().filter(|v| !v.is_empty()) {
        return Ok(id.to_string());
    }
    let raw = serde_json::to_string(&tx.raw_json)?;
    let fingerprint = format!(
        "{}|{}|{}|{}|{}",
        tx.currency,
        tx.amount,
        tx.note.clone().unwrap_or_default(),
        tx.transaction_time_ms,
        raw
    );
    Ok(hex::encode(Sha256::digest(fingerprint.as_bytes())))
}

async fn complete_payment(
    ctx: Arc<AppContext>,
    payment: &CryptoPaymentRequest,
    provider_ref: &str,
    tx_record: &BinancePayTransaction,
) -> Result<()> {
    let mut db_tx = ctx.pool.begin().await?;
    let completed =
        crypto_repo::complete_binance_pay_payment(&mut db_tx, payment.id, provider_ref).await?;
    if completed && payment.purpose == "wallet_topup" {
        wallet_repo::credit_wallet(
            db_tx.as_mut(),
            payment.user_id,
            payment.amount_vnd,
            "topup",
            None,
            payment.wallet_topup_id.or(Some(payment.id)),
            Some("binance_pay"),
        )
        .await?;
    }
    db_tx.commit().await?;

    if completed && let Some(order_id) = &payment.order_id {
        let paid_at = DateTime::<Utc>::from_timestamp_millis(tx_record.transaction_time_ms)
            .unwrap_or_else(Utc::now);
        fulfill_paid_order(
            ctx,
            order_id,
            provider_ref,
            paid_at,
            PaymentSource::BinancePay {
                prepay_id: provider_ref.to_string(),
            },
        )
        .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::domains::crypto_pay::models::{CryptoPaymentMethod, CryptoPaymentStatus};
    use rust_decimal_macros::dec;
    use serde_json::json;

    fn tx(
        note: Option<&str>,
        amount: rust_decimal::Decimal,
        currency: &str,
    ) -> BinancePayTransaction {
        BinancePayTransaction {
            provider_tx_id: Some("tx-1".to_string()),
            provider_order_id: None,
            note: note.map(str::to_string),
            amount,
            currency: currency.to_string(),
            transaction_time_ms: 1_716_000_000_000,
            status: Some("SUCCESS".to_string()),
            direction: Some("PAY".to_string()),
            raw_json: json!({}),
        }
    }

    fn payment(id: i64, memo: &str) -> CryptoPaymentRequest {
        CryptoPaymentRequest {
            id,
            purpose: "wallet_topup".to_string(),
            order_id: None,
            wallet_topup_id: None,
            user_id: 42,
            chat_id: 420,
            method: CryptoPaymentMethod::BinancePay,
            amount_vnd: 26_000,
            rate_vnd_per_usdt: dec!(26000),
            amount_usdt_base: dec!(1),
            amount_usdt_expected: dec!(1),
            amount_token_units: "1000000000000000000".to_string(),
            memo: memo.to_string(),
            address: None,
            binance_prepay_id: None,
            binance_checkout_url: None,
            binance_qrcode_link: None,
            binance_qr_content: None,
            binance_deeplink: None,
            binance_universal_url: None,
            binance_transaction_id: None,
            binance_open_user_id: None,
            tx_hash: None,
            tx_from: None,
            tx_block_number: None,
            confirmations: 0,
            status: CryptoPaymentStatus::Pending,
            failure_reason: None,
            created_at: "2024-05-18T00:00:00+00:00".to_string(),
            expires_at: "2024-05-18T04:30:00+00:00".to_string(),
            completed_at: None,
            updated_at: "2024-05-18T00:00:00+00:00".to_string(),
        }
    }

    #[test]
    fn missing_note_is_ignored() {
        assert_eq!(
            classify_binance_pay_transaction(&tx(None, dec!(1), "USDT"), &[], false, 0, 10),
            BinancePayMatchDecision::Ignore {
                reason: "missing_note".to_string()
            }
        );
    }

    #[test]
    fn unknown_note_is_ignored() {
        assert_eq!(
            classify_binance_pay_transaction(&tx(Some("VI1"), dec!(1), "USDT"), &[], false, 0, 10),
            BinancePayMatchDecision::Ignore {
                reason: "unknown_note".to_string()
            }
        );
    }

    #[test]
    fn duplicate_candidates_go_manual_review() {
        assert_eq!(
            classify_binance_pay_transaction(
                &tx(Some("VI1"), dec!(1), "USDT"),
                &[payment(1, "VI1"), payment(2, "VI1")],
                false,
                0,
                10,
            ),
            BinancePayMatchDecision::ManualReview {
                payment_id: None,
                reason: "duplicate_active_note".to_string()
            }
        );
    }

    #[test]
    fn amount_or_currency_mismatch_go_manual_review() {
        assert_eq!(
            classify_binance_pay_transaction(
                &tx(Some("VI1"), dec!(2), "USDT"),
                &[payment(1, "VI1")],
                false,
                0,
                10,
            ),
            BinancePayMatchDecision::ManualReview {
                payment_id: Some(1),
                reason: "amount_mismatch".to_string()
            }
        );
        assert_eq!(
            classify_binance_pay_transaction(
                &tx(Some("VI1"), dec!(1), "BUSD"),
                &[payment(1, "VI1")],
                false,
                0,
                10,
            ),
            BinancePayMatchDecision::ManualReview {
                payment_id: Some(1),
                reason: "currency_mismatch".to_string()
            }
        );
    }

    #[test]
    fn duplicate_provider_tx_is_ignored() {
        assert_eq!(
            classify_binance_pay_transaction(
                &tx(Some("VI1"), dec!(1), "USDT"),
                &[payment(1, "VI1")],
                true,
                0,
                10,
            ),
            BinancePayMatchDecision::Ignore {
                reason: "duplicate_provider_tx".to_string()
            }
        );
    }

    #[test]
    fn valid_transaction_matches() {
        assert_eq!(
            classify_binance_pay_transaction(
                &tx(Some("VI1"), dec!(1), "USDT"),
                &[payment(1, "VI1")],
                false,
                0,
                10,
            ),
            BinancePayMatchDecision::Match { payment_id: 1 }
        );
    }
}
