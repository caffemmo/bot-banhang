use std::sync::Arc;

use anyhow::Result;
use teloxide::dispatching::UpdateFilterExt;
use teloxide::dispatching::dialogue::Dialogue;

use crate::bot::storage::SqliteDialogueStorage;
use teloxide::dptree;
use teloxide::prelude::*;
use teloxide::types::{
    CallbackQuery, Me, Message, MessageEntity, MessageEntityKind, MessageEntityRef,
};
use tracing::info;

use crate::app::AppContext;
use crate::domains::chat::repo as chat_repo;
use crate::domains::users::repo as users_repo;

pub mod i18n;
pub mod chat_ui;
pub mod plugins;
pub mod storage;
pub mod texts;

#[derive(Clone, Default, Debug, serde::Serialize, serde::Deserialize)]
pub enum State {
    #[default]
    Idle,
    ChoosingQty {
        product_id: i64,
    },
    SelectingPlan {
        product_id: i64,
    },
    CollectingInfo {
        product_id: i64,
        qty: i64,
        plan_id: Option<i64>,
    },
    TopupEnterAmount,
    TopupUsdtEnterAmount,
    TopupBinanceEnterAmount,
    CreatingTutTitle,
    CreatingTutTeaser {
        title: String,
    },
    CreatingTutContent {
        title: String,
        teaser: String,
    },
    ViametaCollectingCookie {
        service: String,
    },
    ViametaCollectingImage {
        service: String,
        cookie: String,
    },
    FacebookUnlockIssue,
    FacebookUnlockDetails {
        issue: String,
    },
    FacebookUnlockWorkerApply,
    FacebookUnlockQuote {
        case_id: String,
    },
}

pub type BotDialogue = Dialogue<State, SqliteDialogueStorage>;

pub async fn run(ctx: Arc<AppContext>) -> Result<()> {
    let storage = SqliteDialogueStorage::new(ctx.pool.clone());

    let bot_name: Me = ctx.bot.get_me().await?;
    info!(
        "Bot started as @{}",
        bot_name.user.username.unwrap_or_default()
    );

    let plugin_message_filter = dptree::filter_async({
        let ctx = ctx.clone();
        move |msg: Message, dialogue: BotDialogue| {
            let ctx = ctx.clone();
            async move {
                let raw_msg = serde_json::to_value(&msg).ok();
                let from_user_id = msg.from().map(|u| u.id.0 as i64);
                let chat_id = Some(msg.chat.id.0);
                let msg_date = Some(msg.date.to_rfc3339());
                let text = msg.text().map(|t| t.trim()).filter(|t| !t.is_empty());

                if let Some(raw) = raw_msg.as_ref() {
                    let _ = chat_repo::insert_update_log(
                        &ctx.pool,
                        chat_id,
                        from_user_id,
                        "message",
                        raw,
                    )
                    .await;
                }

                let _ = chat_repo::insert_message(
                    &ctx.pool,
                    msg.chat.id.0,
                    from_user_id,
                    "in",
                    text,
                    Some(msg.id.0 as i64),
                    msg_date.as_deref(),
                    raw_msg.as_ref(),
                )
                .await;

                if let Some(reply) = telegram_icon_file_id_reply(&ctx, &msg) {
                    if let Err(err) = ctx.bot.send_message(msg.chat.id, reply).await {
                        tracing::warn!(
                            "Failed to reply with Telegram icon file_id in chat {}: {err}",
                            msg.chat.id.0
                        );
                    }
                    return false;
                }

                for plugin in ctx.plugins.iter() {
                    match plugin
                        .handle_message(ctx.clone(), msg.clone(), dialogue.clone())
                        .await
                    {
                        Ok(true) => {
                            return false; // Đã xử lý, chặn lại không cho đi tiếp xuống dưới
                        }
                        Ok(false) => {}
                        Err(err) => {
                            tracing::error!(
                                "Plugin {} failed while handling message {} in chat {}: {err}",
                                plugin.name(),
                                msg.id.0,
                                msg.chat.id.0
                            );
                            return false;
                        }
                    }
                }
                true // Chưa xử lý, cho phép đi tiếp
            }
        }
    });

    let plugin_callback_filter = dptree::filter_async({
        let ctx = ctx.clone();
        move |query: CallbackQuery, dialogue: BotDialogue| {
            let ctx = ctx.clone();
            async move {
                if let Ok(raw) = serde_json::to_value(&query) {
                    let chat_id = query.message.as_ref().map(|m| m.chat().id.0);
                    let user_id = Some(query.from.id.0 as i64);
                    let _ = chat_repo::insert_update_log(
                        &ctx.pool,
                        chat_id,
                        user_id,
                        "callback_query",
                        &raw,
                    )
                    .await;
                }

                for plugin in ctx.plugins.iter() {
                    match plugin
                        .handle_callback(ctx.clone(), query.clone(), dialogue.clone())
                        .await
                    {
                        Ok(true) => {
                            return false;
                        }
                        Ok(false) => {}
                        Err(err) => {
                            tracing::error!(
                                "Plugin {} failed while handling callback {:?}: {err}",
                                plugin.name(),
                                query.data
                            );
                            return false;
                        }
                    }
                }
                true
            }
        }
    });

    // Build message and callback handlers separately so callbacks are not shadowed by filter_message.
    let message_handler = Update::filter_message()
        .enter_dialogue::<Message, SqliteDialogueStorage, State>()
        .chain(plugin_message_filter)
        .endpoint({
            let ctx = ctx.clone();
            move |_bot: Bot, msg: Message| {
                let ctx = ctx.clone();
                async move {
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
                    let text = ctx.get_text_lang(
                        "fallback_default",
                        &lang,
                        "👉 Use /shop to view products or /help for help.",
                    );
                    let _ = i18n::send_message_for_key(&ctx, msg.chat.id, "fallback_default", text)
                        .await;
                    Ok::<(), anyhow::Error>(())
                }
            }
        });

    let callback_handler = Update::filter_callback_query()
        .enter_dialogue::<CallbackQuery, SqliteDialogueStorage, State>()
        .chain(plugin_callback_filter)
        .endpoint({
            let ctx = ctx.clone();
            move |_bot: Bot, query: CallbackQuery| {
                let ctx = ctx.clone();
                async move {
                    if let Some(msg) = query.message {
                        let preferred =
                            users_repo::preferred_language(&ctx.pool, query.from.id.0 as i64)
                                .await
                                .ok()
                                .flatten()
                                .or_else(|| query.from.language_code.clone());
                        let lang = ctx.normalize_language_code(preferred.as_deref());
                        let text = ctx.get_text_lang(
                            "action_invalid",
                            &lang,
                            "Invalid action. Please try again by typing /shop.",
                        );
                        let _ =
                            i18n::send_message_for_key(&ctx, msg.chat().id, "action_invalid", text)
                                .await;
                    }
                    Ok::<(), anyhow::Error>(())
                }
            }
        });

    let handler = dptree::entry()
        .branch(message_handler)
        .branch(callback_handler);

    Dispatcher::builder(ctx.bot.clone(), handler)
        .dependencies(dptree::deps![storage])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}

