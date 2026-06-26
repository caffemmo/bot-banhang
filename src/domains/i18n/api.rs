use axum::{
    Json, Router,
    extract::{Path, State},
    routing::{get, post},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeSet, HashMap};
use std::sync::Arc;

use crate::app::{AppContext, I18nEmojiPrefix, custom_emoji_map_from_values};
use crate::bot::texts::{BotLanguageExport, BotTexts, LanguageInfo};
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::configs::repo as configs_repo;
use crate::domains::i18n::repo;

const REQUIRED_START_BUTTON_KEYS: &[&str] = &[
    "fbunlock_btn_back",
    "fbunlock_btn_admin_refund",
    "fbunlock_btn_admin_reject_refund",
    "fbunlock_btn_approve_worker",
    "fbunlock_btn_cancel_case",
    "fbunlock_btn_confirm_done",
    "fbunlock_btn_customer",
    "fbunlock_btn_customer_my_cases",
    "fbunlock_btn_dispute",
    "fbunlock_btn_message_customer",
    "fbunlock_btn_message_worker",
    "fbunlock_btn_quote_case",
    "fbunlock_btn_reject_worker",
    "fbunlock_btn_worker",
    "fbunlock_btn_worker_apply",
    "fbunlock_btn_worker_cases",
    "fbunlock_btn_worker_my_cases",
    "fbunlock_btn_worker_done",
    "fbunlock_btn_worker_failed",
    "start_btn_api_integration",
    "start_btn_affiliate_register",
    "start_btn_child_bot",
    "start_btn_facebook_unlock",
    "start_btn_help",
    "start_btn_language",
    "start_btn_orders",
    "start_btn_purchased",
    "start_btn_shop",
    "start_btn_topup",
    "start_btn_topup_history",
    "start_btn_tut",
    "start_btn_viameta",
    "start_btn_wallet",
];

fn required_start_button_default(key: &str) -> Option<&'static str> {
    match key {
        "fbunlock_btn_back" => Some("⬅️ Back"),
        "fbunlock_btn_admin_refund" => Some("💸 Refund customer"),
        "fbunlock_btn_admin_reject_refund" => Some("↩️ Reject refund"),
        "fbunlock_btn_approve_worker" => Some("✅ Approve worker"),
        "fbunlock_btn_cancel_case" => Some("❌ Cancel case"),
        "fbunlock_btn_confirm_done" => Some("✅ Confirm success"),
        "fbunlock_btn_customer" => Some("🙋 I need Facebook unlock"),
        "fbunlock_btn_customer_my_cases" => Some("🧾 My cases"),
        "fbunlock_btn_dispute" => Some("⚠️ Dispute"),
        "fbunlock_btn_message_customer" => Some("💬 Message customer"),
        "fbunlock_btn_message_worker" => Some("💬 Message worker"),
        "fbunlock_btn_quote_case" => Some("💬 Quote case"),
        "fbunlock_btn_reject_worker" => Some("❌ Reject worker"),
        "fbunlock_btn_worker" => Some("🧑‍💻 I provide this service"),
        "fbunlock_btn_worker_apply" => Some("📝 Apply as service worker"),
        "fbunlock_btn_worker_cases" => Some("📋 View cases to quote"),
        "fbunlock_btn_worker_my_cases" => Some("🧾 My cases"),
        "fbunlock_btn_worker_done" => Some("✅ Mark completed"),
        "fbunlock_btn_worker_failed" => Some("⚠️ Cannot handle"),
        "start_btn_api_integration" => Some("🔌 API integration"),
        "start_btn_affiliate_register" => Some("🤝 Register affiliate"),
        "start_btn_child_bot" => Some("🤖 Create child bot"),
        "start_btn_facebook_unlock" => Some("🔓 Unlock Facebook"),
        "start_btn_help" => Some("Help"),
        "start_btn_language" => Some("🌐 Language"),
        "start_btn_orders" => Some("📋 Recent orders"),
        "start_btn_purchased" => Some("📦 Purchased"),
        "start_btn_shop" => Some("🛒 Shop"),
        "start_btn_topup" => Some("💰 Top up"),
        "start_btn_topup_history" => Some("📜 Top-up history"),
        "start_btn_tut" => Some("📚 TUT"),
        "start_btn_viameta" => Some("✅ Verification service"),
        "start_btn_wallet" => Some("💳 Wallet"),
        _ => None,
    }
}

