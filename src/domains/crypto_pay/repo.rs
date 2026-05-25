use anyhow::Result;
use rust_decimal::Decimal;
use serde_json::Value;
use sqlx::{Row, Sqlite, SqlitePool, Transaction, sqlite::SqliteRow};

use crate::domains::crypto_pay::models::{CryptoPaymentMethod, CryptoPaymentRequest};

#[derive(Debug, Clone)]
pub struct NewCryptoPayment {
    pub purpose: String,
    pub order_id: Option<String>,
    pub wallet_topup_id: Option<i64>,
    pub user_id: i64,
    pub chat_id: i64,
    pub method: CryptoPaymentMethod,
    pub amount_vnd: i64,
    pub rate_vnd_per_usdt: Decimal,
    pub amount_usdt_base: Decimal,
    pub amount_usdt_expected: Decimal,
    pub amount_token_units: String,
    pub memo: String,
    pub address: Option<String>,
    pub binance_prepay_id: Option<String>,
    pub binance_checkout_url: Option<String>,
    pub binance_qrcode_link: Option<String>,
    pub binance_qr_content: Option<String>,
    pub binance_deeplink: Option<String>,
    pub binance_universal_url: Option<String>,
    pub expires_at: String,
}

#[derive(Debug, Clone)]
pub struct BinancePayTransactionAudit {
    pub provider_tx_id: Option<String>,
    pub provider_order_id: Option<String>,
    pub provider_raw_id: Option<String>,
    pub note: Option<String>,
    pub amount_usdt: String,
    pub currency: String,
    pub transaction_time_ms: i64,
    pub status: Option<String>,
    pub direction: Option<String>,
    pub raw_json: Value,
}