fn telegram_icon_file_id_reply(ctx: &AppContext, msg: &Message) -> Option<String> {
    let from_user_id = msg.from().map(|user| user.id.0 as i64)?;
    if !ctx.is_telegram_icon_admin(from_user_id) {
        return None;
    }

    if let Some(reply) = msg
        .text()
        .zip(msg.entities())
        .and_then(|(text, entities)| custom_emoji_details_from_entities(text, entities))
    {
        return Some(reply);
    }

    if let Some(sticker) = msg.sticker() {
        return Some(format!(
            "Emoji từ media:\n\n{}\n\nKind: {:?}\nFormat: {:?}",
            sticker.emoji.as_deref().unwrap_or("none"),
            sticker.kind,
            sticker.format()
        ));
    }

    if let Some(animation) = msg.animation() {
        return Some(format!(
            "Animation file_id:\n\n{}\n\nFile name: {}\nMime type: {}",
            animation.file.id,
            animation.file_name.as_deref().unwrap_or("none"),
            animation
                .mime_type
                .as_ref()
                .map(|mime| mime.to_string())
                .unwrap_or_else(|| "none".to_string())
        ));
    }

    None
}

fn custom_emoji_details_from_entities(text: &str, entities: &[MessageEntity]) -> Option<String> {
    let parsed = MessageEntityRef::parse(text, entities);
    parsed.into_iter().find_map(|entity| {
        if let MessageEntityKind::CustomEmoji { custom_emoji_id } = entity.kind() {
            Some(format!(
                "Custom emoji:\n\nFallback: {}\nCustom emoji ID: {}",
                entity.text(),
                custom_emoji_id
            ))
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn custom_emoji_details_are_extracted_from_message_entities() {
        let text = "🔥";
        let entities = vec![MessageEntity::custom_emoji(
            teloxide::types::CustomEmojiId("5368324170671202286".to_string()),
            0,
            text.encode_utf16().count(),
        )];

        assert_eq!(
            custom_emoji_details_from_entities(text, &entities).as_deref(),
            Some("Custom emoji:\n\nFallback: 🔥\nCustom emoji ID: 5368324170671202286")
        );

        assert!(matches!(
            entities[0].kind,
            MessageEntityKind::CustomEmoji { .. }
        ));
    }
}