#[derive(Debug, Deserialize)]
pub struct ImportLanguagePayload {
    pub format: String,
    pub content: String,
}

#[derive(Debug, Serialize)]
pub struct ImportLanguageResponse {
    pub language: LanguageInfo,
    pub imported_keys: usize,
    pub warnings: Vec<String>,
}

#[derive(Debug, Serialize)]
pub struct LanguageListItem {
    pub code: String,
    pub label: String,
    pub fallback: String,
    pub enabled: bool,
    pub key_count: usize,
}

#[derive(Debug, Serialize)]
pub struct LanguageDetailResponse {
    pub language: LanguageInfo,
    pub keys: Vec<String>,
    pub bot: HashMap<String, String>,
    pub fallback_bot: HashMap<String, String>,
    pub emojis_enabled: bool,
    pub emojis: HashMap<String, I18nEmojiPrefix>,
    pub custom_emojis: HashMap<String, String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateLanguagePayload {
    pub bot: HashMap<String, String>,
    #[serde(default)]
    pub emojis: Option<HashMap<String, Value>>,
    #[serde(default)]
    pub custom_emojis: Option<HashMap<String, Value>>,
}

pub async fn list_languages(
    State(ctx): State<Arc<AppContext>>,
) -> ApiResult<Vec<LanguageListItem>> {
    let texts = ctx.texts.read().unwrap().clone();
    let items = texts
        .languages()
        .into_iter()
        .map(|language| {
            let key_count = texts
                .export_language(&language.code)
                .map(|export| export.bot.len())
                .unwrap_or_default();
            LanguageListItem {
                code: language.code,
                label: language.label,
                fallback: language.fallback,
                enabled: language.enabled,
                key_count,
            }
        })
        .collect();

    Ok(ok(items))
}

pub async fn import_language(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<ImportLanguagePayload>,
) -> ApiResult<ImportLanguageResponse> {
    let import = BotTexts::parse_language_import(&payload.format, &payload.content)
        .map_err(ApiError::validation)?;
    let imported_keys = repo::save_language_import(&ctx.config.i18n_dir, &import)
        .map_err(|err| ApiError::internal(format!("failed to import language: {err}")))?;
    reload_texts(&ctx)?;

    Ok(ok(ImportLanguageResponse {
        language: import.language,
        imported_keys,
        warnings: Vec::new(),
    }))
}

pub async fn export_language(
    State(ctx): State<Arc<AppContext>>,
    Path(code): Path<String>,
) -> ApiResult<BotLanguageExport> {
    let texts = ctx.texts.read().unwrap().clone();
    let export = texts
        .export_language(&code)
        .ok_or_else(|| ApiError::not_found(format!("language not found: {code}")))?;
    Ok(ok(export))
}

pub async fn language_detail(
    State(ctx): State<Arc<AppContext>>,
    Path(code): Path<String>,
) -> ApiResult<LanguageDetailResponse> {
    let texts = ctx.texts.read().unwrap().clone();
    let language = texts
        .language_by_code(&code)
        .ok_or_else(|| ApiError::not_found(format!("language not found: {code}")))?;
    let bot = texts
        .export_language(&language.code)
        .map(|export| export.bot)
        .unwrap_or_default();
    let mut keys = BTreeSet::new();
    keys.extend(texts.translation_base_keys());
    keys.extend(bot.keys().cloned());
    keys.extend(REQUIRED_START_BUTTON_KEYS.iter().map(|key| (*key).to_string()));
    let keys: Vec<String> = keys.into_iter().collect();
    let fallback_bot = keys
        .iter()
        .filter_map(|key| {
            let value = texts.get_lang(
                key,
                &language.fallback,
                required_start_button_default(key).unwrap_or(""),
            );
            if value.is_empty() {
                None
            } else {
                Some((key.clone(), value))
            }
        })
        .collect();

    Ok(ok(LanguageDetailResponse {
        language,
        keys,
        bot,
        fallback_bot,
        emojis_enabled: ctx.i18n_emojis_enabled(),
        emojis: load_i18n_emoji_map(&ctx),
        custom_emojis: ctx.custom_emoji_map(),
    }))
}

pub async fn update_language(
    State(ctx): State<Arc<AppContext>>,
    Path(code): Path<String>,
    Json(payload): Json<UpdateLanguagePayload>,
) -> ApiResult<ImportLanguageResponse> {
    let texts = ctx.texts.read().unwrap().clone();
    let language = texts
        .language_by_code(&code)
        .ok_or_else(|| ApiError::not_found(format!("language not found: {code}")))?;
    let updated_keys =
        repo::save_language_texts(&ctx.config.i18n_dir, &language.code, &payload.bot)
            .map_err(|err| ApiError::internal(format!("failed to update language: {err}")))?;
    if let Some(emojis) = payload.emojis {
        save_i18n_emoji_map(&ctx, emojis).await?;
    }
    if let Some(custom_emojis) = payload.custom_emojis {
        save_custom_emoji_map(&ctx, custom_emojis).await?;
    }
    reload_texts(&ctx)?;

    Ok(ok(ImportLanguageResponse {
        language,
        imported_keys: updated_keys,
        warnings: Vec::new(),
    }))
}

fn reload_texts(ctx: &AppContext) -> Result<(), ApiError> {
    let new_texts = repo::load_texts_from_dir(&ctx.config.i18n_dir)
        .map_err(|err| ApiError::internal(format!("failed to reload i18n files: {err}")))?;
    ctx.update_texts(new_texts);
    Ok(())
}

fn load_i18n_emoji_map(ctx: &AppContext) -> HashMap<String, I18nEmojiPrefix> {
    let raw = ctx
        .configs
        .read()
        .ok()
        .and_then(|configs| configs.get("telegram_i18n_emojis").cloned())
        .unwrap_or_default();
    serde_json::from_str::<HashMap<String, Value>>(&raw)
        .unwrap_or_default()
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_string();
            let prefix = I18nEmojiPrefix::from_json_value(&value)?;
            if key.is_empty() {
                None
            } else {
                Some((key, prefix))
            }
        })
        .collect()
}

