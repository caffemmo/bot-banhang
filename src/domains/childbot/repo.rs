use anyhow::Result;
use rand::{Rng, distributions::Alphanumeric};
use sha2::{Digest, Sha256};
use sqlx::{FromRow, SqlitePool};

const API_TOKEN_LEN: usize = 48;

#[derive(Debug, Clone, FromRow)]
pub struct ChildBotRecord {
    pub id: i64,
    pub owner_user_id: i64,
    pub bot_username: Option<String>,
    pub shop_name: Option<String>,
    pub token: String,
    pub token_hash: String,
    pub is_active: i64,
    pub created_at: String,
    pub updated_at: String,
}

pub fn hash_api_token(token: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(token.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn generate_api_token() -> String {
    rand::thread_rng()
        .sample_iter(&Alphanumeric)
        .take(API_TOKEN_LEN)
        .map(char::from)
        .collect()
}

pub async fn ensure_schema(pool: &SqlitePool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS child_bots (
            id INTEGER PRIMARY KEY AUTOINCREMENT,
            owner_user_id INTEGER NOT NULL,
            bot_username TEXT,
            shop_name TEXT,
            token TEXT NOT NULL,
            token_hash TEXT NOT NULL UNIQUE,
            is_active INTEGER NOT NULL DEFAULT 1,
            created_at TEXT NOT NULL DEFAULT (datetime('now')),
            updated_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    sqlx::query(
        "CREATE INDEX IF NOT EXISTS idx_child_bots_owner ON child_bots(owner_user_id)",
    )
    .execute(pool)
    .await?;

    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS child_bot_orders (
            order_id TEXT PRIMARY KEY,
            child_bot_id INTEGER NOT NULL,
            affiliate_user_id INTEGER NOT NULL,
            buyer_user_id INTEGER NOT NULL,
            created_at TEXT NOT NULL DEFAULT (datetime('now'))
        )"#,
    )
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn create_child_bot(
    pool: &SqlitePool,
    owner_user_id: i64,
    bot_username: Option<&str>,
    shop_name: Option<&str>,
) -> Result<(ChildBotRecord, String)> {
    ensure_schema(pool).await?;
    let token = generate_api_token();
    let token_hash = hash_api_token(&token);
    sqlx::query(
        r#"INSERT INTO child_bots
        (owner_user_id, bot_username, shop_name, token, token_hash, is_active, created_at, updated_at)
        VALUES (?, ?, ?, ?, ?, 1, datetime('now'), datetime('now'))"#,
    )
    .bind(owner_user_id)
    .bind(normalize_optional(bot_username))
    .bind(normalize_optional(shop_name))
    .bind(&token)
    .bind(&token_hash)
    .execute(pool)
    .await?;

    let record = sqlx::query_as::<_, ChildBotRecord>(
        r#"SELECT id, owner_user_id, bot_username, shop_name, token, token_hash, is_active, created_at, updated_at
        FROM child_bots
        WHERE token_hash = ?"#,
    )
    .bind(&token_hash)
    .fetch_one(pool)
    .await?;
    Ok((record, token))
}

pub async fn verify_api_key(pool: &SqlitePool, token: &str) -> Result<Option<ChildBotRecord>> {
    ensure_schema(pool).await?;
    let token_hash = hash_api_token(token.trim());
    let record = sqlx::query_as::<_, ChildBotRecord>(
        r#"SELECT id, owner_user_id, bot_username, shop_name, token, token_hash, is_active, created_at, updated_at
        FROM child_bots
        WHERE token_hash = ? AND is_active = 1"#,
    )
    .bind(token_hash)
    .fetch_optional(pool)
    .await?;
    Ok(record)
}

pub async fn list_child_bots(pool: &SqlitePool, limit: i64) -> Result<Vec<ChildBotRecord>> {
    ensure_schema(pool).await?;
    let rows = sqlx::query_as::<_, ChildBotRecord>(
        r#"SELECT id, owner_user_id, bot_username, shop_name, token, token_hash, is_active, created_at, updated_at
        FROM child_bots
        ORDER BY id DESC
        LIMIT ?"#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn insert_child_bot_order(
    pool: &SqlitePool,
    order_id: &str,
    child_bot_id: i64,
    affiliate_user_id: i64,
    buyer_user_id: i64,
) -> Result<()> {
    ensure_schema(pool).await?;
    sqlx::query(
        r#"INSERT OR IGNORE INTO child_bot_orders
        (order_id, child_bot_id, affiliate_user_id, buyer_user_id)
        VALUES (?, ?, ?, ?)"#,
    )
    .bind(order_id)
    .bind(child_bot_id)
    .bind(affiliate_user_id)
    .bind(buyer_user_id)
    .execute(pool)
    .await?;
    Ok(())
}

fn normalize_optional(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(str::to_string)
}
