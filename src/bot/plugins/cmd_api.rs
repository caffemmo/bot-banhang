use std::sync::Arc;

use anyhow::Result;
use teloxide::payloads::SendMessageSetters;
use teloxide::requests::Requester;
use teloxide::types::{BotCommand, Message, ParseMode};

use crate::app::AppContext;
use crate::bot::BotDialogue;
use crate::bot::plugins::AppPlugin;
use crate::domains::client::repo as client_repo;

pub struct ApiCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for ApiCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdApi"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "newapi".to_string(),
            description: "Create a new API key".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        if !is_newapi_command(text) {
            return Ok(false);
        }

        let chat_id = msg.chat.id.0;
        let token = client_repo::create_or_replace_api_key(&ctx.pool, chat_id).await?;
        let api_key = format!("{chat_id}:{token}");
        let reply = format!(
            "API key mới của bạn:\n<code>{api_key}</code>\n\nDùng header:\n<code>Authorization: Bearer {api_key}</code>\n\nEndpoints:\nGET /api/client/products\nGET /api/client/wallet\nPOST /api/client/orders"
        );
        ctx.bot
            .send_message(msg.chat.id, reply)
            .parse_mode(ParseMode::Html)
            .await?;
        Ok(true)
    }
}

fn is_newapi_command(text: &str) -> bool {
    let command = text.split_whitespace().next().unwrap_or("");
    command == "/newapi" || command.starts_with("/newapi@")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn newapi_command_accepts_plain_and_bot_username() {
        assert!(is_newapi_command("/newapi"));
        assert!(is_newapi_command("/newapi@mybot"));
        assert!(is_newapi_command("/newapi now"));
        assert!(!is_newapi_command("/newapikey"));
    }
}
