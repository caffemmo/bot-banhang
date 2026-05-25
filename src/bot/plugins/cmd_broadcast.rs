use std::sync::Arc;

use anyhow::Result;
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message,
};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, i18n};
use crate::domains::users::broadcast as broadcast_domain;

pub struct BroadcastCommandPlugin;

fn is_broadcast_command(text: &str) -> bool {
    let text = text.trim();
    text == "/broadcast" || text.starts_with("/broadcast@")
}

fn is_broadcast_callback(text: &str) -> bool {
    text.starts_with("bctpl:")
}

fn broadcast_templates_keyboard(
    templates: &[broadcast_domain::BroadcastTemplate],
) -> InlineKeyboardMarkup {
    let rows = templates
        .iter()
        .map(|template| {
            vec![InlineKeyboardButton::callback(
                format!("{} gửi: {}", template.sort_order, template.name),
                format!("bctpl:{}", template.id),
            )]
        })
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(rows)
}

#[async_trait::async_trait]
impl AppPlugin for BroadcastCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdBroadcast"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "broadcast".to_string(),
            description: "Admin broadcast templates".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("");
        if !is_broadcast_command(text) {
            return Ok(false);
        }

        let Some(user) = msg.from.as_ref() else {
            return Ok(true);
        };
        if !ctx.is_telegram_icon_admin(user.id.0 as i64) {
            let lang = i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await;
            ctx.bot
                .send_message(
                    msg.chat.id,
                    i18n::t(&ctx, &lang, "unauthorized", "Unauthorized."),
                )
                .await?;
            return Ok(true);
        }

        let templates = broadcast_domain::list_broadcast_templates(&ctx.pool).await?;
        if templates.is_empty() {
            ctx.bot
                .send_message(msg.chat.id, "Chưa có mẫu thông báo nào.")
                .await?;
            return Ok(true);
        }

        ctx.bot
            .send_message(msg.chat.id, "Chọn mẫu thông báo để gửi ngay:")
            .reply_markup(broadcast_templates_keyboard(&templates))
            .await?;
        Ok(true)
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = q.data.clone().unwrap_or_default();
        if !is_broadcast_callback(&text) {
            return Ok(false);
        }

        if !ctx.is_telegram_icon_admin(q.from.id.0 as i64) {
            let _ = ctx
                .bot
                .answer_callback_query(q.id.clone())
                .text("Unauthorized")
                .await;
            return Ok(true);
        }

        let template_id = text
            .strip_prefix("bctpl:")
            .and_then(|value| value.parse::<i64>().ok())
            .unwrap_or(0);
        let total = broadcast_domain::enqueue_broadcast_template(ctx.clone(), template_id).await?;
        let _ = ctx
            .bot
            .answer_callback_query(q.id.clone())
            .text("Đã đưa vào hàng gửi")
            .await;
        if let Some(msg) = &q.message {
            ctx.bot
                .send_message(
                    msg.chat().id,
                    format!("Đã đưa mẫu #{template_id} vào hàng gửi cho {total} user."),
                )
                .await?;
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn broadcast_command_accepts_plain_and_bot_username() {
        assert!(is_broadcast_command("/broadcast"));
        assert!(is_broadcast_command("/broadcast@my_bot"));
        assert!(!is_broadcast_command("/shop"));
    }

    #[test]
    fn broadcast_template_callback_is_routed_to_plugin() {
        assert!(is_broadcast_callback("bctpl:1"));
        assert!(!is_broadcast_callback("start:shop"));
    }
}
