use crate::app::AppContext;
use crate::domains::users::repo as users_repo;
use serde_json::{Value, json};
use std::collections::HashMap;
use teloxide::payloads::{SendMessage, SendMessageSetters};
use teloxide::requests::{JsonRequest, Requester};
use teloxide::types::{ChatId, CustomEmojiId, InlineKeyboardButton, KeyboardButton, MessageEntity};

const DEFAULT_CUSTOM_EMOJI_FALLBACK: &str = "✨";

pub async fn user_lang(
    ctx: &AppContext,
    user_id: i64,
    telegram_language_code: Option<&str>,
) -> String {
    if user_id == 0 {
        return ctx.normalize_language_code(telegram_language_code);
    }

    let preferred = users_repo::preferred_language(&ctx.pool, user_id)
        .await
        .ok()
        .flatten()
        .or_else(|| telegram_language_code.map(|lang| lang.to_string()));

    ctx.normalize_language_code(preferred.as_deref())
}

pub async fn user_lang_by_id(ctx: &AppContext, user_id: i64) -> String {
    user_lang(ctx, user_id, None).await
}

pub fn t(ctx: &AppContext, lang: &str, key: &str, default: &str) -> String {
    ctx.get_text_lang(key, lang, default)
}

pub fn tr(
    ctx: &AppContext,
    lang: &str,
    key: &str,
    default: &str,
    vars: &[(&str, String)],
) -> String {
    ctx.render_text_lang(key, lang, default, vars)
}

pub fn button_t(ctx: &AppContext, lang: &str, key: &str, default: &str) -> String {
    button_parts_for_key(ctx, key, t(ctx, lang, key, default)).text
}

pub fn button_tr(
    ctx: &AppContext,
    lang: &str,
    key: &str,
    default: &str,
    vars: &[(&str, String)],
) -> String {
    button_parts_for_key(ctx, key, tr(ctx, lang, key, default, vars)).text
}

