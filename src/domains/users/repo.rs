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
        ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(subs)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    #[tokio::test]
    async fn stock_notification_recipients_include_legacy_opted_out_subscribers() {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();

        let user = Subscriber {
            user_id: 101,
            chat_id: 202,
            username: Some("khqcs".to_string()),
            first_name: None,
            last_name: None,
            full_name: None,
            language_code: Some("vi".to_string()),
            preferred_language: None,
            stock_notifications_enabled: Some(0),
            is_bot: Some(0),
            created_at: None,
            updated_at: None,
        };
        upsert_subscriber(&pool, &user).await.unwrap();

        let recipients = list_stock_notification_subscribers(&pool).await.unwrap();

        assert_eq!(recipients.len(), 1);
        assert_eq!(recipients[0].username.as_deref(), Some("khqcs"));
    }
}