async fn save_i18n_emoji_map(
    ctx: &Arc<AppContext>,
    emojis: HashMap<String, Value>,
) -> Result<(), ApiError> {
    let normalized: HashMap<String, I18nEmojiPrefix> = emojis
        .into_iter()
        .filter_map(|(key, value)| {
            let key = key.trim().to_string();
            let prefix = I18nEmojiPrefix::from_json_value(&value);
            if key.is_empty() || prefix.is_none() {
                None
            } else {
                Some((key, prefix.unwrap()))
            }
        })
        .collect();
    let raw = serde_json::to_string(&normalized)
        .map_err(|err| ApiError::internal(format!("failed to serialize emojis: {err}")))?;
    configs_repo::save_configs(
        &ctx.pool,
        &HashMap::from([("telegram_i18n_emojis".to_string(), raw)]),
    )
    .await
    .map_err(|err| ApiError::internal(format!("failed to save emoji map: {err}")))?;
    let configs = configs_repo::get_all_configs(&ctx.pool)
        .await
        .map_err(|err| ApiError::internal(format!("failed to reload configs: {err}")))?;
    ctx.update_configs(configs);
    Ok(())
}

async fn save_custom_emoji_map(
    ctx: &Arc<AppContext>,
    custom_emojis: HashMap<String, Value>,
) -> Result<(), ApiError> {
    let normalized = custom_emoji_map_from_values(custom_emojis);
    let raw = serde_json::to_string(&normalized)
        .map_err(|err| ApiError::internal(format!("failed to serialize custom emojis: {err}")))?;
    configs_repo::save_configs(
        &ctx.pool,
        &HashMap::from([("telegram_custom_emojis".to_string(), raw)]),
    )
    .await
    .map_err(|err| ApiError::internal(format!("failed to save custom emoji map: {err}")))?;
    let configs = configs_repo::get_all_configs(&ctx.pool)
        .await
        .map_err(|err| ApiError::internal(format!("failed to reload configs: {err}")))?;
    ctx.update_configs(configs);
    Ok(())
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/admin/i18n/languages", get(list_languages))
        .route("/api/admin/i18n/import", post(import_language))
        .route("/api/admin/i18n/export/:code", get(export_language))
        .route(
            "/api/admin/i18n/language/:code",
            get(language_detail).put(update_language),
        )
}
