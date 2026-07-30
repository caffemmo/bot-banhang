use std::sync::Arc;

use chrono::Utc;
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, ParseMode,
};

use crate::app::AppContext;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::core::totp::current_totp_code;
use crate::domains::orders::api::html_escape;

pub const TOTP_CALLBACK: &str = "totp:get";

pub struct TotpCommandPlugin;

pub async fn prompt_totp_secret(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    dialogue: BotDialogue,
) -> anyhow::Result<()> {
    dialogue.update(State::TotpInput).await?;
    ctx.bot
        .send_message(
            chat_id,
            "🔐 Gửi secret 2FA để bot lấy mã 6 số hiện tại.\n\nVí dụ:\n<code>VE7YPIHKWN4H2HMHWSQNT4QERLN4PP65</code>\n\nGửi /cancel để hủy.",
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(totp_keyboard())
        .await?;
    Ok(())
}

fn totp_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "🔐 Lấy mã 2FA",
            TOTP_CALLBACK,
        )],
        vec![InlineKeyboardButton::callback("🏠 Menu chính", "start:menu")],
    ])
}

async fn handle_totp_input(
    ctx: &Arc<AppContext>,
    msg: &Message,
    dialogue: BotDialogue,
) -> anyhow::Result<bool> {
    let Some(raw_input) = msg.text().map(str::trim).filter(|value| !value.is_empty()) else {
        ctx.bot
            .send_message(msg.chat.id, "Vui lòng gửi secret 2FA.")
            .reply_markup(totp_keyboard())
            .await?;
        return Ok(true);
    };

    if raw_input.eq_ignore_ascii_case("/cancel") {
        dialogue.update(State::Idle).await?;
        ctx.bot
            .send_message(msg.chat.id, "Đã hủy lấy mã 2FA.")
            .reply_markup(totp_keyboard())
            .await?;
        return Ok(true);
    }
    if raw_input.starts_with('/') {
        dialogue.update(State::Idle).await?;
        return Ok(false);
    }

    let _ = ctx.bot.delete_message(msg.chat.id, msg.id).await;
    let secret = normalize_secret(raw_input);
    let now = Utc::now().timestamp().max(0) as u64;
    let Some(code) = current_totp_code(&secret, now) else {
        ctx.bot
            .send_message(
                msg.chat.id,
                "❌ Secret 2FA không hợp lệ. Hãy gửi secret dạng Base32, ví dụ:\n<code>VE7YPIHKWN4H2HMHWSQNT4QERLN4PP65</code>",
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(totp_keyboard())
            .await?;
        return Ok(true);
    };

    ctx.bot
        .send_message(
            msg.chat.id,
            format!(
                "🔐 Mã 2FA hiện tại:\n<code>{}</code>\n\n⏱ Mã đổi sau khoảng {} giây.",
                html_escape(&code.code),
                code.seconds_remaining
            ),
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(totp_keyboard())
        .await?;
    Ok(true)
}

fn normalize_secret(value: &str) -> String {
    value
        .trim()
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace() && *ch != '-')
        .collect::<String>()
}

#[async_trait::async_trait]
impl AppPlugin for TotpCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdTotp"
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
        if !matches!(dialogue.get().await?, Some(State::TotpInput)) {
            return Ok(false);
        }
        handle_totp_input(&ctx, &msg, dialogue).await
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        if q.data.as_deref() != Some(TOTP_CALLBACK) {
            return Ok(false);
        }
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(message) = q.message else {
            return Ok(true);
        };
        prompt_totp_secret(&ctx, message.chat().id, dialogue).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_secret_spaces_and_dashes() {
        assert_eq!(
            normalize_secret(" VE7Y PIHK-WN4H "),
            "VE7YPIHKWN4H".to_string()
        );
    }

    #[test]
    fn keyboard_keeps_totp_and_home_actions() {
        let keyboard = totp_keyboard();
        let json = serde_json::to_value(keyboard).unwrap();

        assert_eq!(json["inline_keyboard"][0][0]["callback_data"], TOTP_CALLBACK);
        assert_eq!(json["inline_keyboard"][1][0]["callback_data"], "start:menu");
    }
}
