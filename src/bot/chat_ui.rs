use anyhow::Result;
use serde_json::{Value, json};
use teloxide::types::{ChatId, MessageId};

use crate::app::AppContext;
use crate::bot::i18n;

const MENU_KIND: &str = "menu";

pub async fn delete_previous_menu(ctx: &AppContext, chat_id: ChatId) {
    let Ok(Some(message_id)) = last_message_id(ctx, chat_id, MENU_KIND).await else {
        return;
    };

    let _ = i18n::send_raw_telegram_method(
        ctx,
        "deleteMessage",
        json!({
            "chat_id": chat_id.0,
            "message_id": message_id,
        }),
    )
    .await;
    let _ = forget_message(ctx, chat_id, MENU_KIND).await;
}

pub async fn delete_message(ctx: &AppContext, chat_id: ChatId, message_id: MessageId) {
    let _ = i18n::send_raw_telegram_method(
        ctx,
        "deleteMessage",
        json!({
            "chat_id": chat_id.0,
            "message_id": message_id.0,
        }),
    )
    .await;
}

pub async fn send_clean_menu(
    ctx: &AppContext,
    chat_id: ChatId,
    key: &str,
    text: impl Into<String>,
    reply_markup: Value,
) -> Result<()> {
    let payload = i18n::message_payload_with_json_keyboard(ctx, chat_id, key, text, reply_markup)?;
    send_clean_menu_payload(ctx, chat_id, payload).await
}

pub async fn send_clean_menu_payload(
    ctx: &AppContext,
    chat_id: ChatId,
    payload: Value,
) -> Result<()> {
    delete_previous_menu(ctx, chat_id).await;
    let response = i18n::send_raw_telegram_method(ctx, "sendMessage", payload).await?;
    remember_menu_from_response(ctx, chat_id, &response).await?;
    Ok(())
}

pub async fn remember_menu_message(
    ctx: &AppContext,
    chat_id: ChatId,
    message_id: i64,
) -> Result<()> {
    remember_message(ctx, chat_id, MENU_KIND, message_id).await
}

pub async fn remember_menu_from_response(
    ctx: &AppContext,
    chat_id: ChatId,
    response: &Value,
) -> Result<()> {
    if let Some(message_id) = response
        .get("result")
        .and_then(|result| result.get("message_id"))
        .and_then(Value::as_i64)
    {
        remember_menu_message(ctx, chat_id, message_id).await?;
    }
    Ok(())
}

async fn last_message_id(ctx: &AppContext, chat_id: ChatId, kind: &str) -> Result<Option<i64>> {
    let row = sqlx::query_as::<_, (i64,)>(
        "SELECT message_id FROM chat_ui_messages WHERE chat_id = ? AND kind = ?",
    )
    .bind(chat_id.0)
    .bind(kind)
    .fetch_optional(&ctx.pool)
    .await?;
    Ok(row.map(|(message_id,)| message_id))
}

async fn remember_message(
    ctx: &AppContext,
    chat_id: ChatId,
    kind: &str,
    message_id: i64,
) -> Result<()> {
    sqlx::query(
        "INSERT INTO chat_ui_messages (chat_id, kind, message_id, updated_at)
         VALUES (?, ?, ?, CURRENT_TIMESTAMP)
         ON CONFLICT(chat_id, kind) DO UPDATE SET
            message_id = excluded.message_id,
            updated_at = CURRENT_TIMESTAMP",
    )
    .bind(chat_id.0)
    .bind(kind)
    .bind(message_id)
    .execute(&ctx.pool)
    .await?;
    Ok(())
}

async fn forget_message(ctx: &AppContext, chat_id: ChatId, kind: &str) -> Result<()> {
    sqlx::query("DELETE FROM chat_ui_messages WHERE chat_id = ? AND kind = ?")
        .bind(chat_id.0)
        .bind(kind)
        .execute(&ctx.pool)
        .await?;
    Ok(())
}