pub fn button_text_for_key(ctx: &AppContext, key: &str, text: impl Into<String>) -> String {
    button_parts_for_key(ctx, key, text).text
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ButtonText {
    pub text: String,
    pub icon_custom_emoji_id: Option<String>,
}

pub fn button_parts_for_key(ctx: &AppContext, key: &str, text: impl Into<String>) -> ButtonText {
    let text = text.into();
    if let Some((text, custom_id)) = button_custom_emoji_placeholder_parts(&text) {
        return ButtonText {
            text,
            icon_custom_emoji_id: Some(custom_id),
        };
    }

    let Some(prefix) = ctx.i18n_emoji_prefix_for_key(key) else {
        return ButtonText {
            text,
            icon_custom_emoji_id: None,
        };
    };
    let trimmed = text.trim_start();
    if let Some(custom_id) = prefix.custom_emoji_id {
        ButtonText {
            text: trimmed
                .strip_prefix(&prefix.fallback)
                .map(str::trim_start)
                .unwrap_or(trimmed)
                .to_string(),
            icon_custom_emoji_id: Some(custom_id),
        }
    } else if trimmed.starts_with(&prefix.fallback) {
        ButtonText {
            text,
            icon_custom_emoji_id: None,
        }
    } else {
        ButtonText {
            text: format!("{} {}", prefix.fallback, text),
            icon_custom_emoji_id: None,
        }
    }
}

pub fn inline_button_callback_json(
    ctx: &AppContext,
    lang: &str,
    key: &str,
    default: &str,
    callback_data: impl Into<String>,
) -> Value {
    let parts = button_parts_for_key(ctx, key, t(ctx, lang, key, default));
    let mut button = json!({
        "text": parts.text,
        "callback_data": callback_data.into(),
    });
    if let Some(icon_id) = parts.icon_custom_emoji_id
        && let Some(obj) = button.as_object_mut()
    {
        obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
    }
    button
}

pub fn inline_button_callback(
    ctx: &AppContext,
    lang: &str,
    key: &str,
    default: &str,
    callback_data: impl Into<String>,
) -> InlineKeyboardButton {
    InlineKeyboardButton::callback(
        fallback_button_text_for_key(ctx, key, t(ctx, lang, key, default)),
        callback_data,
    )
}

pub fn keyboard_button_json(ctx: &AppContext, key: &str, text: impl Into<String>) -> Value {
    let parts = button_parts_for_key(ctx, key, text);
    let mut button = json!({
        "text": parts.text,
    });
    if let Some(icon_id) = parts.icon_custom_emoji_id
        && let Some(obj) = button.as_object_mut()
    {
        obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
    }
    button
}

pub fn keyboard_button(ctx: &AppContext, key: &str, text: impl Into<String>) -> KeyboardButton {
    KeyboardButton::new(fallback_button_text_for_key(ctx, key, text))
}

fn fallback_button_text_for_key(ctx: &AppContext, key: &str, text: impl Into<String>) -> String {
    let text = text.into();
    if contains_custom_emoji_id_placeholder(&text) {
        return render_button_custom_emoji_id_placeholders(&text);
    }

    let Some(prefix) = ctx.i18n_emoji_prefix_for_key(key) else {
        return text;
    };
    if text.trim_start().starts_with(&prefix.fallback) {
        text
    } else {
        format!("{} {}", prefix.fallback, text)
    }
}

fn button_custom_emoji_placeholder_parts(text: &str) -> Option<(String, String)> {
    let mut rendered = String::with_capacity(text.len());
    let mut first_custom_id = None;
    let mut byte_index = 0usize;

    while byte_index < text.len() {
        let remaining = &text[byte_index..];
        if let Some((placeholder, custom_id)) = raw_custom_emoji_id_placeholder(remaining) {
            first_custom_id.get_or_insert(custom_id);
            byte_index += placeholder.len();
        } else if let Some(ch) = remaining.chars().next() {
            rendered.push(ch);
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }

    first_custom_id.map(|custom_id| {
        let text = rendered.split_whitespace().collect::<Vec<_>>().join(" ");
        let text = if text.is_empty() {
            DEFAULT_CUSTOM_EMOJI_FALLBACK.to_string()
        } else {
            text
        };
        (text, custom_id)
    })
}

fn contains_custom_emoji_id_placeholder(text: &str) -> bool {
    let mut byte_index = 0usize;
    while byte_index < text.len() {
        let remaining = &text[byte_index..];
        if raw_custom_emoji_id_placeholder(remaining).is_some() {
            return true;
        }
        if let Some(ch) = remaining.chars().next() {
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }
    false
}

fn render_button_custom_emoji_id_placeholders(text: &str) -> String {
    let mut rendered = String::with_capacity(text.len());
    let mut byte_index = 0usize;

    while byte_index < text.len() {
        let remaining = &text[byte_index..];
        if let Some((placeholder, _custom_id)) = raw_custom_emoji_id_placeholder(remaining) {
            rendered.push_str(DEFAULT_CUSTOM_EMOJI_FALLBACK);
            byte_index += placeholder.len();
        } else if let Some(ch) = remaining.chars().next() {
            rendered.push(ch);
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }

    rendered.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[allow(dead_code)]
pub fn text_for_key(ctx: &AppContext, key: &str, text: impl Into<String>) -> String {
    rich_text_for_key(ctx, key, text).text
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RichText {
    pub text: String,
    pub entities: Vec<MessageEntity>,
}

pub fn rich_text_for_key(ctx: &AppContext, key: &str, text: impl Into<String>) -> RichText {
    let text = text.into();
    let Some(prefix) = ctx.i18n_emoji_prefix_for_key(key) else {
        let mut rich = render_custom_emoji_id_placeholders(&text, &ctx.custom_emoji_map());
        let entities = custom_emoji_entities_for_text(ctx, &rich.text);
        merge_custom_emoji_entities(&mut rich.entities, entities);
        return rich;
    };

    let prefix_custom_emojis = custom_emoji_map_from_prefix(&prefix);
    let placeholder_rich = render_custom_emoji_id_placeholders(&text, &prefix_custom_emojis);
    if !placeholder_rich.entities.is_empty() {
        let mut entities = custom_emoji_entities_for_text(ctx, &placeholder_rich.text);
        merge_custom_emoji_entities(&mut entities, placeholder_rich.entities);
        return RichText {
            text: placeholder_rich.text,
            entities,
        };
    }

    let full_text = format!("{} {}", prefix.fallback, text);
    let mut entities = custom_emoji_entities_for_text(ctx, &full_text);
    let prefix_entities = custom_emoji_entities_for_prefix(&prefix);
    if !prefix_entities.is_empty() {
        entities.retain(|entity| {
            !prefix_entities.iter().any(|prefix_entity| {
                prefix_entity.offset == entity.offset && prefix_entity.length == entity.length
            })
        });
        entities.extend(prefix_entities);
        entities.sort_by_key(|entity| entity.offset);
    }

    RichText {
        text: full_text,
        entities,
    }
}

pub fn render_custom_emoji_id_placeholders(
    text: &str,
    custom_emojis: &HashMap<String, String>,
) -> RichText {
    let mut placeholders = custom_emojis
        .iter()
        .map(|(fallback, custom_id)| {
            (
                format!("{{{custom_id}}}"),
                fallback.clone(),
                custom_id.clone(),
            )
        })
        .collect::<Vec<_>>();
    placeholders.sort_by_key(|(placeholder, _, _)| std::cmp::Reverse(placeholder.len()));

    let mut rendered = String::with_capacity(text.len());
    let mut entities = Vec::new();
    let mut byte_index = 0usize;
    let mut utf16_offset = 0usize;

    while byte_index < text.len() {
        let remaining = &text[byte_index..];
        if let Some((placeholder, fallback, custom_id)) = placeholders
            .iter()
            .find(|(placeholder, _, _)| remaining.starts_with(placeholder.as_str()))
        {
            rendered.push_str(fallback);
            entities.push(MessageEntity::custom_emoji(
                CustomEmojiId(custom_id.clone()),
                utf16_offset,
                fallback.encode_utf16().count(),
            ));
            byte_index += placeholder.len();
            utf16_offset += fallback.encode_utf16().count();
        } else if let Some((placeholder, custom_id)) = raw_custom_emoji_id_placeholder(remaining) {
            rendered.push_str(DEFAULT_CUSTOM_EMOJI_FALLBACK);
            entities.push(MessageEntity::custom_emoji(
                CustomEmojiId(custom_id),
                utf16_offset,
                DEFAULT_CUSTOM_EMOJI_FALLBACK.encode_utf16().count(),
            ));
            byte_index += placeholder.len();
            utf16_offset += DEFAULT_CUSTOM_EMOJI_FALLBACK.encode_utf16().count();
        } else if let Some(ch) = remaining.chars().next() {
            rendered.push(ch);
            byte_index += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        } else {
            break;
        }
    }

    RichText {
        text: rendered,
        entities,
    }
}

fn raw_custom_emoji_id_placeholder(text: &str) -> Option<(String, String)> {
    let rest = text.strip_prefix('{')?;
    let close_index = rest.find('}')?;
    let candidate = &rest[..close_index];
    if candidate.len() < 8 || candidate.len() > 64 || !candidate.chars().all(|c| c.is_ascii_digit())
    {
        return None;
    }
    Some((format!("{{{candidate}}}"), candidate.to_string()))
}

fn custom_emoji_entities_for_text(ctx: &AppContext, text: &str) -> Vec<MessageEntity> {
    let map = ctx.custom_emoji_map();
    custom_emoji_entities_for_map(&map, text)
}

pub fn custom_emoji_entities_for_map(
    map: &HashMap<String, String>,
    text: &str,
) -> Vec<MessageEntity> {
    if map.is_empty() {
        return Vec::new();
    }

    let mut fallbacks = map.keys().cloned().collect::<Vec<_>>();
    fallbacks.sort_by_key(|fallback| std::cmp::Reverse(fallback.encode_utf16().count()));

    let mut entities = Vec::new();
    let mut byte_index = 0usize;
    let mut utf16_offset = 0usize;

    while byte_index < text.len() {
        let remaining = &text[byte_index..];
        if let Some(fallback) = fallbacks
            .iter()
            .find(|fallback| remaining.starts_with(fallback.as_str()))
        {
            if let Some(custom_id) = map.get(fallback) {
                entities.push(MessageEntity::custom_emoji(
                    CustomEmojiId(custom_id.clone()),
                    utf16_offset,
                    fallback.encode_utf16().count(),
                ));
            }
            byte_index += fallback.len();
            utf16_offset += fallback.encode_utf16().count();
        } else if let Some(ch) = remaining.chars().next() {
            byte_index += ch.len_utf8();
            utf16_offset += ch.len_utf16();
        } else {
            break;
        }
    }

    entities
}

pub fn merge_custom_emoji_entities(
    entities: &mut Vec<MessageEntity>,
    mut extra_entities: Vec<MessageEntity>,
) {
    entities.retain(|entity| {
        !extra_entities
            .iter()
            .any(|extra| extra.offset == entity.offset && extra.length == entity.length)
    });
    entities.append(&mut extra_entities);
    entities.sort_by_key(|entity| entity.offset);
}

fn custom_emoji_map_from_prefix(prefix: &crate::app::I18nEmojiPrefix) -> HashMap<String, String> {
    prefix
        .emojis
        .iter()
        .filter_map(|item| {
            Some((
                item.fallback.clone(),
                item.custom_emoji_id.as_ref()?.clone(),
            ))
        })
        .collect()
}

fn custom_emoji_entities_for_prefix(prefix: &crate::app::I18nEmojiPrefix) -> Vec<MessageEntity> {
    let items = if prefix.emojis.is_empty() {
        Vec::new()
    } else {
        prefix.emojis.clone()
    };
    let mut entities = Vec::new();
    let mut utf16_offset = 0usize;
    for (index, item) in items.iter().enumerate() {
        let fallback_len = item.fallback.encode_utf16().count();
        if let Some(custom_id) = item.custom_emoji_id.as_ref() {
            entities.push(MessageEntity::custom_emoji(
                CustomEmojiId(custom_id.clone()),
                utf16_offset,
                fallback_len,
            ));
        }
        utf16_offset += fallback_len;
        if index + 1 < items.len() {
            utf16_offset += " ".encode_utf16().count();
        }
    }
    entities
}

pub fn message_payload_with_json_keyboard(
    ctx: &AppContext,
    chat_id: ChatId,
    key: &str,
    text: impl Into<String>,
    reply_markup: Value,
) -> serde_json::Result<Value> {
    let rich = rich_text_for_key(ctx, key, text);
    let mut payload = json!({
        "chat_id": chat_id.0,
        "text": rich.text,
        "reply_markup": reply_markup,
    });
    if !rich.entities.is_empty()
        && let Some(obj) = payload.as_object_mut()
    {
        obj.insert("entities".to_string(), serde_json::to_value(rich.entities)?);
    }
    Ok(payload)
}

pub async fn send_message_with_json_keyboard(
    ctx: &AppContext,
    chat_id: ChatId,
    key: &str,
    text: impl Into<String>,
    reply_markup: Value,
) -> anyhow::Result<()> {
    let payload = message_payload_with_json_keyboard(ctx, chat_id, key, text, reply_markup)?;
    send_raw_telegram_method(ctx, "sendMessage", payload).await
}

pub async fn send_raw_telegram_method(
    ctx: &AppContext,
    method: &str,
    payload: Value,
) -> anyhow::Result<()> {
    let url = format!(
        "https://api.telegram.org/bot{}/{}",
        ctx.config.telegram_token, method
    );
    let response = ctx.bot.client().post(url).json(&payload).send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        anyhow::bail!("Telegram {method} failed with HTTP {status}: {body}");
    }
    let json: Value = serde_json::from_str(&body)?;
    if json.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        anyhow::bail!("Telegram {method} failed: {body}");
    }
}

pub fn send_message_for_key(
    ctx: &AppContext,
    chat_id: ChatId,
    key: &str,
    text: impl Into<String>,
) -> JsonRequest<SendMessage> {
    let rich = rich_text_for_key(ctx, key, text);
    let request = ctx.bot.send_message(chat_id, rich.text);
    if rich.entities.is_empty() {
        request
    } else {
        request.entities(rich.entities)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use std::collections::HashMap;
    use teloxide::Bot;

    #[tokio::test]
    async fn text_for_key_prefixes_configured_emoji() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":"❓"}"#.to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(text_for_key(&ctx, "help", "Help text"), "❓ Help text");
    }

    #[tokio::test]
    async fn rich_text_for_key_prefixes_custom_emoji_entity() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":{"fallback":"🔥","custom_emoji_id":"5368324170671202286"}}"#
                        .to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let rich = rich_text_for_key(&ctx, "help", "Help text");

        assert_eq!(rich.text, "🔥 Help text");
        assert_eq!(rich.entities.len(), 1);
        assert_eq!(rich.entities[0].offset, 0);
        assert_eq!(rich.entities[0].length, "🔥".encode_utf16().count());
        match &rich.entities[0].kind {
            teloxide::types::MessageEntityKind::CustomEmoji { custom_emoji_id } => {
                assert_eq!(custom_emoji_id.0, "5368324170671202286");
            }
            kind => panic!("expected custom emoji entity, got {kind:?}"),
        }
    }

    #[tokio::test]
    async fn rich_text_for_key_prefixes_multiple_custom_emoji_entities() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":{"emojis":[{"fallback":"🔥","custom_emoji_id":"5368324170671202286"},{"fallback":"🎁","custom_emoji_id":"5368324170671202287"}]}}"#
                        .to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let rich = rich_text_for_key(&ctx, "help", "Help text");

        assert_eq!(rich.text, "🔥 🎁 Help text");
        assert_eq!(rich.entities.len(), 2);
        assert_eq!(rich.entities[0].offset, 0);
        assert_eq!(rich.entities[1].offset, "🔥 ".encode_utf16().count());
        let custom_ids = rich
            .entities
            .iter()
            .map(|entity| match &entity.kind {
                teloxide::types::MessageEntityKind::CustomEmoji { custom_emoji_id } => {
                    custom_emoji_id.0.as_str()
                }
                kind => panic!("expected custom emoji entity, got {kind:?}"),
            })
            .collect::<Vec<_>>();
        assert_eq!(
            custom_ids,
            vec!["5368324170671202286", "5368324170671202287"]
        );
    }

    #[tokio::test]
    async fn rich_text_for_key_places_custom_emoji_id_placeholders_inline() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"help":{"emojis":[{"fallback":"🔥","custom_emoji_id":"5368324170671202286"},{"fallback":"🎁","custom_emoji_id":"5368324170671202287"}]}}"#
                        .to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let rich = rich_text_for_key(
            &ctx,
            "help",
            "Dong 1 {5368324170671202286}\nDong 2 {5368324170671202287}",
        );

        assert_eq!(rich.text, "Dong 1 🔥\nDong 2 🎁");
        assert_eq!(rich.entities.len(), 2);
        assert_eq!(rich.entities[0].offset, "Dong 1 ".encode_utf16().count());
        assert_eq!(
            rich.entities[1].offset,
            "Dong 1 🔥\nDong 2 ".encode_utf16().count()
        );
    }

    #[tokio::test]
    async fn rich_text_for_key_places_unconfigured_custom_emoji_id_placeholders_inline() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([("telegram_i18n_emojis_enabled".to_string(), "1".to_string())]),
            BotTexts::default(),
            vec![],
        );

        let rich = rich_text_for_key(
            &ctx,
            "broadcast_message",
            "test {5375135722514685501} emoji ty nha {5420147074266044260}",
        );

        assert_eq!(rich.text, "test ✨ emoji ty nha ✨");
        assert_eq!(rich.entities.len(), 2);
        assert_eq!(rich.entities[0].offset, "test ".encode_utf16().count());
        assert_eq!(
            rich.entities[1].offset,
            "test ✨ emoji ty nha ".encode_utf16().count()
        );
    }

    #[tokio::test]
    async fn rich_text_for_key_marks_multiple_global_custom_emojis() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_custom_emojis".to_string(),
                    r#"{"🔥":"5368324170671202286","🎁":"5368324170671202287","✅":"5368324170671202288"}"#.to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let rich = rich_text_for_key(&ctx, "broadcast_message", "🔥 text1\n🎁 text2\n✅ text3");

        assert_eq!(rich.text, "🔥 text1\n🎁 text2\n✅ text3");
        assert_eq!(rich.entities.len(), 3);
        assert_eq!(rich.entities[0].offset, 0);
        assert_eq!(rich.entities[0].length, "🔥".encode_utf16().count());
        assert_eq!(rich.entities[1].offset, "🔥 text1\n".encode_utf16().count());
        assert_eq!(
            rich.entities[2].offset,
            "🔥 text1\n🎁 text2\n".encode_utf16().count()
        );
    }

    #[tokio::test]
    async fn message_payload_with_json_keyboard_keeps_entities_and_icon_button() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_custom_emojis".to_string(),
                    r#"{"🔥":"5368324170671202286"}"#.to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let payload = message_payload_with_json_keyboard(
            &ctx,
            ChatId(1),
            "start",
            "🔥 text",
            json!({"inline_keyboard":[[{"text":"Buy","callback_data":"buy:1","icon_custom_emoji_id":"5368324170671202287"}]]}),
        )
        .unwrap();

        assert_eq!(
            payload["entities"][0]["custom_emoji_id"],
            "5368324170671202286"
        );
        assert_eq!(
            payload["reply_markup"]["inline_keyboard"][0][0]["icon_custom_emoji_id"],
            "5368324170671202287"
        );
    }

    #[tokio::test]
    async fn button_text_for_key_uses_fallback_emoji_without_custom_entity() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"start_btn_language":{"fallback":"🌐","custom_emoji_id":"5368324170671202286"},"start_btn_help":"❓"}"#.to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        assert_eq!(
            button_text_for_key(&ctx, "start_btn_language", "Language"),
            "Language"
        );
        assert_eq!(
            button_text_for_key(&ctx, "start_btn_language", "🌐 Language"),
            "Language"
        );
        assert_eq!(
            button_text_for_key(&ctx, "start_btn_help", "Help"),
            "❓ Help"
        );
        let parts = button_parts_for_key(&ctx, "start_btn_language", "🌐 Language");
        assert_eq!(parts.text, "Language");
        assert_eq!(
            parts.icon_custom_emoji_id.as_deref(),
            Some("5368324170671202286")
        );
        let json =
            inline_button_callback_json(&ctx, "en", "start_btn_language", "Language", "lang");
        assert_eq!(json["text"], "Language");
        assert_eq!(json["callback_data"], "lang");
        assert_eq!(json["icon_custom_emoji_id"], "5368324170671202286");
    }

    #[tokio::test]
    async fn button_text_for_key_uses_custom_emoji_id_placeholder_as_button_icon() {
        let mut env = HashMap::new();
        env.insert("TELOXIDE_TOKEN".to_string(), "test-token".to_string());
        env.insert("DATABASE_URL".to_string(), "sqlite::memory:".to_string());
        env.insert("ADMIN_SETUP_CODE".to_string(), "setup123".to_string());
        env.insert(
            "ADMIN_JWT_SECRET".to_string(),
            "0123456789abcdef0123456789abcdef".to_string(),
        );
        let config = Config::from_env_map(&env).unwrap();
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            config,
            HashMap::new(),
            BotTexts::default(),
            vec![],
        );

        let parts =
            button_parts_for_key(&ctx, "start_btn_shop", "{6172437452590944785} Xem sản phẩm");

        assert_eq!(parts.text, "Xem sản phẩm");
        assert_eq!(
            parts.icon_custom_emoji_id.as_deref(),
            Some("6172437452590944785")
        );
        let json =
            keyboard_button_json(&ctx, "start_btn_shop", "{6172437452590944785} Xem sản phẩm");
        assert_eq!(json["text"], "Xem sản phẩm");
        assert_eq!(json["icon_custom_emoji_id"], "6172437452590944785");
    }
}
