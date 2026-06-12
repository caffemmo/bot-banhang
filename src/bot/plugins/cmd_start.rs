use anyhow::Result;
use serde_json::{Value, json};
use std::sync::Arc;
use teloxide::payloads::AnswerCallbackQuerySetters;
use teloxide::payloads::SendMessageSetters;
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, InlineKeyboardButton, InlineKeyboardMarkup, Message, User,
};
use url::Url;

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::plugins::cmd_orders;
use crate::bot::plugins::cmd_shop;
use crate::bot::plugins::cmd_wallet;
use crate::bot::texts::BotTexts;
use crate::bot::{BotDialogue, State};
use crate::domains::users::models::Subscriber;
use crate::domains::users::repo as users_repo;

pub struct StartCommandPlugin;
const JOIN_CHECK_CALLBACK: &str = "start:check_join";
const FACEBOOK_UNLOCK_CALLBACK: &str = "start:facebook_unlock";
const FACEBOOK_UNLOCK_TYPE_282_CALLBACK: &str = "facebook_unlock:type:282";
const FACEBOOK_UNLOCK_TYPE_956_CALLBACK: &str = "facebook_unlock:type:956";
const FACEBOOK_UNLOCK_ENTER_UID_CALLBACK: &str = "facebook_unlock:enter_uid:282";
const FACEBOOK_UNLOCK_NO_UID_CALLBACK: &str = "facebook_unlock:no_uid:282";
const SUPPORT_HISTORY_CALLBACK: &str = "start:support_history";
const DEFAULT_REQUIRED_CHANNEL_URL: &str = "https://t.me/zvwboo";

#[derive(Debug, Clone, PartialEq, Eq)]
enum StartEntry {
    Menu(String),
    LanguagePrompt(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StartMenuAction {
    Shop,
    Topup,
    Wallet,
    Orders,
    TopupHistory,
    ApiIntegration,
    FacebookUnlock,
    SupportHistory,
    Help,
    Language,
}

fn t_lang(ctx: &AppContext, lang: &str, key: &str, default: &str) -> String {
    ctx.get_text_lang(key, lang, default)
}

fn required_channel_enabled_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes" | "enabled"
    )
}

fn normalize_t_me_url(value: &str) -> Option<String> {
    let trimmed = value.trim();
    let username = trimmed
        .strip_prefix("https://t.me/")
        .or_else(|| trimmed.strip_prefix("http://t.me/"))
        .or_else(|| trimmed.strip_prefix("t.me/"))?
        .split(['?', '/', '#'])
        .next()
        .unwrap_or("")
        .trim();

    if username.is_empty() || username.starts_with('+') {
        None
    } else {
        Some(format!("@{username}"))
    }
}

fn normalize_required_channel_id(channel_id: &str, channel_url: &str) -> Option<String> {
    let id = channel_id.trim();
    if id.is_empty() {
        return normalize_t_me_url(channel_url);
    }

    if id.starts_with("http://t.me/") || id.starts_with("https://t.me/") || id.starts_with("t.me/")
    {
        return normalize_t_me_url(id);
    }

    if id.starts_with('@') || id.starts_with("-100") {
        Some(id.to_string())
    } else {
        Some(format!("@{id}"))
    }
}

fn required_channel_enabled(ctx: &AppContext) -> bool {
    required_channel_enabled_value(&ctx.get_text("required_channel_enabled", "1"))
}

fn required_channel_id(ctx: &AppContext) -> Option<String> {
    normalize_required_channel_id(
        &ctx.get_text("required_channel_id", "@zvwboo"),
        &ctx.get_text("required_channel_url", DEFAULT_REQUIRED_CHANNEL_URL),
    )
}

fn required_channel_url(ctx: &AppContext) -> String {
    ctx.get_text("required_channel_url", DEFAULT_REQUIRED_CHANNEL_URL)
}

fn start_user_is_admin(ctx: &AppContext, user_id: i64) -> bool {
    ctx.is_telegram_icon_admin(user_id)
        || ctx
            .order_notification_admin_ids()
            .into_iter()
            .any(|admin_id| admin_id == user_id)
}

async fn preferred_or_telegram_lang(
    ctx: &AppContext,
    user_id: i64,
    telegram_language_code: Option<&str>,
) -> String {
    let preferred = users_repo::preferred_language(&ctx.pool, user_id)
        .await
        .ok()
        .flatten()
        .or_else(|| telegram_language_code.map(|lang| lang.to_string()));

    ctx.normalize_language_code(preferred.as_deref())
}

async fn upsert_subscriber_from_user(
    ctx: &AppContext,
    user: &User,
    chat_id: i64,
    preferred_language: Option<String>,
) {
    let first_name = Some(user.first_name.clone());
    let last_name = user.last_name.clone();
    let full_name_from_parts = format!(
        "{} {}",
        first_name.clone().unwrap_or_default(),
        last_name.clone().unwrap_or_default()
    )
    .trim()
    .to_string();
    let full_name = if full_name_from_parts.is_empty() {
        user.username.clone()
    } else {
        Some(full_name_from_parts)
    };

    let profile = Subscriber {
        user_id: user.id.0 as i64,
        chat_id,
        username: user.username.clone(),
        first_name,
        last_name,
        full_name,
        language_code: user.language_code.clone(),
        preferred_language,
        stock_notifications_enabled: Some(1),
        is_bot: Some(if user.is_bot { 1 } else { 0 }),
        created_at: None,
        updated_at: None,
    };
    if let Err(err) = users_repo::upsert_subscriber(&ctx.pool, &profile).await {
        tracing::warn!("Failed to upsert subscriber {}: {err}", user.id.0);
    }
}

async fn user_has_joined_required_channel(
    ctx: &AppContext,
    user_id: teloxide::types::UserId,
) -> bool {
    if !required_channel_enabled(ctx) {
        return true;
    }

    let Some(channel_id) = required_channel_id(ctx) else {
        return true;
    };

    match ctx.bot.get_chat_member(channel_id, user_id).await {
        Ok(member) => member.kind.is_present(),
        Err(err) => {
            tracing::warn!("Failed to check required channel membership: {err}");
            false
        }
    }
}

