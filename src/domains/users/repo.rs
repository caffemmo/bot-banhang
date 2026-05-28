use crate::domains::users::models::Subscriber;
use anyhow::Result;
use sqlx::SqlitePool;

pub async fn upsert_subscriber(pool: &SqlitePool, profile: &Subscriber) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO subscribers (
            user_id, chat_id, username, first_name, last_name, full_name, language_code, preferred_language, stock_notifications_enabled, is_bot
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET
            chat_id = excluded.chat_id,
            username = excluded.username,
            first_name = excluded.first_name,
            last_name = excluded.last_name,
            full_name = excluded.full_name,
            language_code = excluded.language_code,
            preferred_language = coalesce(subscribers.preferred_language, excluded.preferred_language),
            stock_notifications_enabled = coalesce(subscribers.stock_notifications_enabled, excluded.stock_notifications_enabled),
            is_bot = excluded.is_bot,
            updated_at = datetime('now')"#,
    )
    .bind(profile.user_id)
    .bind(profile.chat_id)
    .bind(&profile.username)
    .bind(&profile.first_name)
    .bind(&profile.last_name)
    .bind(&profile.full_name)
    .bind(&profile.language_code)
    .bind(&profile.preferred_language)
    .bind(profile.stock_notifications_enabled.unwrap_or(1))
    .bind(profile.is_bot)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn update_preferred_language(
    pool: &SqlitePool,
    user_id: i64,
    preferred_language: &str,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE subscribers
        SET preferred_language = ?, updated_at = datetime('now')
        WHERE user_id = ?"#,
    )
    .bind(preferred_language)
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn preferred_language(pool: &SqlitePool, user_id: i64) -> Result<Option<String>> {
    let lang = sqlx::query_scalar::<_, Option<String>>(
        "SELECT preferred_language FROM subscribers WHERE user_id = ?",
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(lang.flatten())
}

pub async fn get_subscriber_by_user_id(
    pool: &SqlitePool,
    user_id: i64,
) -> Result<Option<Subscriber>> {
    let sub = sqlx::query_as::<sqlx::Sqlite, Subscriber>(
        r#"SELECT user_id, chat_id, username, first_name, last_name, full_name, language_code, preferred_language, stock_notifications_enabled, is_bot, created_at, updated_at
        FROM subscribers
        WHERE user_id = ?"#,
    )
    .bind(user_id)
    .fetch_optional(pool)
    .await?;
    Ok(sub)
}

pub async fn list_subscribers(pool: &SqlitePool) -> Result<Vec<Subscriber>> {
    let subs = sqlx::query_as::<sqlx::Sqlite, Subscriber>(
        r#"SELECT user_id, chat_id, username, first_name, last_name, full_name, language_code, preferred_language, stock_notifications_enabled, is_bot, created_at, updated_at
        FROM subscribers
        ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(subs)
}

pub async fn list_stock_notification_subscribers(pool: &SqlitePool) -> Result<Vec<Subscriber>> {
    let subs = sqlx::query_as::<sqlx::Sqlite, Subscriber>(
        r#"SELECT user_id, chat_id, username, first_name, last_name, full_name, language_code, preferred_language, stock_notifications_enabled, is_bot, created_at, updated_at
        FROM subscribers
        WHERE IFNULL(stock_notifications_enabled, 1) = 1
        ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(subs)
}

pub async fn update_stock_notifications_enabled(
    pool: &SqlitePool,
    user_id: i64,
    enabled: bool,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE subscribers
        SET stock_notifications_enabled = ?, updated_at = datetime('now')
        WHERE user_id = ?"#,
    )
    .bind(if enabled { 1 } else { 0 })
    .bind(user_id)
    .execute(pool)
    .await?;
    Ok(())
}
