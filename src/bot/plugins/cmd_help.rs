use std::sync::Arc;
use teloxide::types::{BotCommand, Message};

use crate::app::AppContext;
use crate::bot::BotDialogue;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::domains::users::repo as users_repo;

pub struct HelpCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for HelpCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdHelp"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "help".to_string(),
            description: "Help".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("");

        if text.starts_with("/help") {
            let lang = if let Some(user) = msg.from() {
                let preferred = users_repo::preferred_language(&ctx.pool, user.id.0 as i64)
                    .await
                    .ok()
                    .flatten()
                    .or_else(|| user.language_code.clone());
                ctx.normalize_language_code(preferred.as_deref())
            } else {
                ctx.normalize_language_code(None)
            };
            let msg_text = ctx.get_text_lang(
                "help",
                &lang,
                "❓ Quick help:\n/shop - products\n/orders - your orders\n/help - help.",
            );
            i18n::send_message_for_key(&ctx, msg.chat.id, "help", msg_text).await?;
            return Ok(true);
        }

        Ok(false)
    }
}