fn join_required_channel_keyboard(
    ctx: &AppContext,
    channel_url: &str,
    lang: &str,
) -> InlineKeyboardMarkup {
    let mut rows: Vec<Vec<InlineKeyboardButton>> = Vec::new();
    if let Ok(url) = Url::parse(channel_url) {
        rows.push(vec![InlineKeyboardButton::url(
            t_lang(ctx, lang, "required_channel_join_btn", "📢 Join channel"),
            url,
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        t_lang(ctx, lang, "required_channel_check_btn", "✅ I joined"),
        JOIN_CHECK_CALLBACK,
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn join_required_channel_keyboard_json(ctx: &AppContext, channel_url: &str, lang: &str) -> Value {
    let mut rows: Vec<Vec<Value>> = Vec::new();
    if Url::parse(channel_url).is_ok() {
        rows.push(vec![json!({
            "text": i18n::button_text_for_key(
                ctx,
                "required_channel_join_btn",
                t_lang(ctx, lang, "required_channel_join_btn", "📢 Join channel"),
            ),
            "url": channel_url,
        })]);
    }
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "required_channel_check_btn",
        "✅ I joined",
        JOIN_CHECK_CALLBACK,
    )]);
    json!({ "inline_keyboard": rows })
}

async fn send_required_channel_prompt(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    lang: &str,
) -> Result<(), anyhow::Error> {
    let channel_url = required_channel_url(ctx);
    let text = ctx.render_text_lang(
        "required_channel_message",
        lang,
        "📢 Please join the channel before using this bot:\n{channel_url}\n\nAfter joining, press “I joined”.",
        &[("channel_url", channel_url.clone())],
    );
    i18n::send_message_with_json_keyboard(
        ctx,
        chat_id,
        "required_channel_message",
        text,
        join_required_channel_keyboard_json(ctx, &channel_url, lang),
    )
    .await?;
    Ok(())
}

async fn send_start_menu(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    lang: &str,
    is_admin: bool,
) -> Result<(), anyhow::Error> {
    let msg_text = t_lang(
        ctx,
        lang,
        "start",
        "👋 Welcome! Use the buttons below, or type /shop to buy and /orders to view orders.",
    );
    i18n::send_message_with_json_keyboard(
        ctx,
        chat_id,
        "start",
        msg_text,
        start_menu_keyboard_json(ctx, lang, is_admin),
    )
    .await?;
    Ok(())
}

async fn send_facebook_unlock_type_prompt(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    lang: &str,
) -> Result<(), anyhow::Error> {
    let text = t_lang(
        ctx,
        lang,
        "facebook_unlock_type_prompt",
        "Bạn muốn mở tài khoản facebook dạng nào?",
    );
    i18n::send_message_with_json_keyboard(
        ctx,
        chat_id,
        "facebook_unlock_type_prompt",
        text,
        facebook_unlock_type_keyboard_json(ctx, lang),
    )
    .await?;
    Ok(())
}

fn facebook_unlock_type_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    json!({
        "inline_keyboard": [
            [
                i18n::inline_button_callback_json(
                    ctx,
                    lang,
                    "facebook_unlock_type_282_btn",
                    "dạng khóa 282",
                    FACEBOOK_UNLOCK_TYPE_282_CALLBACK,
                ),
                i18n::inline_button_callback_json(
                    ctx,
                    lang,
                    "facebook_unlock_type_956_btn",
                    "dạng khóa 956",
                    FACEBOOK_UNLOCK_TYPE_956_CALLBACK,
                ),
            ],
            [i18n::inline_button_callback_json(
                ctx,
                lang,
                "back_btn",
                "⬅️ Quay lại",
                "start:menu",
            )],
        ]
    })
}

async fn send_facebook_unlock_uid_prompt(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    lang: &str,
) -> Result<(), anyhow::Error> {
    let text = t_lang(
        ctx,
        lang,
        "facebook_unlock_uid_prompt",
        "Vui lòng nhập UID facebook (Bắt buộc):",
    );
    i18n::send_message_with_json_keyboard(
        ctx,
        chat_id,
        "facebook_unlock_uid_prompt",
        text,
        facebook_unlock_uid_keyboard_json(ctx, lang),
    )
    .await?;
    Ok(())
}

fn facebook_unlock_uid_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    json!({
        "inline_keyboard": [
            [
                i18n::inline_button_callback_json(
                    ctx,
                    lang,
                    "facebook_unlock_enter_uid_btn",
                    "Nhập UID",
                    FACEBOOK_UNLOCK_ENTER_UID_CALLBACK,
                ),
                i18n::inline_button_callback_json(
                    ctx,
                    lang,
                    "facebook_unlock_no_uid_btn",
                    "Tôi không có UID",
                    FACEBOOK_UNLOCK_NO_UID_CALLBACK,
                ),
            ],
            [i18n::inline_button_callback_json(
                ctx,
                lang,
                "back_btn",
                "⬅️ Quay lại",
                FACEBOOK_UNLOCK_CALLBACK,
            )],
        ]
    })
}

async fn send_facebook_unlock_uid_help(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    lang: &str,
) -> Result<(), anyhow::Error> {
    let text = t_lang(
        ctx,
        lang,
        "facebook_unlock_uid_help",
        "Video lấy UID Bị khóa 282 trên máy tính:\nhttps://drive.google.com/file/d/1JGWYTf7Feff4HPD2bCvFGjyyIZMNBnl2/view?usp=sharing\n\nNếu không có máy tính vui lòng liên hệ admin",
    );
    ctx.bot.send_message(chat_id, text).await?;
    Ok(())
}

async fn handle_facebook_unlock_uid_message(
    ctx: &AppContext,
    msg: Message,
    dialogue: BotDialogue,
    lang: &str,
    lock_type: String,
) -> Result<(), anyhow::Error> {
    let uid = msg.text().unwrap_or("").trim();
    if uid.is_empty() {
        let text = t_lang(
            ctx,
            lang,
            "facebook_unlock_uid_empty",
            "UID không được để trống. Vui lòng nhập UID facebook:",
        );
        i18n::send_message_with_json_keyboard(
            ctx,
            msg.chat.id,
            "facebook_unlock_uid_empty",
            text,
            facebook_unlock_uid_keyboard_json(ctx, lang),
        )
        .await?;
        return Ok(());
    }

    let text = ctx.render_text_lang(
        "facebook_unlock_uid_received",
        lang,
        "Mình đã nhận UID facebook: {uid}\nDạng khóa: {lock_type}\n\nVui lòng chờ hướng dẫn tiếp theo.",
        &[("uid", uid.to_string()), ("lock_type", lock_type)],
    );
    ctx.bot.send_message(msg.chat.id, text).await?;
    dialogue.update(State::Idle).await?;
    Ok(())
}

