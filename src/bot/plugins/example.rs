use std::sync::Arc;
use teloxide::prelude::*;

use crate::{
    app::AppContext,
    bot::{i18n, plugins::AppPlugin},
};

pub struct ExamplePlugin;

#[async_trait::async_trait]
impl AppPlugin for ExamplePlugin {
    fn name(&self) -> &'static str {
        "Example Plugin"
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: crate::bot::BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        if let Some(text) = msg.text()
            && text == "/ping"
        {
            let lang = if let Some(user) = msg.from() {
                i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
            } else {
                "en".to_string()
            };
            let text = i18n::t(&ctx, &lang, "ping_pong", "pong from plugin! 🏓");
            let _ = ctx.bot.send_message(msg.chat.id, text).await;
            return Ok(true);
        }
        Ok(false)
    }
}
