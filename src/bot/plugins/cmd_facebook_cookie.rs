use std::sync::Arc;

use teloxide::payloads::{
    AnswerCallbackQuerySetters, EditMessageTextSetters, SendDocumentSetters, SendMessageSetters,
};
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, Message,
    ParseMode,
};

use crate::app::AppContext;
use crate::bot::facebook_cookie::{FacebookCookieInput, get_live_cookie};
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::domains::orders::api::html_escape;

pub const FACEBOOK_COOKIE_CALLBACK: &str = "facebook_cookie:get";

pub struct FacebookCookieCommandPlugin;

pub async fn prompt_facebook_cookie(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    dialogue: BotDialogue,
) -> anyhow::Result<()> {
    dialogue.update(State::FacebookCookieInput).await?;
    ctx.bot
        .send_message(
            chat_id,
            "🔐 Gửi một trong các định dạng sau:\n\n\
             <code>UID|PASS|2FA|COOKIE</code>\n\
             <code>UID|PASS|2FA</code>\n\
             <code>COOKIE</code>\n\n\
             2FA có thể là secret hoặc mã 6 số hiện tại. Bot sẽ kiểm tra/đăng nhập và trả cookie Facebook live mới nhất.",
        )
        .parse_mode(ParseMode::Html)
        .reply_markup(facebook_cookie_keyboard())
        .await?;
    Ok(())
}

fn facebook_cookie_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback(
            "🍪 Lấy cookie",
            FACEBOOK_COOKIE_CALLBACK,
        )],
        vec![InlineKeyboardButton::callback(
            "🏠 Menu chính",
            "start:menu",
        )],
    ])
}

fn facebook_cookie_proxy_url(ctx: &AppContext) -> Option<String> {
    [
        ctx.get_text("facebook_cookie_proxy_url", ""),
        std::env::var("FACEBOOK_COOKIE_PROXY_URL").unwrap_or_default(),
    ]
    .into_iter()
    .map(|value| value.trim().to_string())
    .find(|value| !value.is_empty())
}

async fn handle_facebook_cookie_input(
    ctx: &Arc<AppContext>,
    msg: &Message,
    dialogue: BotDialogue,
) -> anyhow::Result<bool> {
    let Some(raw_input) = msg.text().map(str::trim).filter(|value| !value.is_empty()) else {
        ctx.bot
            .send_message(
                msg.chat.id,
                "Vui lòng gửi UID|PASS|2FA|COOKIE, UID|PASS|2FA hoặc COOKIE.",
            )
            .reply_markup(facebook_cookie_keyboard())
            .await?;
        return Ok(true);
    };

    if raw_input.eq_ignore_ascii_case("/cancel") {
        dialogue.update(State::Idle).await?;
        ctx.bot
            .send_message(msg.chat.id, "Đã hủy lấy cookie.")
            .reply_markup(facebook_cookie_keyboard())
            .await?;
        return Ok(true);
    }
    if raw_input.starts_with('/') {
        dialogue.update(State::Idle).await?;
        return Ok(false);
    }

    let _ = ctx.bot.delete_message(msg.chat.id, msg.id).await;
    let input = match FacebookCookieInput::parse(raw_input) {
        Ok(input) => input,
        Err(err) => {
            ctx.bot
                .send_message(msg.chat.id, format!("❌ {err}"))
                .reply_markup(facebook_cookie_keyboard())
                .await?;
            return Ok(true);
        }
    };

    let progress = ctx
        .bot
        .send_message(
            msg.chat.id,
            "⏳ Đang kiểm tra tài khoản và lấy cookie Facebook live mới nhất...",
        )
        .await?;
    let proxy_url = facebook_cookie_proxy_url(ctx);

    match get_live_cookie(&input, proxy_url.as_deref()).await {
        Ok(cookie) => {
            dialogue.update(State::Idle).await?;
            let _ = ctx.bot.delete_message(msg.chat.id, progress.id).await;
            send_facebook_cookie_result(ctx, msg.chat.id, &cookie).await?;
        }
        Err(err) => {
            ctx.bot
                .edit_message_text(
                    msg.chat.id,
                    progress.id,
                    format!(
                        "❌ Không lấy được cookie: {err:#}\n\nBạn có thể gửi lại thông tin để thử lần nữa."
                    ),
                )
                .reply_markup(facebook_cookie_keyboard())
                .await?;
        }
    }

    Ok(true)
}

async fn send_facebook_cookie_result(
    ctx: &Arc<AppContext>,
    chat_id: teloxide::types::ChatId,
    cookie: &str,
) -> anyhow::Result<()> {
    if cookie.chars().count() <= 3400 {
        ctx.bot
            .send_message(
                chat_id,
                format!(
                    "🍪 <b>Cookie Facebook live mới nhất</b>\n\n<pre>{}</pre>",
                    html_escape(cookie)
                ),
            )
            .parse_mode(ParseMode::Html)
            .reply_markup(facebook_cookie_keyboard())
            .await?;
    } else {
        ctx.bot
            .send_document(
                chat_id,
                InputFile::memory(cookie.as_bytes().to_vec()).file_name("facebook_cookie.txt"),
            )
            .caption("🍪 Cookie Facebook live mới nhất được gửi trong file.")
            .reply_markup(facebook_cookie_keyboard())
            .await?;
    }
    Ok(())
}

#[async_trait::async_trait]
impl AppPlugin for FacebookCookieCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdFacebookCookie"
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
        if !matches!(dialogue.get().await?, Some(State::FacebookCookieInput)) {
            return Ok(false);
        }
        handle_facebook_cookie_input(&ctx, &msg, dialogue).await
    }

    async fn handle_callback(
        &self,
        ctx: Arc<AppContext>,
        q: CallbackQuery,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        if q.data.as_deref() != Some(FACEBOOK_COOKIE_CALLBACK) {
            return Ok(false);
        }
        let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
        let Some(message) = q.message else {
            return Ok(true);
        };
        prompt_facebook_cookie(&ctx, message.chat().id, dialogue).await?;
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn result_keyboard_keeps_cookie_and_home_actions() {
        let keyboard = facebook_cookie_keyboard();
        let json = serde_json::to_value(keyboard).unwrap();
        assert_eq!(
            json["inline_keyboard"][0][0]["callback_data"],
            FACEBOOK_COOKIE_CALLBACK
        );
        assert_eq!(json["inline_keyboard"][1][0]["callback_data"], "start:menu");
    }
}
