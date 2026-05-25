use anyhow::Result;
use serde_json::Value;
use sqlx::{FromRow, SqlitePool};

use crate::domains::chat::models::{ChatMessage, Conversation};

#[derive(Debug, FromRow)]
struct CountRow {
    count: i64,
}

pub async fn list_conversations(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    query: Option<&str>,
) -> Result<Vec<Conversation>> {
    let q = query
        .map(|v| format!("%{}%", v.trim().to_lowercase()))
        .filter(|v| !v.is_empty());

    let rows = sqlx::query_as::<_, Conversation>(
        r#"
        WITH last_activity AS (
            SELECT chat_id, MAX(created_at) AS last_activity_at
            FROM telegram_chat_messages
            GROUP BY chat_id
        )
        SELECT
            s.user_id,
            s.chat_id,
            s.username,
            s.full_name,
            s.first_name,
            s.last_name,
            s.updated_at,
            COALESCE(la.last_activity_at, s.updated_at, s.created_at) AS last_activity_at
        FROM subscribers s
        LEFT JOIN last_activity la ON la.chat_id = s.chat_id
        WHERE (?1 IS NULL
            OR CAST(s.user_id AS TEXT) LIKE ?1
            OR CAST(s.chat_id AS TEXT) LIKE ?1
            OR LOWER(COALESCE(s.username, '')) LIKE ?1
            OR LOWER(COALESCE(s.full_name, '')) LIKE ?1
            OR LOWER(COALESCE(s.first_name, '')) LIKE ?1
            OR LOWER(COALESCE(s.last_name, '')) LIKE ?1)
        ORDER BY datetime(COALESCE(la.last_activity_at, s.updated_at, s.created_at)) DESC, s.user_id DESC
        LIMIT ?2 OFFSET ?3
        "#,
    )
    .bind(q.clone())
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn count_conversations(pool: &SqlitePool, query: Option<&str>) -> Result<i64> {
    let q = query
        .map(|v| format!("%{}%", v.trim().to_lowercase()))
        .filter(|v| !v.is_empty());

    let row = sqlx::query_as::<_, CountRow>(
        r#"
        SELECT COUNT(*) AS count
        FROM subscribers s
        WHERE (?1 IS NULL
            OR CAST(s.user_id AS TEXT) LIKE ?1
            OR CAST(s.chat_id AS TEXT) LIKE ?1
            OR LOWER(COALESCE(s.username, '')) LIKE ?1
            OR LOWER(COALESCE(s.full_name, '')) LIKE ?1
            OR LOWER(COALESCE(s.first_name, '')) LIKE ?1
            OR LOWER(COALESCE(s.last_name, '')) LIKE ?1)
        "#,
    )
    .bind(q)
    .fetch_one(pool)
    .await?;

    Ok(row.count)
}

pub async fn list_messages(
    pool: &SqlitePool,
    chat_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<ChatMessage>> {
    let rows = sqlx::query_as::<_, ChatMessage>(
        r#"
        SELECT id, chat_id, user_id, direction, text, telegram_message_id, telegram_date, raw_json, created_at
        FROM telegram_chat_messages
        WHERE chat_id = ?1
        ORDER BY datetime(created_at) DESC, id DESC
        LIMIT ?2 OFFSET ?3
        "#,
    )
    .bind(chat_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(rows)
}

pub async fn count_messages(pool: &SqlitePool, chat_id: i64) -> Result<i64> {
    let row = sqlx::query_as::<_, CountRow>(
        "SELECT COUNT(*) AS count FROM telegram_chat_messages WHERE chat_id = ?1",
    )
    .bind(chat_id)
    .fetch_one(pool)
    .await?;

    Ok(row.count)
}

pub async fn insert_message(
    pool: &SqlitePool,
    chat_id: i64,
    user_id: Option<i64>,
    direction: &str,
    text: Option<&str>,
    telegram_message_id: Option<i64>,
    telegram_date: Option<&str>,
    raw_json: Option<&Value>,
) -> Result<()> {
    let raw_json_str = raw_json.map(|v| v.to_string());

    sqlx::query(
        r#"
        INSERT INTO telegram_chat_messages (
            chat_id, user_id, direction, text, telegram_message_id, telegram_date, raw_json
        ) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
        "#,
    )
    .bind(chat_id)
    .bind(user_id)
    .bind(direction)
    .bind(text)
    .bind(telegram_message_id)
    .bind(telegram_date)
    .bind(raw_json_str)
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_update_log(
    pool: &SqlitePool,
    chat_id: Option<i64>,
    user_id: Option<i64>,
    update_type: &str,
    raw_json: &Value,
) -> Result<()> {
    sqlx::query(
        r#"
        INSERT INTO telegram_update_logs (chat_id, user_id, update_type, raw_json)
        VALUES (?1, ?2, ?3, ?4)
        "#,
    )
    .bind(chat_id)
    .bind(user_id)
    .bind(update_type)
    .bind(raw_json.to_string())
    .execute(pool)
    .await?;

    Ok(())
}