fn start_menu_keyboard_json(ctx: &AppContext, lang: &str, is_admin: bool) -> Value {
    let mut rows = vec![
            vec![i18n::inline_button_callback_json(ctx, lang, "start_btn_shop", "🛒 Shop", "start:shop")],
            vec![i18n::inline_button_callback_json(
                ctx,
                lang,
                "start_btn_facebook_unlock",
                "Mở khóa facebook",
                FACEBOOK_UNLOCK_CALLBACK,
            )],
            vec![
                i18n::inline_button_callback_json(ctx, lang, "start_btn_topup", "💰 Top up", "wallet:topup"),
                i18n::inline_button_callback_json(ctx, lang, "start_btn_wallet", "💳 Wallet", "start:wallet"),
            ],
            vec![
                i18n::inline_button_callback_json(ctx, lang, "start_btn_purchased", "📦 Purchased", "start:orders"),
                i18n::inline_button_callback_json(ctx, lang, "start_btn_topup_history", "📜 Top-up history", "wallet:topup_history"),
            ],
            vec![
                i18n::inline_button_callback_json(ctx, lang, "start_btn_api_integration", "🔌 API integration", "shop_api"),
                i18n::inline_button_callback_json(ctx, lang, "start_btn_help", "Help", "start:help"),
            ],
    ];
    if is_admin {
        rows.push(vec![i18n::inline_button_callback_json(
            ctx,
            lang,
            "start_btn_support_history",
            "🧾 Lịch sử yêu cầu",
            SUPPORT_HISTORY_CALLBACK,
        )]);
    }
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        lang,
        "start_btn_language",
        "🌐 Language",
        "start:language",
    )]);
    json!({
        "inline_keyboard": rows
    })
}

fn start_menu_button_specs_from_texts(texts: &BotTexts, lang: &str) -> Vec<Vec<(String, String)>> {
    vec![
        vec![(
            texts.get_lang("start_btn_shop", lang, "🛒 Shop"),
            "start:shop".to_string(),
        )],
        vec![(
            texts.get_lang("start_btn_facebook_unlock", lang, "Mở khóa facebook"),
            FACEBOOK_UNLOCK_CALLBACK.to_string(),
        )],
        vec![
            (
                texts.get_lang("start_btn_topup", lang, "💰 Top up"),
                "wallet:topup".to_string(),
            ),
            (
                texts.get_lang("start_btn_wallet", lang, "💳 Wallet"),
                "start:wallet".to_string(),
            ),
        ],
        vec![
            (
                texts.get_lang("start_btn_purchased", lang, "📦 Purchased"),
                "start:orders".to_string(),
            ),
            (
                texts.get_lang("start_btn_topup_history", lang, "📜 Top-up history"),
                "wallet:topup_history".to_string(),
            ),
        ],
        vec![
            (
                texts.get_lang("start_btn_api_integration", lang, "🔌 API integration"),
                "shop_api".to_string(),
            ),
            (
                texts.get_lang("start_btn_help", lang, "Help"),
                "start:help".to_string(),
            ),
        ],
        vec![(
            texts.get_lang("start_btn_language", lang, "🌐 Language"),
            "start:language".to_string(),
        )],
    ]
}

async fn send_message_with_start_reply_keyboard(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    key: &str,
    text: impl Into<String>,
    lang: &str,
) -> Result<(), anyhow::Error> {
    i18n::send_message_with_json_keyboard(
        ctx,
        chat_id,
        key,
        text,
        start_reply_keyboard_json(ctx, lang),
    )
    .await
}

fn start_reply_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    json!({
        "keyboard": start_reply_keyboard_button_rows(ctx, lang),
        "is_persistent": true,
        "resize_keyboard": true,
        "input_field_placeholder": t_lang(
            ctx,
            lang,
            "start_keyboard_placeholder",
            "Choose an action",
        ),
    })
}

fn start_reply_keyboard_button_rows(ctx: &AppContext, lang: &str) -> Vec<Vec<Value>> {
    ctx.texts
        .read()
        .map(|texts| {
            start_menu_button_specs_from_texts(&texts, lang)
                .into_iter()
                .map(|row| {
                    row.into_iter()
                        .map(|(label, callback)| {
                            i18n::keyboard_button_json(
                                ctx,
                                start_menu_button_key_for_callback(&callback),
                                label,
                            )
                        })
                        .collect::<Vec<_>>()
                })
                .collect::<Vec<_>>()
        })
        .unwrap_or_else(|_| {
            vec![
                vec![json!({"text": "🛒 Shop"})],
                vec![json!({"text": "Mở khóa facebook"})],
                vec![json!({"text": "💰 Top up"}), json!({"text": "💳 Wallet"})],
                vec![
                    json!({"text": "📦 Purchased"}),
                    json!({"text": "📜 Top-up history"}),
                ],
                vec![
                    json!({"text": "🔌 API integration"}),
                    json!({"text": "Help"}),
                ],
                vec![json!({"text": "🌐 Language"})],
            ]
        })
}

fn start_menu_button_key_for_callback(callback: &str) -> &'static str {
    match callback {
        "start:shop" => "start_btn_shop",
        "wallet:topup" => "start_btn_topup",
        "start:wallet" => "start_btn_wallet",
        "start:orders" => "start_btn_purchased",
        "wallet:topup_history" => "start_btn_topup_history",
        "shop_api" => "start_btn_api_integration",
        FACEBOOK_UNLOCK_CALLBACK => "start_btn_facebook_unlock",
        "start:help" => "start_btn_help",
        "start:language" => "start_btn_language",
        _ => "start_btn",
    }
}

fn start_reply_keyboard_specs_from_texts(texts: &BotTexts, lang: &str) -> Vec<Vec<String>> {
    start_menu_button_specs_from_texts(texts, lang)
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|(label, _callback)| label)
                .collect::<Vec<_>>()
        })
        .collect()
}

