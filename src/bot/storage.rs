use std::sync::Arc;

use futures::future::BoxFuture;
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use teloxide::dispatching::dialogue::Storage;
use teloxide::types::ChatId;

pub struct SqliteDialogueStorage {
    pool: SqlitePool,
}

impl SqliteDialogueStorage {
    pub fn new(pool: SqlitePool) -> Arc<Self> {
        Arc::new(Self { pool })
    }
}

#[derive(Debug, thiserror::Error)]
pub enum StorageError {
    #[error("Database error: {0}")]
    Db(#[from] sqlx::Error),
    #[error("Serialization error: {0}")]
    Serde(#[from] serde_json::Error),
}

impl<D> Storage<D> for SqliteDialogueStorage
where
    D: Send + 'static + Serialize + for<'de> Deserialize<'de>,
{
    type Error = StorageError;

    fn remove_dialogue(
        self: Arc<Self>,
        chat_id: ChatId,
    ) -> BoxFuture<'static, Result<(), Self::Error>> {
        Box::pin(async move {
            let id = chat_id.0;
            sqlx::query("DELETE FROM dialogue_states WHERE chat_id = ?")
                .bind(id)
                .execute(&self.pool)
                .await?;
            Ok(())
        })
    }

    fn update_dialogue(
        self: Arc<Self>,
        chat_id: ChatId,
        dialogue: D,
    ) -> BoxFuture<'static, Result<(), Self::Error>> {
        Box::pin(async move {
            let id = chat_id.0;
            let state_json = serde_json::to_string(&dialogue)?;
            sqlx::query(
                "INSERT INTO dialogue_states (chat_id, state_json) VALUES (?, ?)
                 ON CONFLICT(chat_id) DO UPDATE SET state_json = excluded.state_json",
            )
            .bind(id)
            .bind(state_json)
            .execute(&self.pool)
            .await?;
            Ok(())
        })
    }

    fn get_dialogue(
        self: Arc<Self>,
        chat_id: ChatId,
    ) -> BoxFuture<'static, Result<Option<D>, Self::Error>> {
        Box::pin(async move {
            let id = chat_id.0;
            let row: Option<(String,)> =
                sqlx::query_as("SELECT state_json FROM dialogue_states WHERE chat_id = ?")
                    .bind(id)
                    .fetch_optional(&self.pool)
                    .await?;
            match row {
                Some((json,)) => Ok(Some(serde_json::from_str(&json)?)),
                None => Ok(None),
            }
        })
    }
}
