use anyhow::{Result, anyhow};
use chrono::Utc;
use sqlx::{SqliteConnection, SqlitePool};

use super::models::{AdminWalletUser, Wallet, WalletTopupRequest, WalletTransaction};

// ── Wallet balance ────────────────────────────────────────────────

pub async fn get_or_create_wallet(pool: &SqlitePool, user_id: i64) -> Result<Wallet> {
    sqlx::query(
        "INSERT OR IGNORE INTO wallets (user_id, balance, updated_at)
         VALUES (?, 0, datetime('now'))",
    )
    .bind(user_id)
    .execute(pool)
    .await?;

    let wallet = sqlx::query_as::<_, Wallet>(
        "SELECT user_id, balance, updated_at FROM wallets WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_one(pool)
    .await?;

    Ok(wallet)
}

/// Cộng tiền vào ví, ghi log transaction. Trả về balance sau khi cộng.
pub async fn credit_wallet(
    tx: &mut SqliteConnection,
    user_id: i64,
    amount: i64,
    tx_type: &str,
    order_id: Option<&str>,
    topup_id: Option<i64>,
    note: Option<&str>,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();

    let balance_after: Option<i64> = sqlx::query_scalar(
        "UPDATE wallets SET balance = balance + ?, updated_at = ?
         WHERE user_id = ?
         RETURNING balance",
    )
    .bind(amount)
    .bind(&now)
    .bind(user_id)
    .fetch_optional(&mut *tx)
    .await?;

    // Nếu chưa có ví thì insert trước rồi retry
    let Some(balance_after) = balance_after else {
        sqlx::query(
            "INSERT OR IGNORE INTO wallets (user_id, balance, updated_at) VALUES (?, 0, ?)",
        )
        .bind(user_id)
        .bind(&now)
        .execute(&mut *tx)
        .await?;

        let b: i64 = sqlx::query_scalar(
            "UPDATE wallets SET balance = balance + ?, updated_at = ?
             WHERE user_id = ?
             RETURNING balance",
        )
        .bind(amount)
        .bind(&now)
        .bind(user_id)
        .fetch_one(&mut *tx)
        .await?;

        log_transaction(tx, user_id, tx_type, amount, b, order_id, topup_id, note).await?;
        return Ok(b);
    };

    log_transaction(
        tx,
        user_id,
        tx_type,
        amount,
        balance_after,
        order_id,
        topup_id,
        note,
    )
    .await?;
    Ok(balance_after)
}

/// Trừ tiền khỏi ví (atomic check balance >= amount). Trả về balance sau khi trừ.
/// Lỗi nếu không đủ số dư.
pub async fn debit_wallet(
    tx: &mut SqliteConnection,
    user_id: i64,
    amount: i64,
    order_id: &str,
    note: Option<&str>,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();

    let balance_after: Option<i64> = sqlx::query_scalar(
        "UPDATE wallets SET balance = balance - ?, updated_at = ?
         WHERE user_id = ? AND balance >= ?
         RETURNING balance",
    )
    .bind(amount)
    .bind(&now)
    .bind(user_id)
    .bind(amount)
    .fetch_optional(&mut *tx)
    .await?;

    let balance_after = balance_after.ok_or_else(|| anyhow!("Số dư ví không đủ"))?;

    log_transaction(
        tx,
        user_id,
        "purchase",
        -amount,
        balance_after,
        Some(order_id),
        None,
        note,
    )
    .await?;

    Ok(balance_after)
}

/// Admin: điều chỉnh số dư (cộng hoặc trừ, amount có thể âm).
pub async fn admin_adjust_wallet(
    pool: &SqlitePool,
    user_id: i64,
    amount: i64,
    note: Option<&str>,
) -> Result<i64> {
    let now = Utc::now().to_rfc3339();

    sqlx::query("INSERT OR IGNORE INTO wallets (user_id, balance, updated_at) VALUES (?, 0, ?)")
        .bind(user_id)
        .bind(&now)
        .execute(pool)
        .await?;

    let mut tx = pool.begin().await?;

    let balance_after: i64 = sqlx::query_scalar(
        "UPDATE wallets SET balance = balance + ?, updated_at = ?
         WHERE user_id = ?
         RETURNING balance",
    )
    .bind(amount)
    .bind(&now)
    .bind(user_id)
    .fetch_one(&mut *tx)
    .await?;

    log_transaction(
        &mut tx,
        user_id,
        "admin_adjust",
        amount,
        balance_after,
        None,
        None,
        note,
    )
    .await?;

    tx.commit().await?;
    Ok(balance_after)
}

async fn log_transaction(
    tx: &mut SqliteConnection,
    user_id: i64,
    tx_type: &str,
    amount: i64,
    balance_after: i64,
    order_id: Option<&str>,
    topup_id: Option<i64>,
    note: Option<&str>,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO wallet_transactions
         (user_id, type, amount, balance_after, order_id, topup_id, note)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(tx_type)
    .bind(amount)
    .bind(balance_after)
    .bind(order_id)
    .bind(topup_id)
    .bind(note)
    .execute(&mut *tx)
    .await?;
    Ok(())
}

pub async fn list_transactions(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<WalletTransaction>> {
    let rows = sqlx::query_as::<_, WalletTransaction>(
        "SELECT id, user_id, type, amount, balance_after,
                order_id, topup_id, note, created_at
         FROM wallet_transactions
         WHERE user_id = ?
         ORDER BY created_at DESC
         LIMIT ?",
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn list_all_wallets(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<Wallet>> {
    let rows = sqlx::query_as::<_, Wallet>(
        "SELECT user_id, balance, updated_at FROM wallets ORDER BY balance DESC LIMIT ? OFFSET ?",
    )
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn count_wallets(pool: &SqlitePool) -> Result<i64> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM wallets")
        .fetch_one(pool)
        .await?)
}

const ADMIN_WALLET_USERS_BASE: &str = r#"
    SELECT
        s.user_id AS user_id,
        s.chat_id AS chat_id,
        s.username AS username,
        s.first_name AS first_name,
        s.last_name AS last_name,
        s.full_name AS full_name,
        s.language_code AS language_code,
        s.preferred_language AS preferred_language,
        s.is_bot AS is_bot,
        s.created_at AS user_created_at,
        s.updated_at AS user_updated_at,
        COALESCE(w.balance, 0) AS balance,
        w.updated_at AS wallet_updated_at,
        COALESCE(tx_stats.transaction_count, 0) AS transaction_count,
        tx_stats.last_transaction_at AS last_transaction_at
    FROM subscribers s
    LEFT JOIN wallets w ON w.user_id = s.user_id
    LEFT JOIN (
        SELECT user_id, COUNT(1) AS transaction_count, MAX(created_at) AS last_transaction_at
        FROM wallet_transactions
        GROUP BY user_id
    ) tx_stats ON tx_stats.user_id = s.user_id
    UNION ALL
    SELECT
        w.user_id AS user_id,
        NULL AS chat_id,
        NULL AS username,
        NULL AS first_name,
        NULL AS last_name,
        NULL AS full_name,
        NULL AS language_code,
        NULL AS preferred_language,
        NULL AS is_bot,
        NULL AS user_created_at,
        NULL AS user_updated_at,
        w.balance AS balance,
        w.updated_at AS wallet_updated_at,
        COALESCE(tx_stats.transaction_count, 0) AS transaction_count,
        tx_stats.last_transaction_at AS last_transaction_at
    FROM wallets w
    LEFT JOIN subscribers s ON s.user_id = w.user_id
    LEFT JOIN (
        SELECT user_id, COUNT(1) AS transaction_count, MAX(created_at) AS last_transaction_at
        FROM wallet_transactions
        GROUP BY user_id
    ) tx_stats ON tx_stats.user_id = w.user_id
    WHERE s.user_id IS NULL
"#;

const ADMIN_WALLET_USERS_FILTER: &str = r#"
    WHERE ? = ''
       OR CAST(user_id AS TEXT) LIKE ?
       OR CAST(COALESCE(chat_id, '') AS TEXT) LIKE ?
       OR COALESCE(username, '') LIKE ?
       OR COALESCE(first_name, '') LIKE ?
       OR COALESCE(last_name, '') LIKE ?
       OR COALESCE(full_name, '') LIKE ?
"#;

pub async fn list_admin_wallet_users(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    query: Option<&str>,
) -> Result<Vec<AdminWalletUser>> {
    let raw_query = query.map(str::trim).unwrap_or("");
    let pattern = format!("%{raw_query}%");
    let sql = format!(
        "SELECT * FROM ({ADMIN_WALLET_USERS_BASE}) wallet_users
         {ADMIN_WALLET_USERS_FILTER}
         ORDER BY balance DESC, COALESCE(last_transaction_at, user_created_at, wallet_updated_at, '') DESC, user_id DESC
         LIMIT ? OFFSET ?"
    );

    let rows = sqlx::query_as::<_, AdminWalletUser>(&sql)
        .bind(raw_query)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;
    Ok(rows)
}

pub async fn count_admin_wallet_users(pool: &SqlitePool, query: Option<&str>) -> Result<i64> {
    let raw_query = query.map(str::trim).unwrap_or("");
    let pattern = format!("%{raw_query}%");
    let sql = format!(
        "SELECT COUNT(1) FROM ({ADMIN_WALLET_USERS_BASE}) wallet_users {ADMIN_WALLET_USERS_FILTER}"
    );

    Ok(sqlx::query_scalar::<_, i64>(&sql)
        .bind(raw_query)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .bind(&pattern)
        .fetch_one(pool)
        .await?)
}

// ── Top-up requests ───────────────────────────────────────────────

pub async fn create_topup_request(
    pool: &SqlitePool,
    user_id: i64,
    chat_id: i64,
    amount: i64,
    memo: &str,
) -> Result<WalletTopupRequest> {
    sqlx::query(
        "INSERT INTO wallet_topup_requests (user_id, chat_id, amount, memo)
         VALUES (?, ?, ?, ?)",
    )
    .bind(user_id)
    .bind(chat_id)
    .bind(amount)
    .bind(memo)
    .execute(pool)
    .await?;

    let req = sqlx::query_as::<_, WalletTopupRequest>(
        "SELECT id, user_id, chat_id, amount, memo, status, created_at, completed_at
         FROM wallet_topup_requests WHERE memo = ?",
    )
    .bind(memo)
    .fetch_one(pool)
    .await?;

    Ok(req)
}

pub async fn find_topup_by_memo(
    pool: &SqlitePool,
    memo: &str,
) -> Result<Option<WalletTopupRequest>> {
    let row = sqlx::query_as::<_, WalletTopupRequest>(
        "SELECT id, user_id, chat_id, amount, memo, status, created_at, completed_at
         FROM wallet_topup_requests WHERE memo = ?",
    )
    .bind(memo)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn find_latest_pending_topup_by_user_id(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<WalletTopupRequest>> {
    let row = sqlx::query_as::<_, WalletTopupRequest>(
        "SELECT id, user_id, chat_id, amount, memo, status, created_at, completed_at
         FROM wallet_topup_requests
         WHERE user_id = ? AND status = 'pending'
         ORDER BY created_at DESC, id DESC
         LIMIT 1",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

pub async fn complete_topup(tx: &mut SqliteConnection, topup_id: i64) -> Result<bool> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE wallet_topup_requests SET status = 'completed', completed_at = ?
         WHERE id = ? AND status = 'pending'",
    )
    .bind(&now)
    .bind(topup_id)
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn complete_paid_topup(tx: &mut SqliteConnection, topup_id: i64) -> Result<bool> {
    let now = Utc::now().to_rfc3339();
    let result = sqlx::query(
        "UPDATE wallet_topup_requests SET status = 'completed', completed_at = ?
         WHERE id = ? AND status IN ('pending', 'expired')",
    )
    .bind(&now)
    .bind(topup_id)
    .execute(&mut *tx)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn expire_topup(pool: &SqlitePool, topup_id: i64) -> Result<()> {
    sqlx::query(
        "UPDATE wallet_topup_requests SET status = 'expired' WHERE id = ? AND status = 'pending'",
    )
    .bind(topup_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_pending_topups_before(
    pool: &SqlitePool,
    cutoff: &str,
) -> Result<Vec<WalletTopupRequest>> {
    let rows = sqlx::query_as::<_, WalletTopupRequest>(
        "SELECT id, user_id, chat_id, amount, memo, status, created_at, completed_at
         FROM wallet_topup_requests
         WHERE status = 'pending' AND created_at < ?",
    )
    .bind(cutoff)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

/// Admin: nạp thủ công vào ví. Ghi loại giao dịch là `topup`.
pub async fn admin_manual_topup_wallet(
    pool: &SqlitePool,
    user_id: i64,
    amount: i64,
    note: Option<&str>,
) -> Result<i64> {
    let mut tx = pool.begin().await?;
    let balance_after = credit_wallet(&mut tx, user_id, amount, "topup", None, None, note).await?;
    tx.commit().await?;
    Ok(balance_after)
}

pub async fn credit_order_payment_to_wallet_once(
    tx: &mut SqliteConnection,
    user_id: i64,
    amount: i64,
    order_id: &str,
    note: Option<&str>,
) -> Result<Option<i64>> {
    let existing: i64 = sqlx::query_scalar(
        "SELECT COUNT(1) FROM wallet_transactions
         WHERE type = 'refund' AND order_id = ?",
    )
    .bind(order_id)
    .fetch_one(&mut *tx)
    .await?;

    if existing > 0 {
        return Ok(None);
    }

    let balance_after =
        credit_wallet(tx, user_id, amount, "refund", Some(order_id), None, note).await?;
    Ok(Some(balance_after))
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn credit_wallet_handles_balance_becoming_zero() {
        let pool = test_pool().await;
        sqlx::query("INSERT INTO wallets (user_id, balance, updated_at) VALUES (?, ?, ?)")
            .bind(42_i64)
            .bind(-1_000_i64)
            .bind("2026-05-13T00:00:00Z")
            .execute(&pool)
            .await
            .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let balance_after = credit_wallet(&mut tx, 42, 1_000, "topup", None, None, None)
            .await
            .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(balance_after, 0);
        let wallet = get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 0);
    }

    #[tokio::test]
    async fn list_admin_wallet_users_includes_subscribers_without_wallets() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO subscribers (user_id, chat_id, username, full_name)
             VALUES (?, ?, ?, ?)",
        )
        .bind(7_i64)
        .bind(70_i64)
        .bind("alice")
        .bind("Alice Nguyen")
        .execute(&pool)
        .await
        .unwrap();

        let rows = list_admin_wallet_users(&pool, 20, 0, None).await.unwrap();

        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].user_id, 7);
        assert_eq!(rows[0].username.as_deref(), Some("alice"));
        assert_eq!(rows[0].balance, 0);
        assert_eq!(rows[0].transaction_count, 0);
    }

    #[tokio::test]
    async fn credit_order_payment_to_wallet_once_is_idempotent_per_order() {
        let pool = test_pool().await;

        let mut tx = pool.begin().await.unwrap();
        let first = credit_order_payment_to_wallet_once(
            &mut tx,
            42,
            50_000,
            "order-1",
            Some("payment fallback"),
        )
        .await
        .unwrap();
        let second = credit_order_payment_to_wallet_once(
            &mut tx,
            42,
            50_000,
            "order-1",
            Some("payment fallback"),
        )
        .await
        .unwrap();
        tx.commit().await.unwrap();

        assert_eq!(first, Some(50_000));
        assert_eq!(second, None);
        let wallet = get_or_create_wallet(&pool, 42).await.unwrap();
        assert_eq!(wallet.balance, 50_000);
        let transactions = list_transactions(&pool, 42, 10).await.unwrap();
        assert_eq!(transactions.len(), 1);
        assert_eq!(transactions[0].tx_type, "refund");
        assert_eq!(transactions[0].order_id.as_deref(), Some("order-1"));
    }

    #[tokio::test]
    async fn complete_paid_topup_allows_expired_request_once() {
        let pool = test_pool().await;
        sqlx::query(
            "INSERT INTO wallet_topup_requests (id, user_id, chat_id, amount, memo, status)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(1_i64)
        .bind(42_i64)
        .bind(420_i64)
        .bind(50_000_i64)
        .bind("NAPABC12345")
        .bind("expired")
        .execute(&pool)
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        assert!(complete_paid_topup(&mut tx, 1).await.unwrap());
        assert!(!complete_paid_topup(&mut tx, 1).await.unwrap());
        tx.commit().await.unwrap();

        let status: String =
            sqlx::query_scalar("SELECT status FROM wallet_topup_requests WHERE id = ?")
                .bind(1_i64)
                .fetch_one(&pool)
                .await
                .unwrap();
        assert_eq!(status, "completed");
    }
}