pub async fn create_crypto_payment(
    pool: &SqlitePool,
    input: NewCryptoPayment,
) -> Result<CryptoPaymentRequest> {
    let result = sqlx::query(
        r#"
        INSERT INTO crypto_payment_requests (
            purpose,
            order_id,
            wallet_topup_id,
            user_id,
            chat_id,
            method,
            amount_vnd,
            amount_usdt,
            rate_vnd_per_usdt,
            amount_usdt_base,
            amount_usdt_expected,
            amount_token_units,
            memo,
            address,
            binance_prepay_id,
            binance_checkout_url,
            binance_qrcode_link,
            binance_qr_content,
            binance_deeplink,
            binance_universal_url,
            expires_at
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        "#,
    )
    .bind(&input.purpose)
    .bind(&input.order_id)
    .bind(input.wallet_topup_id)
    .bind(input.user_id)
    .bind(input.chat_id)
    .bind(input.method.to_string())
    .bind(input.amount_vnd)
    .bind(input.amount_usdt_expected.to_string().parse::<f64>().ok())
    .bind(input.rate_vnd_per_usdt.to_string())
    .bind(input.amount_usdt_base.to_string())
    .bind(input.amount_usdt_expected.to_string())
    .bind(&input.amount_token_units)
    .bind(&input.memo)
    .bind(&input.address)
    .bind(&input.binance_prepay_id)
    .bind(&input.binance_checkout_url)
    .bind(&input.binance_qrcode_link)
    .bind(&input.binance_qr_content)
    .bind(&input.binance_deeplink)
    .bind(&input.binance_universal_url)
    .bind(&input.expires_at)
    .execute(pool)
    .await?;

    find_crypto_payment_by_id(pool, result.last_insert_rowid())
        .await?
        .ok_or_else(|| anyhow::anyhow!("created crypto payment not found"))
}

pub async fn find_crypto_payment_by_id(
    pool: &SqlitePool,
    id: i64,
) -> Result<Option<CryptoPaymentRequest>> {
    fetch_optional(
        pool,
        "SELECT * FROM crypto_payment_requests WHERE id = ?",
        &[SqlParam::I64(id)],
    )
    .await
}

pub async fn find_crypto_payment_by_memo(
    pool: &SqlitePool,
    memo: &str,
) -> Result<Option<CryptoPaymentRequest>> {
    fetch_optional(
        pool,
        "SELECT * FROM crypto_payment_requests WHERE memo = ?",
        &[SqlParam::Str(memo)],
    )
    .await
}

pub async fn list_crypto_payments_admin(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
) -> Result<Vec<CryptoPaymentRequest>> {
    let rows = sqlx::query(
        "SELECT * FROM crypto_payment_requests
         ORDER BY id DESC
         LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(map_row).collect()
}

pub async fn count_crypto_payments_admin(pool: &SqlitePool) -> Result<i64> {
    sqlx::query_scalar("SELECT COUNT(1) FROM crypto_payment_requests")
        .fetch_one(pool)
        .await
        .map_err(Into::into)
}

pub async fn find_pending_crypto_payment_by_order(
    pool: &SqlitePool,
    order_id: &str,
) -> Result<Option<CryptoPaymentRequest>> {
    fetch_optional(
        pool,
        "SELECT * FROM crypto_payment_requests
         WHERE order_id = ? AND status IN ('pending','confirming')
         ORDER BY id DESC LIMIT 1",
        &[SqlParam::Str(order_id)],
    )
    .await
}

pub async fn find_pending_crypto_payment_by_wallet_topup(
    pool: &SqlitePool,
    wallet_topup_id: i64,
) -> Result<Option<CryptoPaymentRequest>> {
    fetch_optional(
        pool,
        "SELECT * FROM crypto_payment_requests
         WHERE wallet_topup_id = ? AND status IN ('pending','confirming')
         ORDER BY id DESC LIMIT 1",
        &[SqlParam::I64(wallet_topup_id)],
    )
    .await
}

pub async fn find_pending_bep20_by_token_units(
    pool: &SqlitePool,
    amount_token_units: &str,
) -> Result<Option<CryptoPaymentRequest>> {
    fetch_optional(
        pool,
        "SELECT * FROM crypto_payment_requests
         WHERE method = 'bep20'
           AND status IN ('pending','confirming')
           AND amount_token_units = ?
         ORDER BY id DESC LIMIT 1",
        &[SqlParam::Str(amount_token_units)],
    )
    .await
}

pub async fn list_pending_crypto_payments_before(
    pool: &SqlitePool,
    cutoff: &str,
) -> Result<Vec<CryptoPaymentRequest>> {
    let rows = sqlx::query(
        "SELECT * FROM crypto_payment_requests
         WHERE status = 'pending' AND expires_at < ?
         ORDER BY expires_at ASC, id ASC",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(map_row).collect()
}

pub async fn list_bep20_payments_for_scan(pool: &SqlitePool) -> Result<Vec<CryptoPaymentRequest>> {
    let rows = sqlx::query(
        "SELECT * FROM crypto_payment_requests
         WHERE method = 'bep20'
           AND status IN ('pending','confirming')
         ORDER BY id ASC",
    )
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(map_row).collect()
}

pub async fn list_active_binance_pay_candidates_by_memo(
    pool: &SqlitePool,
    memo: &str,
) -> Result<Vec<CryptoPaymentRequest>> {
    let rows = sqlx::query(
        "SELECT * FROM crypto_payment_requests
         WHERE method = 'binance_pay'
           AND status IN ('pending','confirming')
           AND memo = ?
         ORDER BY id ASC",
    )
    .bind(memo)
    .fetch_all(pool)
    .await?;
    rows.into_iter().map(map_row).collect()
}

pub async fn provider_tx_already_completed(
    pool: &SqlitePool,
    provider_ref: &str,
    except_payment_id: Option<i64>,
) -> Result<bool> {
    let count: i64 = sqlx::query_scalar(
        "SELECT COUNT(1) FROM crypto_payment_requests
         WHERE status = 'completed'
           AND (? IS NULL OR id != ?)
           AND (tx_hash = ? OR binance_transaction_id = ?)",
    )
    .bind(except_payment_id)
    .bind(except_payment_id)
    .bind(provider_ref)
    .bind(provider_ref)
    .fetch_one(pool)
    .await?;
    Ok(count > 0)
}

pub async fn upsert_binance_pay_transaction(
    pool: &SqlitePool,
    input: &BinancePayTransactionAudit,
) -> Result<()> {
    let raw_json = serde_json::to_string(&input.raw_json)?;
    let conflict_clause = if input
        .provider_tx_id
        .as_deref()
        .is_some_and(|v| !v.is_empty())
    {
        "ON CONFLICT(provider_tx_id) WHERE provider_tx_id IS NOT NULL AND provider_tx_id != ''"
    } else {
        "ON CONFLICT(provider_raw_id) WHERE provider_raw_id IS NOT NULL AND provider_raw_id != ''"
    };
    let sql = format!(
        r#"
        INSERT INTO binance_pay_transactions (
            provider_tx_id,
            provider_order_id,
            provider_raw_id,
            note,
            amount_usdt,
            currency,
            transaction_time_ms,
            status,
            direction,
            raw_json
        )
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        {conflict_clause}
        DO UPDATE SET
            provider_order_id = excluded.provider_order_id,
            provider_raw_id = excluded.provider_raw_id,
            note = excluded.note,
            amount_usdt = excluded.amount_usdt,
            currency = excluded.currency,
            transaction_time_ms = excluded.transaction_time_ms,
            status = excluded.status,
            direction = excluded.direction,
            raw_json = excluded.raw_json,
            last_seen_at = datetime('now')
        "#
    );
    sqlx::query(&sql)
        .bind(&input.provider_tx_id)
        .bind(&input.provider_order_id)
        .bind(&input.provider_raw_id)
        .bind(&input.note)
        .bind(&input.amount_usdt)
        .bind(&input.currency)
        .bind(input.transaction_time_ms)
        .bind(&input.status)
        .bind(&input.direction)
        .bind(raw_json)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn mark_binance_pay_transaction_match(
    pool: &SqlitePool,
    provider_ref: &str,
    matched_payment_id: Option<i64>,
    match_status: &str,
    match_reason: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"
        UPDATE binance_pay_transactions
        SET matched_payment_id = ?,
            match_status = ?,
            match_reason = ?,
            last_seen_at = datetime('now')
        WHERE provider_tx_id = ? OR provider_raw_id = ?
        "#,
    )
    .bind(matched_payment_id)
    .bind(match_status)
    .bind(match_reason)
    .bind(provider_ref)
    .bind(provider_ref)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn mark_crypto_payment_confirming(
    pool: &SqlitePool,
    id: i64,
    tx_hash: &str,
    tx_from: &str,
    block_number: i64,
    confirmations: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'confirming',
            tx_hash = ?,
            tx_from = ?,
            tx_block_number = ?,
            confirmations = ?,
            updated_at = datetime('now')
        WHERE id = ? AND status = 'pending'
        "#,
    )
    .bind(tx_hash)
    .bind(tx_from)
    .bind(block_number)
    .bind(confirmations)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn mark_crypto_payment_manual_review(
    pool: &SqlitePool,
    id: i64,
    reason: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'manual_review',
            failure_reason = ?,
            updated_at = datetime('now')
        WHERE id = ? AND status IN ('pending','confirming','expired')
        "#,
    )
    .bind(reason)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

#[allow(clippy::too_many_arguments)]
pub async fn update_binance_order_fields(
    pool: &SqlitePool,
    id: i64,
    prepay_id: &str,
    checkout_url: &str,
    qrcode_link: &str,
    qr_content: &str,
    deeplink: &str,
    universal_url: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET binance_prepay_id = ?,
            binance_checkout_url = ?,
            binance_qrcode_link = ?,
            binance_qr_content = ?,
            binance_deeplink = ?,
            binance_universal_url = ?,
            updated_at = datetime('now')
        WHERE id = ? AND method = 'binance_pay'
        "#,
    )
    .bind(prepay_id)
    .bind(checkout_url)
    .bind(qrcode_link)
    .bind(qr_content)
    .bind(deeplink)
    .bind(universal_url)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn complete_crypto_payment(
    tx: &mut Transaction<'_, Sqlite>,
    id: i64,
    tx_hash: Option<&str>,
    confirmations: i64,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'completed',
            tx_hash = COALESCE(?, tx_hash),
            confirmations = ?,
            completed_at = datetime('now'),
            updated_at = datetime('now')
        WHERE id = ? AND status IN ('pending','confirming')
        "#,
    )
    .bind(tx_hash)
    .bind(confirmations)
    .bind(id)
    .execute(tx.as_mut())
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn complete_binance_pay_payment(
    tx: &mut Transaction<'_, Sqlite>,
    id: i64,
    provider_ref: &str,
) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'completed',
            binance_transaction_id = COALESCE(?, binance_transaction_id),
            tx_hash = COALESCE(?, tx_hash),
            confirmations = 0,
            completed_at = datetime('now'),
            updated_at = datetime('now')
        WHERE id = ? AND status IN ('pending','confirming')
        "#,
    )
    .bind(provider_ref)
    .bind(provider_ref)
    .bind(id)
    .execute(tx.as_mut())
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn expire_crypto_payment(pool: &SqlitePool, id: i64) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'expired', updated_at = datetime('now')
        WHERE id = ? AND status = 'pending'
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn expire_crypto_payment_tx(tx: &mut Transaction<'_, Sqlite>, id: i64) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'expired', updated_at = datetime('now')
        WHERE id = ? AND status = 'pending'
        "#,
    )
    .bind(id)
    .execute(tx.as_mut())
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn fail_crypto_payment(pool: &SqlitePool, id: i64, reason: &str) -> Result<bool> {
    let result = sqlx::query(
        r#"
        UPDATE crypto_payment_requests
        SET status = 'failed',
            failure_reason = ?,
            updated_at = datetime('now')
        WHERE id = ? AND status IN ('pending','confirming','manual_review')
        "#,
    )
    .bind(reason)
    .bind(id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn get_worker_state(pool: &SqlitePool, key: &str) -> Result<Option<String>> {
    sqlx::query_scalar("SELECT value FROM crypto_worker_state WHERE key = ?")
        .bind(key)
        .fetch_optional(pool)
        .await
        .map_err(Into::into)
}

pub async fn set_worker_state(pool: &SqlitePool, key: &str, value: &str) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO crypto_worker_state (key, value, updated_at)
        VALUES (?, ?, datetime('now'))
        ON CONFLICT(key) DO UPDATE SET
            value = excluded.value,
            updated_at = datetime('now')
        "#,
    )
    .bind(key)
    .bind(value)
    .execute(pool)
    .await?;
    Ok(())
}

enum SqlParam<'a> {
    I64(i64),
    Str(&'a str),
}

async fn fetch_optional(
    pool: &SqlitePool,
    sql: &str,
    params: &[SqlParam<'_>],
) -> Result<Option<CryptoPaymentRequest>> {
    let mut query = sqlx::query(sql);
    for param in params {
        query = match param {
            SqlParam::I64(value) => query.bind(value),
            SqlParam::Str(value) => query.bind(value),
        };
    }
    query.fetch_optional(pool).await?.map(map_row).transpose()
}

fn map_row(row: SqliteRow) -> Result<CryptoPaymentRequest> {
    Ok(CryptoPaymentRequest {
        id: row.try_get("id")?,
        purpose: row.try_get("purpose")?,
        order_id: row.try_get("order_id")?,
        wallet_topup_id: row.try_get("wallet_topup_id")?,
        user_id: row.try_get("user_id")?,
        chat_id: row.try_get("chat_id")?,
        method: parse_string(&row, "method")?,
        amount_vnd: row.try_get("amount_vnd")?,
        rate_vnd_per_usdt: parse_decimal(&row, "rate_vnd_per_usdt")?,
        amount_usdt_base: parse_decimal(&row, "amount_usdt_base")?,
        amount_usdt_expected: parse_decimal(&row, "amount_usdt_expected")?,
        amount_token_units: row.try_get("amount_token_units")?,
        memo: row.try_get("memo")?,
        address: row.try_get("address")?,
        binance_prepay_id: row.try_get("binance_prepay_id")?,
        binance_checkout_url: row.try_get("binance_checkout_url")?,
        binance_qrcode_link: row.try_get("binance_qrcode_link")?,
        binance_qr_content: row.try_get("binance_qr_content")?,
        binance_deeplink: row.try_get("binance_deeplink")?,
        binance_universal_url: row.try_get("binance_universal_url")?,
        binance_transaction_id: row.try_get("binance_transaction_id")?,
        binance_open_user_id: row.try_get("binance_open_user_id")?,
        tx_hash: row.try_get("tx_hash")?,
        tx_from: row.try_get("tx_from")?,
        tx_block_number: row.try_get("tx_block_number")?,
        confirmations: row.try_get("confirmations")?,
        status: parse_string(&row, "status")?,
        failure_reason: row.try_get("failure_reason")?,
        created_at: row.try_get("created_at")?,
        expires_at: row.try_get("expires_at")?,
        completed_at: row.try_get("completed_at")?,
        updated_at: row.try_get("updated_at")?,
    })
}

fn parse_decimal(row: &SqliteRow, column: &str) -> Result<Decimal> {
    let value: String = row.try_get(column)?;
    value.parse().map_err(Into::into)
}

fn parse_string<T>(row: &SqliteRow, column: &str) -> Result<T>
where
    T: std::str::FromStr,
    T::Err: Into<anyhow::Error>,
{
    let value: String = row.try_get(column)?;
    value.parse::<T>().map_err(Into::into)
}

#[cfg(test)]
mod tests {
    use rust_decimal_macros::dec;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};

    use super::*;
    use crate::domains::crypto_pay::models::{CryptoPaymentMethod, CryptoPaymentStatus};

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    async fn seed_order(pool: &SqlitePool, order_id: &str) {
        sqlx::query("INSERT INTO products (id, name, price, is_active) VALUES (?, ?, ?, ?)")
            .bind(1_i64)
            .bind("USDT Test")
            .bind(50_000_i64)
            .bind(1_i64)
            .execute(pool)
            .await
            .unwrap();
        sqlx::query(
            "INSERT INTO orders (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(order_id)
        .bind(42_i64)
        .bind(420_i64)
        .bind(1_i64)
        .bind(1_i64)
        .bind(50_000_i64)
        .bind("pending")
        .bind("DHUSDT001")
        .bind("2026-05-21T01:00:00+00:00")
        .execute(pool)
        .await
        .unwrap();
    }

    async fn create_bep20(pool: &SqlitePool, memo: &str, units: &str) -> i64 {
        create_crypto_payment(
            pool,
            NewCryptoPayment {
                purpose: "order".to_string(),
                order_id: Some("order-1".to_string()),
                wallet_topup_id: None,
                user_id: 42,
                chat_id: 420,
                method: CryptoPaymentMethod::Bep20,
                amount_vnd: 50_000,
                rate_vnd_per_usdt: dec!(25250),
                amount_usdt_base: dec!(1.99),
                amount_usdt_expected: dec!(1.993842),
                amount_token_units: units.to_string(),
                memo: memo.to_string(),
                address: Some("0x0000000000000000000000000000000000000001".to_string()),
                binance_prepay_id: None,
                binance_checkout_url: None,
                binance_qrcode_link: None,
                binance_qr_content: None,
                binance_deeplink: None,
                binance_universal_url: None,
                expires_at: "2026-05-21T01:30:00+00:00".to_string(),
            },
        )
        .await
        .unwrap()
        .id
    }

    #[tokio::test]
    async fn creates_and_finds_pending_bep20_payment() {
        let pool = test_pool().await;
        seed_order(&pool, "order-1").await;
        let id = create_bep20(&pool, "CPRTEST001", "1993842000000000000").await;

        let by_id = find_crypto_payment_by_id(&pool, id).await.unwrap().unwrap();
        let by_order = find_pending_crypto_payment_by_order(&pool, "order-1")
            .await
            .unwrap()
            .unwrap();
        let by_units = find_pending_bep20_by_token_units(&pool, "1993842000000000000")
            .await
            .unwrap()
            .unwrap();

        assert_eq!(by_id.memo, "CPRTEST001");
        assert_eq!(by_order.id, id);
        assert_eq!(by_units.method, CryptoPaymentMethod::Bep20);
        assert_eq!(by_units.status, CryptoPaymentStatus::Pending);
    }

    #[tokio::test]
    async fn confirming_and_completion_are_idempotent() {
        let pool = test_pool().await;
        seed_order(&pool, "order-1").await;
        let id = create_bep20(&pool, "CPRTEST002", "1993842000000000000").await;

        assert!(
            mark_crypto_payment_confirming(&pool, id, "0xtx", "0xfrom", 123, 3,)
                .await
                .unwrap()
        );
        let confirming = find_crypto_payment_by_id(&pool, id).await.unwrap().unwrap();
        assert_eq!(confirming.status, CryptoPaymentStatus::Confirming);
        assert_eq!(confirming.tx_hash.as_deref(), Some("0xtx"));

        let mut tx = pool.begin().await.unwrap();
        assert!(
            complete_crypto_payment(&mut tx, id, Some("0xtx"), 12)
                .await
                .unwrap()
        );
        tx.commit().await.unwrap();

        let mut tx = pool.begin().await.unwrap();
        assert!(
            !complete_crypto_payment(&mut tx, id, Some("0xtx"), 12)
                .await
                .unwrap()
        );
        tx.commit().await.unwrap();
    }

    #[tokio::test]
    async fn active_bep20_token_units_are_unique_until_expired() {
        let pool = test_pool().await;
        seed_order(&pool, "order-1").await;
        create_bep20(&pool, "CPRTEST003", "1993842000000000000").await;

        let duplicate = create_crypto_payment(
            &pool,
            NewCryptoPayment {
                purpose: "order".to_string(),
                order_id: Some("order-1".to_string()),
                wallet_topup_id: None,
                user_id: 42,
                chat_id: 420,
                method: CryptoPaymentMethod::Bep20,
                amount_vnd: 50_000,
                rate_vnd_per_usdt: dec!(25250),
                amount_usdt_base: dec!(1.99),
                amount_usdt_expected: dec!(1.993842),
                amount_token_units: "1993842000000000000".to_string(),
                memo: "CPRTEST004".to_string(),
                address: Some("0x0000000000000000000000000000000000000001".to_string()),
                binance_prepay_id: None,
                binance_checkout_url: None,
                binance_qrcode_link: None,
                binance_qr_content: None,
                binance_deeplink: None,
                binance_universal_url: None,
                expires_at: "2026-05-21T01:30:00+00:00".to_string(),
            },
        )
        .await;
        assert!(duplicate.is_err());

        let first = find_pending_crypto_payment_by_order(&pool, "order-1")
            .await
            .unwrap()
            .unwrap();
        assert!(expire_crypto_payment(&pool, first.id).await.unwrap());

        let reused = create_bep20(&pool, "CPRTEST005", "1993842000000000000").await;
        assert!(reused > first.id);
    }

    #[tokio::test]
    async fn lists_only_expired_pending_crypto_payments() {
        let pool = test_pool().await;
        seed_order(&pool, "order-1").await;
        let expired_id = create_bep20(&pool, "CPRTEST006", "1000001000000000000").await;
        sqlx::query("UPDATE crypto_payment_requests SET expires_at = ? WHERE id = ?")
            .bind("2026-05-21T01:00:00+00:00")
            .bind(expired_id)
            .execute(&pool)
            .await
            .unwrap();

        let future_id = create_bep20(&pool, "CPRTEST007", "1000002000000000000").await;
        sqlx::query("UPDATE crypto_payment_requests SET expires_at = ? WHERE id = ?")
            .bind("2026-05-21T02:00:00+00:00")
            .bind(future_id)
            .execute(&pool)
            .await
            .unwrap();

        let confirming_id = create_bep20(&pool, "CPRTEST008", "1000003000000000000").await;
        sqlx::query(
            "UPDATE crypto_payment_requests SET status = 'confirming', expires_at = ? WHERE id = ?",
        )
        .bind("2026-05-21T01:00:00+00:00")
        .bind(confirming_id)
        .execute(&pool)
        .await
        .unwrap();

        let expired = list_pending_crypto_payments_before(&pool, "2026-05-21T01:30:00+00:00")
            .await
            .unwrap();
        let ids = expired.iter().map(|payment| payment.id).collect::<Vec<_>>();

        assert_eq!(ids, vec![expired_id]);
    }
}
