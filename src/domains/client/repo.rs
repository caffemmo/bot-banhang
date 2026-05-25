use anyhow::Result;
use rand::{Rng, distributions::Alphanumeric};
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

const API_TOKEN_LEN: usize = 48;

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

pub async fn create_or_replace_api_key(pool: &SqlitePool, chat_id: i64) -> Result<String> {
    let token = generate_api_token();
    let token_hash = hash_api_token(&token);
    sqlx::query(
        r#"INSERT INTO client_api_keys (chat_id, token, token_hash, created_at, updated_at)
        VALUES (?, ?, ?, datetime('now'), datetime('now'))
        ON CONFLICT(chat_id) DO UPDATE SET
            token = excluded.token,
            token_hash = excluded.token_hash,
            updated_at = excluded.updated_at"#,
    )
    .bind(chat_id)
    .bind(&token)
    .bind(token_hash)
    .execute(pool)
    .await?;
    Ok(token)
}

pub async fn get_api_key(pool: &SqlitePool, chat_id: i64) -> Result<Option<String>> {
    let token = sqlx::query_scalar("SELECT token FROM client_api_keys WHERE chat_id = ?")
        .bind(chat_id)
        .fetch_optional(pool)
        .await?;
    Ok(token.filter(|value: &String| !value.trim().is_empty()))
}

pub async fn get_or_create_api_key(pool: &SqlitePool, chat_id: i64) -> Result<String> {
    if let Some(token) = get_api_key(pool, chat_id).await? {
        return Ok(token);
    }
    create_or_replace_api_key(pool, chat_id).await
}

pub async fn verify_api_key(pool: &SqlitePool, chat_id: i64, token: &str) -> Result<bool> {
    let Some(stored_hash): Option<String> =
        sqlx::query_scalar("SELECT token_hash FROM client_api_keys WHERE chat_id = ?")
            .bind(chat_id)
            .fetch_optional(pool)
            .await?
    else {
        return Ok(false);
    };

    Ok(stored_hash == hash_api_token(token))
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
    async fn create_or_replace_api_key_invalidates_previous_token() {
        let pool = test_pool().await;

        let first = create_or_replace_api_key(&pool, 42).await.unwrap();
        let second = create_or_replace_api_key(&pool, 42).await.unwrap();

        assert_ne!(first, second);
        assert!(!verify_api_key(&pool, 42, &first).await.unwrap());
        assert!(verify_api_key(&pool, 42, &second).await.unwrap());
    }

    #[tokio::test]
    async fn get_api_key_returns_current_plaintext_token_for_display() {
        let pool = test_pool().await;

        assert_eq!(get_api_key(&pool, 42).await.unwrap(), None);
        let token = create_or_replace_api_key(&pool, 42).await.unwrap();

        assert_eq!(
            get_api_key(&pool, 42).await.unwrap().as_deref(),
            Some(token.as_str())
        );
    }
}