async fn send_language_prompt(
    ctx: &AppContext,
    chat_id: teloxide::types::ChatId,
    lang: &str,
) -> Result<(), anyhow::Error> {
    let text = t_lang(
        ctx,
        lang,
        "language_prompt",
        "🌐 Please choose your language before continuing.",
    );
    i18n::send_message_with_json_keyboard(
        ctx,
        chat_id,
        "language_prompt",
        text,
        language_keyboard_json(ctx, lang),
    )
    .await?;
    Ok(())
}

fn language_keyboard_json(ctx: &AppContext, current_lang: &str) -> Value {
    let specs = ctx
        .texts
        .read()
        .map(|texts| language_button_specs_from_texts(&texts, current_lang))
        .unwrap_or_default();
    let mut rows = specs
        .chunks(2)
        .map(|chunk| {
            chunk
                .iter()
                .map(|(label, callback)| {
                    let key = callback
                        .strip_prefix("lang:")
                        .map(|code| format!("language_btn_{code}"))
                        .unwrap_or_else(|| "language_btn".to_string());
                    let parts = i18n::button_parts_for_key(ctx, &key, label.clone());
                    let mut button = json!({
                        "text": parts.text,
                        "callback_data": callback,
                    });
                    if let Some(icon_id) = parts.icon_custom_emoji_id
                        && let Some(obj) = button.as_object_mut()
                    {
                        obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
                    }
                    button
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    rows.push(vec![i18n::inline_button_callback_json(
        ctx,
        current_lang,
        "back_btn",
        "⬅️ Quay lại",
        "start:menu",
    )]);
    json!({ "inline_keyboard": rows })
}

fn language_button_specs_from_texts(texts: &BotTexts, current_lang: &str) -> Vec<(String, String)> {
    texts
        .enabled_languages()
        .into_iter()
        .map(|language| {
            let key = format!("language_btn_{}", language.code);
            let label = texts.get_lang(&key, current_lang, &language.label);
            (label, format!("lang:{}", language.code))
        })
        .collect()
}

fn start_entry_from_language_choice(
    texts: &BotTexts,
    preferred_language: Option<&str>,
    telegram_language_code: Option<&str>,
) -> StartEntry {
    if let Some(preferred) = preferred_language
        .map(str::trim)
        .filter(|preferred| !preferred.is_empty())
    {
        StartEntry::Menu(texts.normalize_language(Some(preferred)))
    } else {
        StartEntry::LanguagePrompt(texts.normalize_language(telegram_language_code))
    }
}

fn start_menu_action_from_text(
    texts: &BotTexts,
    lang: &str,
    text: &str,
) -> Option<StartMenuAction> {
    let input_variants = i18n::button_text_match_variants(text);
    start_menu_match_languages(texts, lang)
        .into_iter()
        .find_map(|candidate_lang| {
            start_menu_action_labels(texts, &candidate_lang)
                .into_iter()
                .find_map(|(action, label)| {
                    let label_variants = i18n::button_text_match_variants(&label);
                    label_variants
                        .iter()
                        .any(|label| input_variants.iter().any(|input| input == label))
                        .then_some(action)
                })
        })
}

fn start_menu_match_languages(texts: &BotTexts, lang: &str) -> Vec<String> {
    let mut languages = vec![texts.normalize_language(Some(lang))];
    for language in texts.enabled_languages() {
        if !languages
            .iter()
            .any(|candidate| candidate == &language.code)
        {
            languages.push(language.code);
        }
    }
    languages
}

fn start_menu_action_labels(texts: &BotTexts, lang: &str) -> Vec<(StartMenuAction, String)> {
    vec![
        (
            StartMenuAction::Shop,
            texts.get_lang("start_btn_shop", lang, "🛒 Shop"),
        ),
        (
            StartMenuAction::Topup,
            texts.get_lang("start_btn_topup", lang, "💰 Top up"),
        ),
        (
            StartMenuAction::Wallet,
            texts.get_lang("start_btn_wallet", lang, "💳 Wallet"),
        ),
        (
            StartMenuAction::Orders,
            texts.get_lang("start_btn_purchased", lang, "📦 Purchased"),
        ),
        (
            StartMenuAction::Orders,
            texts.get_lang("start_btn_orders", lang, "📋 Recent orders"),
        ),
        (
            StartMenuAction::TopupHistory,
            texts.get_lang("start_btn_topup_history", lang, "📜 Top-up history"),
        ),
        (
            StartMenuAction::ApiIntegration,
            texts.get_lang("start_btn_api_integration", lang, "🔌 API integration"),
        ),
        (
            StartMenuAction::FacebookUnlock,
            texts.get_lang("start_btn_facebook_unlock", lang, "Mở khóa facebook"),
        ),
        (
            StartMenuAction::SupportHistory,
            texts.get_lang("start_btn_support_history", lang, "🧾 Lịch sử yêu cầu"),
        ),
        (
            StartMenuAction::Help,
            texts.get_lang("start_btn_help", lang, "Help"),
        ),
        (
            StartMenuAction::Language,
            texts.get_lang("start_btn_language", lang, "🌐 Language"),
        ),
    ]
}

#[async_trait::async_trait]
impl AppPlugin for StartCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdStart"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "start".to_string(),
            description: "Start".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("");

        let lang = if let Some(user) = msg.from() {
            preferred_or_telegram_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
        } else {
            ctx.normalize_language_code(None)
        };
        let keyboard_action = ctx
            .texts
            .read()
            .ok()
            .and_then(|texts| start_menu_action_from_text(&texts, &lang, text));
        if let Some(action) = keyboard_action {
            match action {
                StartMenuAction::Shop => {
                    cmd_shop::send_products(
                        ctx.clone(),
                        ctx.bot.clone(),
                        msg.chat.id,
                        0,
                        None,
                        &lang,
                    )
                    .await?;
                }
                StartMenuAction::Topup => {
                    if msg.from().is_some() {
                        cmd_wallet::prompt_topup_amount(&ctx, msg.chat.id, dialogue.clone(), &lang)
                            .await?;
                    } else {
                        send_message_with_start_reply_keyboard(
                            &ctx,
                            msg.chat.id,
                            "user_unknown",
                            t_lang(&ctx, &lang, "user_unknown", "Cannot identify user."),
                            &lang,
                        )
                        .await?;
                    }
                }
                StartMenuAction::Wallet => {
                    if let Some(user) = msg.from() {
                        cmd_wallet::show_wallet(&ctx, msg.chat.id, user.id.0 as i64).await?;
                    } else {
                        send_message_with_start_reply_keyboard(
                            &ctx,
                            msg.chat.id,
                            "user_unknown",
                            t_lang(&ctx, &lang, "user_unknown", "Cannot identify user."),
                            &lang,
                        )
                        .await?;
                    }
                }
                StartMenuAction::Orders => {
                    cmd_orders::send_orders(ctx.clone(), ctx.bot.clone(), msg.chat.id, msg.from())
                        .await?;
                }
                StartMenuAction::TopupHistory => {
                    if let Some(user) = msg.from() {
                        cmd_wallet::show_topup_history(
                            &ctx,
                            msg.chat.id,
                            None,
                            user.id.0 as i64,
                            &lang,
                        )
                        .await?;
                    } else {
                        send_message_with_start_reply_keyboard(
                            &ctx,
                            msg.chat.id,
                            "user_unknown",
                            t_lang(&ctx, &lang, "user_unknown", "Cannot identify user."),
                            &lang,
                        )
                        .await?;
                    }
                }
                StartMenuAction::ApiIntegration => {
                    if let Some(user) = msg.from() {
                        cmd_shop::send_api_integration_page(
                            ctx.clone(),
                            msg.chat.id,
                            user.id.0 as i64,
                            false,
                            &lang,
                        )
                        .await?;
                    } else {
                        send_message_with_start_reply_keyboard(
                            &ctx,
                            msg.chat.id,
                            "user_unknown",
                            t_lang(&ctx, &lang, "user_unknown", "Cannot identify user."),
                            &lang,
                        )
                        .await?;
                    }
                }
                StartMenuAction::FacebookUnlock => {
                    send_facebook_unlock_type_prompt(&ctx, msg.chat.id, &lang).await?;
                }
                StartMenuAction::SupportHistory => {
                    cmd_orders::send_support_request_history(
                        ctx.clone(),
                        msg.chat.id,
                        msg.from(),
                    )
                    .await?;
                }
                StartMenuAction::Help => {
                    let msg_text = t_lang(
                        &ctx,
                        &lang,
                        "help",
                        "❓ Quick help:\n/shop - products\n/orders - your orders\n/help - help.",
                    );
                    send_message_with_start_reply_keyboard(
                        &ctx,
                        msg.chat.id,
                        "help",
                        msg_text,
                        &lang,
                    )
                    .await?;
                }
                StartMenuAction::Language => {
                    send_language_prompt(&ctx, msg.chat.id, &lang).await?;
                }
            }
            if !matches!(action, StartMenuAction::Topup) {
                let _ = dialogue.update(State::Idle).await;
            }
            return Ok(true);
        }

        if !text.starts_with('/')
            && let State::FacebookUnlockEnterUid { lock_type } =
                dialogue.get().await?.unwrap_or_default()
        {
            handle_facebook_unlock_uid_message(&ctx, msg, dialogue, &lang, lock_type).await?;
            return Ok(true);
        }

        let start_payload = if text.starts_with("/start ") {
            text.split_whitespace().nth(1).unwrap_or("")
        } else {
            ""
        };

        if text.starts_with("/start") && start_payload.is_empty() {
            if let Some(user) = msg.from() {
                upsert_subscriber_from_user(&ctx, user, msg.chat.id.0, None).await;
            }

            let preferred_language = if let Some(user) = msg.from() {
                users_repo::preferred_language(&ctx.pool, user.id.0 as i64)
                    .await
                    .ok()
                    .flatten()
            } else {
                None
            };
            let telegram_language_code = msg.from().and_then(|user| user.language_code.as_deref());
            let start_entry = ctx
                .texts
                .read()
                .map(|texts| {
                    start_entry_from_language_choice(
                        &texts,
                        preferred_language.as_deref(),
                        telegram_language_code,
                    )
                })
                .unwrap_or_else(|_| {
                    StartEntry::LanguagePrompt(ctx.normalize_language_code(telegram_language_code))
                });
            match start_entry {
                StartEntry::Menu(lang) => {
                    if let Some(user) = msg.from() {
                        if user_has_joined_required_channel(&ctx, user.id).await {
                            send_start_menu(
                                &ctx,
                                msg.chat.id,
                                &lang,
                                start_user_is_admin(&ctx, user.id.0 as i64),
                            )
                            .await?;
                        } else {
                            send_required_channel_prompt(&ctx, msg.chat.id, &lang).await?;
                        }
                    } else {
                        send_start_menu(&ctx, msg.chat.id, &lang, false).await?;
                    }
                }
                StartEntry::LanguagePrompt(lang) => {
                    send_language_prompt(&ctx, msg.chat.id, &lang).await?;
                }
            }
            let _ = dialogue.update(State::Idle).await;
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
        let Some(data) = q.data.clone() else {
            return Ok(false);
        };

        if data == "start:menu" {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                if user_has_joined_required_channel(&ctx, q.from.id).await {
                    send_start_menu(
                        &ctx,
                        msg.chat().id,
                        &lang,
                        start_user_is_admin(&ctx, q.from.id.0 as i64),
                    )
                    .await?;
                } else {
                    send_required_channel_prompt(&ctx, msg.chat().id, &lang).await?;
                }
            }
            return Ok(true);
        }

        if let Some(lang) = data.strip_prefix("lang:") {
            if !ctx.is_supported_language(lang) {
                let fallback_lang = ctx.normalize_language_code(None);
                let ack = t_lang(
                    &ctx,
                    &fallback_lang,
                    "action_invalid",
                    "Invalid action. Please try again.",
                );
                let _ = ctx.bot.answer_callback_query(q.id.clone()).text(ack).await;
                return Ok(true);
            }
            let lang = ctx.normalize_language_code(Some(lang));
            if let Some(msg) = &q.message {
                upsert_subscriber_from_user(&ctx, &q.from, msg.chat().id.0, Some(lang.clone()))
                    .await;
            }
            let _ =
                users_repo::update_preferred_language(&ctx.pool, q.from.id.0 as i64, &lang).await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                if user_has_joined_required_channel(&ctx, q.from.id).await {
                    send_start_menu(
                        &ctx,
                        msg.chat().id,
                        &lang,
                        start_user_is_admin(&ctx, q.from.id.0 as i64),
                    )
                    .await?;
                } else {
                    send_required_channel_prompt(&ctx, msg.chat().id, &lang).await?;
                }
            }
            return Ok(true);
        }

        if data == "start:help" {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                let msg_text = t_lang(
                    &ctx,
                    &lang,
                    "help",
                    "❓ Quick help:\n/shop - products\n/orders - your orders\n/help - help.",
                );
                ctx.bot.send_message(msg.chat().id, msg_text).await?;
            }
            return Ok(true);
        }

        if data == SUPPORT_HISTORY_CALLBACK {
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                cmd_orders::send_support_request_history(
                    ctx.clone(),
                    msg.chat().id,
                    Some(&q.from),
                )
                .await?;
            }
            return Ok(true);
        }

        if data == "start:language" {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                send_language_prompt(&ctx, msg.chat().id, &lang).await?;
            }
            return Ok(true);
        }

        if data == FACEBOOK_UNLOCK_CALLBACK {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                send_facebook_unlock_type_prompt(&ctx, msg.chat().id, &lang).await?;
            }
            return Ok(true);
        }

        if data == FACEBOOK_UNLOCK_TYPE_282_CALLBACK {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                dialogue
                    .update(State::FacebookUnlockEnterUid {
                        lock_type: "282".to_string(),
                    })
                    .await?;
                send_facebook_unlock_uid_prompt(&ctx, msg.chat().id, &lang).await?;
            }
            return Ok(true);
        }

        if data == FACEBOOK_UNLOCK_TYPE_956_CALLBACK {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                let text = t_lang(
                    &ctx,
                    &lang,
                    "facebook_unlock_next_step_pending",
                    "Mình đã ghi nhận lựa chọn. Vui lòng chờ hướng dẫn tiếp theo.",
                );
                ctx.bot.send_message(msg.chat().id, text).await?;
            }
            return Ok(true);
        }

        if data == FACEBOOK_UNLOCK_ENTER_UID_CALLBACK {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                dialogue
                    .update(State::FacebookUnlockEnterUid {
                        lock_type: "282".to_string(),
                    })
                    .await?;
                send_facebook_unlock_uid_prompt(&ctx, msg.chat().id, &lang).await?;
            }
            return Ok(true);
        }

        if data == FACEBOOK_UNLOCK_NO_UID_CALLBACK {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                dialogue
                    .update(State::FacebookUnlockEnterUid {
                        lock_type: "282".to_string(),
                    })
                    .await?;
                send_facebook_unlock_uid_help(&ctx, msg.chat().id, &lang).await?;
            }
            return Ok(true);
        }

        if data == JOIN_CHECK_CALLBACK {
            let lang = preferred_or_telegram_lang(
                &ctx,
                q.from.id.0 as i64,
                q.from.language_code.as_deref(),
            )
            .await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                if user_has_joined_required_channel(&ctx, q.from.id).await {
                    send_start_menu(
                        &ctx,
                        msg.chat().id,
                        &lang,
                        start_user_is_admin(&ctx, q.from.id.0 as i64),
                    )
                    .await?;
                } else {
                    let text = t_lang(
                        &ctx,
                        &lang,
                        "required_channel_not_joined",
                        "⚠️ You have not joined the channel yet. Please join, then press “I joined” again.",
                    );
                    let channel_url = required_channel_url(&ctx);
                    ctx.bot
                        .send_message(msg.chat().id, text)
                        .reply_markup(join_required_channel_keyboard(&ctx, &channel_url, &lang))
                        .await?;
                }
            }
            return Ok(true);
        }

        if data == "start:orders" {
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                cmd_orders::send_orders(ctx.clone(), ctx.bot.clone(), msg.chat().id, Some(&q.from))
                    .await?;
            }
            return Ok(true);
        }

        if data == "start:wallet" {
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                cmd_wallet::show_wallet(&ctx, msg.chat().id, q.from.id.0 as i64).await?;
            }
            return Ok(true);
        }

        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::{BotTexts, LanguageInfo};
    use crate::config::Config;
    use std::collections::HashMap;
    use teloxide::Bot;
    use teloxide::types::UserId;

    #[test]
    fn required_channel_enabled_accepts_common_truthy_values() {
        assert!(required_channel_enabled_value("1"));
        assert!(required_channel_enabled_value("true"));
        assert!(required_channel_enabled_value("ON"));
        assert!(required_channel_enabled_value("yes"));
        assert!(!required_channel_enabled_value(""));
        assert!(!required_channel_enabled_value("0"));
        assert!(!required_channel_enabled_value("false"));
    }

    #[test]
    fn normalize_required_channel_uses_id_or_t_me_url() {
        assert_eq!(
            normalize_required_channel_id("@zvwboo", "https://t.me/other"),
            Some("@zvwboo".to_string())
        );
        assert_eq!(
            normalize_required_channel_id("", "https://t.me/zvwboo"),
            Some("@zvwboo".to_string())
        );
        assert_eq!(
            normalize_required_channel_id("zvwboo", ""),
            Some("@zvwboo".to_string())
        );
        assert_eq!(normalize_required_channel_id("", ""), None);
    }

    #[test]
    fn join_check_callback_key_is_stable() {
        assert_eq!(JOIN_CHECK_CALLBACK, "start:check_join");
    }

    #[test]
    fn facebook_unlock_callback_keys_are_stable() {
        assert_eq!(FACEBOOK_UNLOCK_CALLBACK, "start:facebook_unlock");
        assert_eq!(FACEBOOK_UNLOCK_TYPE_282_CALLBACK, "facebook_unlock:type:282");
        assert_eq!(FACEBOOK_UNLOCK_TYPE_956_CALLBACK, "facebook_unlock:type:956");
        assert_eq!(
            FACEBOOK_UNLOCK_ENTER_UID_CALLBACK,
            "facebook_unlock:enter_uid:282"
        );
        assert_eq!(FACEBOOK_UNLOCK_NO_UID_CALLBACK, "facebook_unlock:no_uid:282");
    }

    #[tokio::test]
    async fn raw_start_payload_keeps_custom_emoji_entities_for_message_text() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            HashMap::from([
                ("telegram_i18n_emojis_enabled".to_string(), "1".to_string()),
                (
                    "telegram_i18n_emojis".to_string(),
                    r#"{"start":{"fallback":"👋","custom_emoji_id":"5368324170671202286"}}"#
                        .to_string(),
                ),
            ]),
            BotTexts::default(),
            vec![],
        );

        let payload = i18n::message_payload_with_json_keyboard(
            &ctx,
            teloxide::types::ChatId(1),
            "start",
            "Chào mừng bạn đến shop! Nhấn nút bên dưới để khám phá sản phẩm ngay.",
            json!({ "inline_keyboard": [] }),
        )
        .unwrap();

        assert_eq!(
            payload["text"],
            "👋 Chào mừng bạn đến shop! Nhấn nút bên dưới để khám phá sản phẩm ngay."
        );
        assert_eq!(payload["entities"][0]["type"], "custom_emoji");
        assert_eq!(
            payload["entities"][0]["custom_emoji_id"],
            "5368324170671202286"
        );
    }

    #[test]
    fn telegram_language_code_is_normalized_to_supported_language() {
        let texts = BotTexts::default();

        assert_eq!(texts.normalize_language(Some("vi")), "vi");
        assert_eq!(texts.normalize_language(Some("vi-VN")), "vi");
        assert_eq!(texts.normalize_language(Some("en")), "en");
        assert_eq!(texts.normalize_language(Some("fr")), "en");
        assert_eq!(texts.normalize_language(None), "en");
    }

    #[test]
    fn language_button_specs_follow_enabled_language_registry() {
        let texts = BotTexts::from_language_maps(
            vec![
                LanguageInfo {
                    code: "vi".to_string(),
                    label: "Tiếng Việt".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
                LanguageInfo {
                    code: "en".to_string(),
                    label: "English".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
                LanguageInfo {
                    code: "th".to_string(),
                    label: "ไทย".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
            ],
            HashMap::from([(
                "en".to_string(),
                HashMap::from([("language_btn_th".to_string(), "🇹🇭 Thai".to_string())]),
            )]),
        );

        let specs = language_button_specs_from_texts(&texts, "en");

        assert_eq!(
            specs,
            vec![
                ("Tiếng Việt".to_string(), "lang:vi".to_string()),
                ("English".to_string(), "lang:en".to_string()),
                ("🇹🇭 Thai".to_string(), "lang:th".to_string()),
            ]
        );
    }

    #[test]
    fn start_menu_button_specs_put_shop_first_and_include_all_main_actions() {
        let texts = BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "en".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([
                    ("start_btn_shop".to_string(), "🛒 Xem sản phẩm".to_string()),
                    (
                        "start_btn_facebook_unlock".to_string(),
                        "Mở khóa facebook".to_string(),
                    ),
                    ("start_btn_topup".to_string(), "💰 Nạp tiền".to_string()),
                    ("start_btn_wallet".to_string(), "💳 Ví tiền".to_string()),
                    ("start_btn_purchased".to_string(), "📦 Đã mua".to_string()),
                    (
                        "start_btn_topup_history".to_string(),
                        "📜 Lịch sử nạp".to_string(),
                    ),
                    (
                        "start_btn_api_integration".to_string(),
                        "🔌 Tích hợp API".to_string(),
                    ),
                    ("start_btn_help".to_string(), "Hướng dẫn".to_string()),
                    ("start_btn_language".to_string(), "🌐 Ngôn ngữ".to_string()),
                ]),
            )]),
        );

        let rows = start_menu_button_specs_from_texts(&texts, "vi");

        assert_eq!(
            rows,
            vec![
                vec![("🛒 Xem sản phẩm".to_string(), "start:shop".to_string())],
                vec![(
                    "Mở khóa facebook".to_string(),
                    FACEBOOK_UNLOCK_CALLBACK.to_string(),
                )],
                vec![
                    ("💰 Nạp tiền".to_string(), "wallet:topup".to_string()),
                    ("💳 Ví tiền".to_string(), "start:wallet".to_string()),
                ],
                vec![
                    ("📦 Đã mua".to_string(), "start:orders".to_string()),
                    (
                        "📜 Lịch sử nạp".to_string(),
                        "wallet:topup_history".to_string()
                    ),
                ],
                vec![
                    ("🔌 Tích hợp API".to_string(), "shop_api".to_string()),
                    ("Hướng dẫn".to_string(), "start:help".to_string()),
                ],
                vec![("🌐 Ngôn ngữ".to_string(), "start:language".to_string())],
            ]
        );
    }

    #[test]
    fn reply_keyboard_specs_match_main_start_actions() {
        let texts = BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "en".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([
                    ("start_btn_shop".to_string(), "🛒 Xem sản phẩm".to_string()),
                    (
                        "start_btn_facebook_unlock".to_string(),
                        "Mở khóa facebook".to_string(),
                    ),
                    ("start_btn_topup".to_string(), "💰 Nạp tiền".to_string()),
                    ("start_btn_wallet".to_string(), "💳 Ví tiền".to_string()),
                    ("start_btn_purchased".to_string(), "📦 Đã mua".to_string()),
                    (
                        "start_btn_topup_history".to_string(),
                        "📜 Lịch sử nạp".to_string(),
                    ),
                    (
                        "start_btn_api_integration".to_string(),
                        "🔌 Tích hợp API".to_string(),
                    ),
                    ("start_btn_help".to_string(), "Hướng dẫn".to_string()),
                    ("start_btn_language".to_string(), "🌐 Ngôn ngữ".to_string()),
                ]),
            )]),
        );

        let rows = start_reply_keyboard_specs_from_texts(&texts, "vi");

        assert_eq!(
            rows,
            vec![
                vec!["🛒 Xem sản phẩm".to_string()],
                vec!["Mở khóa facebook".to_string()],
                vec!["💰 Nạp tiền".to_string(), "💳 Ví tiền".to_string()],
                vec!["📦 Đã mua".to_string(), "📜 Lịch sử nạp".to_string()],
                vec!["🔌 Tích hợp API".to_string(), "Hướng dẫn".to_string()],
                vec!["🌐 Ngôn ngữ".to_string()],
            ]
        );
    }

    #[tokio::test]
    async fn reply_keyboard_json_hides_custom_emoji_id_placeholders() {
        let ctx = test_ctx_with_texts(BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "en".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([
                    (
                        "start_btn_shop".to_string(),
                        "{6172437452590944785} 🛒 Xem sản phẩm".to_string(),
                    ),
                    (
                        "start_btn_wallet".to_string(),
                        "{6113868675792507468} 💳 Ví tiền".to_string(),
                    ),
                ]),
            )]),
        ));

        let keyboard = start_reply_keyboard_json(&ctx, "vi");
        let rows = keyboard["keyboard"].as_array().unwrap();

        assert_eq!(rows[0][0]["text"], "🛒 Xem sản phẩm");
        assert_eq!(rows[0][0]["icon_custom_emoji_id"], "6172437452590944785");
        assert_eq!(rows[1][1]["text"], "💳 Ví tiền");
        assert_eq!(rows[1][1]["icon_custom_emoji_id"], "6113868675792507468");
        assert!(!keyboard.to_string().contains("{6172437452590944785}"));
        assert!(!keyboard.to_string().contains("{6113868675792507468}"));
    }

    #[test]
    fn reply_keyboard_text_maps_to_start_action() {
        let texts = BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "en".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([
                    ("start_btn_topup".to_string(), "💰 Nạp tiền".to_string()),
                    (
                        "start_btn_facebook_unlock".to_string(),
                        "Mở khóa facebook".to_string(),
                    ),
                    (
                        "start_btn_topup_history".to_string(),
                        "📜 Lịch sử nạp".to_string(),
                    ),
                    (
                        "start_btn_api_integration".to_string(),
                        "🔌 Tích hợp API".to_string(),
                    ),
                    (
                        "start_btn_orders".to_string(),
                        "📋 Đơn hàng gần đây".to_string(),
                    ),
                    ("start_btn_language".to_string(), "🌐 Ngôn ngữ".to_string()),
                ]),
            )]),
        );

        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "💰 Nạp tiền"),
            Some(StartMenuAction::Topup)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "Mở khóa facebook"),
            Some(StartMenuAction::FacebookUnlock)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "📜 Lịch sử nạp"),
            Some(StartMenuAction::TopupHistory)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "🔌 Tích hợp API"),
            Some(StartMenuAction::ApiIntegration)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "📋 Đơn hàng gần đây"),
            Some(StartMenuAction::Orders)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "🌐 Ngôn ngữ"),
            Some(StartMenuAction::Language)
        );
    }

    #[test]
    fn reply_keyboard_text_maps_rendered_custom_emoji_buttons_to_start_action() {
        let texts = BotTexts::from_language_maps(
            vec![LanguageInfo {
                code: "vi".to_string(),
                label: "Tiếng Việt".to_string(),
                fallback: "en".to_string(),
                enabled: true,
            }],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([
                    (
                        "start_btn_shop".to_string(),
                        "{6172437452590944785} 🛒 Xem sản phẩm".to_string(),
                    ),
                    (
                        "start_btn_wallet".to_string(),
                        "{6113868675792507468} 💳 Ví tiền".to_string(),
                    ),
                ]),
            )]),
        );

        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "🛒 Xem sản phẩm"),
            Some(StartMenuAction::Shop)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "✨ 💳 Ví tiền"),
            Some(StartMenuAction::Wallet)
        );
        assert_eq!(
            start_menu_action_from_text(&texts, "vi", "Ví tiền"),
            Some(StartMenuAction::Wallet)
        );
    }

    #[test]
    fn reply_keyboard_text_maps_enabled_language_labels_when_current_lang_differs() {
        let texts = BotTexts::from_language_maps(
            vec![
                LanguageInfo {
                    code: "en".to_string(),
                    label: "English".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
                LanguageInfo {
                    code: "vi".to_string(),
                    label: "Tiếng Việt".to_string(),
                    fallback: "en".to_string(),
                    enabled: true,
                },
            ],
            HashMap::from([(
                "vi".to_string(),
                HashMap::from([("start_btn_help".to_string(), "Hướng dẫn".to_string())]),
            )]),
        );

        assert_eq!(
            start_menu_action_from_text(&texts, "en", "Hướng dẫn"),
            Some(StartMenuAction::Help)
        );
    }

    #[tokio::test]
    async fn language_keyboard_has_back_to_main_menu() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            HashMap::new(),
            BotTexts::default(),
            vec![],
        );

        let keyboard = language_keyboard_json(&ctx, "vi");
        let rows = keyboard["inline_keyboard"].as_array().unwrap();
        let last_row = rows.last().unwrap().as_array().unwrap();

        assert_eq!(last_row[0]["callback_data"], "start:menu");
    }

    #[tokio::test]
    async fn upsert_subscriber_from_callback_user_saves_preferred_language() {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        let ctx = AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            HashMap::new(),
            BotTexts::default(),
            vec![],
        );
        let user = User {
            id: UserId(42),
            is_bot: false,
            first_name: "Nam".to_string(),
            last_name: None,
            username: Some("nam".to_string()),
            language_code: Some("en".to_string()),
            is_premium: false,
            added_to_attachment_menu: false,
        };

        upsert_subscriber_from_user(&ctx, &user, 420, Some("vi".to_string())).await;

        assert_eq!(
            users_repo::preferred_language(&ctx.pool, 42).await.unwrap(),
            Some("vi".to_string())
        );
    }

    #[test]
    fn start_entry_uses_saved_language_before_telegram_language() {
        let texts = BotTexts::default();

        let entry = start_entry_from_language_choice(&texts, Some("vi"), Some("en"));

        assert_eq!(entry, StartEntry::Menu("vi".to_string()));
    }

    #[test]
    fn start_entry_asks_for_language_when_user_has_no_saved_language() {
        let texts = BotTexts::default();

        let entry = start_entry_from_language_choice(&texts, None, Some("vi"));

        assert_eq!(entry, StartEntry::LanguagePrompt("vi".to_string()));
    }

    fn test_ctx_with_texts(texts: BotTexts) -> Arc<AppContext> {
        let pool = sqlx::sqlite::SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        AppContext::new(
            Bot::new("test-token"),
            pool,
            Config {
                telegram_token: "test-token".to_string(),
                database_url: "sqlite::memory:".to_string(),
                bank_name: "VCB".to_string(),
                bank_account: Some("123".to_string()),
                bank_account_name: None,
                webhook_secret: "secret".to_string(),
                admin_jwt_secret: "12345678901234567890123456789012".to_string(),
                admin_setup_code: "setupcode".to_string(),
                admin_cookie_secure: false,
                base_url: None,
                i18n_dir: "i18n".to_string(),
                port: 8080,
                crypto: crate::config::CryptoConfig::default(),
            },
            HashMap::new(),
            texts,
            vec![],
        )
    }
}
