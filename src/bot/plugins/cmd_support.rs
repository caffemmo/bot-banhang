use std::sync::Arc;

use anyhow::Result;
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::requests::Requester;
use teloxide::types::{BotCommand, CallbackQuery, Message};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::domains::support::models::support_kind_label;
use crate::domains::support::repo;

pub struct SupportCommandPlugin;

fn is_support_admin(ctx: &AppContext, user_id: i64) -> bool {
    ctx.is_telegram_icon_admin(user_id)
        || ctx
            .order_notification_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
}

async fn save_admin_reply(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    ticket_id: i64,
    text: &str,
) -> Result<()> {
    let Some(ticket) = repo::get_ticket(&ctx.pool, ticket_id).await? else {
        ctx.bot
            .send_message(chat_id, format!("❌ Không tìm thấy ticket #{ticket_id}."))
            .await?;
        return Ok(());
    };
    if ticket.status == "closed" {
        ctx.bot
            .send_message(chat_id, format!("⚠️ Ticket #{ticket_id} đã đóng."))
            .await?;
        return Ok(());
    }
    repo::add_message(&ctx.pool, ticket_id, "admin", text).await?;
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Đã trả lời ticket #{}.\nLoại: {}\nKhách mở web ticket là thấy phản hồi.",
                ticket.id,
                support_kind_label(&ticket.kind),
            ),
        )
        .await?;
    Ok(())
}

async fn close_ticket(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    ticket_id: i64,
) -> Result<()> {
    let Some(ticket) = repo::get_ticket(&ctx.pool, ticket_id).await? else {
        ctx.bot
            .send_message(chat_id, format!("❌ Không tìm thấy ticket #{ticket_id}."))
            .await?;
        return Ok(());
    };
    repo::close_ticket(&ctx.pool, ticket_id).await?;
    ctx.bot
        .send_message(
            chat_id,
            format!(
                "✅ Đã đóng ticket #{}.\nLoại: {}",
                ticket.id,
                support_kind_label(&ticket.kind),
            ),
        )
        .await?;
    Ok(())
}

fn parse_id_and_text(text: &str, command: &str) -> Option<(i64, String)> {
    let rest = text.strip_prefix(command)?.trim();
    let mut parts = rest.splitn(2, char::is_whitespace);
    let id = parts.next()?.trim().parse::<i64>().ok()?;
    let message = parts.next().unwrap_or("").trim().to_string();
    Some((id, message))
}

#[async_trait::async_trait]
impl AppPlugin for SupportCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdSupport"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let user_id = msg.from().map(|user| user.id.0 as i64).unwrap_or(0);
        if !is_support_admin(&ctx, user_id) {
            return Ok(false);
        }

        let text = msg.text().unwrap_or("").trim();
        let state = dialogue.get().await?.unwrap_or_default();
        if let State::SupportReply { ticket_id } = state {
            if text.is_empty() {
                ctx.bot
                    .send_message(msg.chat.id, "Nhập nội dung trả lời ticket.")
                    .await?;
                return Ok(true);
            }
            save_admin_reply(&ctx, msg.chat.id, ticket_id, text).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        if let Some((ticket_id, reply)) = parse_id_and_text(text, "/supportreply") {
            if reply.is_empty() {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        "Cách dùng: /supportreply <ticket_id> <nội dung>",
                    )
                    .await?;
                return Ok(true);
            }
            save_admin_reply(&ctx, msg.chat.id, ticket_id, &reply).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        if let Some((ticket_id, _)) = parse_id_and_text(text, "/supportclose") {
            close_ticket(&ctx, msg.chat.id, ticket_id).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        Ok(false)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let Some(data) = q.data.as_deref() else {
            return Ok(false);
        };
        if !data.starts_with("support:") {
            return Ok(false);
        }
        let user_id = q.from.id.0 as i64;
        if !is_support_admin(&ctx, user_id) {
            let _ = ctx
                .bot
                .answer_callback_query(q.id)
                .text("Bạn không có quyền xử lý ticket.")
                .await;
            return Ok(true);
        }

        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else {
            return Ok(true);
        };
        let chat_id = msg.chat().id;

        if let Some(id) = data.strip_prefix("support:reply:") {
            let ticket_id = id.parse::<i64>().unwrap_or(0);
            if repo::get_ticket(&ctx.pool, ticket_id).await?.is_none() {
                ctx.bot
                    .send_message(chat_id, format!("❌ Không tìm thấy ticket #{ticket_id}."))
                    .await?;
                return Ok(true);
            }
            dialogue.update(State::SupportReply { ticket_id }).await?;
            ctx.bot
                .send_message(
                    chat_id,
                    format!("✍️ Nhập nội dung trả lời ticket #{ticket_id}."),
                )
                .await?;
            return Ok(true);
        }

        if let Some(id) = data.strip_prefix("support:close:") {
            let ticket_id = id.parse::<i64>().unwrap_or(0);
            close_ticket(&ctx, chat_id, ticket_id).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        Ok(true)
    }
}
