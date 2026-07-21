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
    "fbunlock_btn_cancel_request",
    "fbunlock_btn_contact_quote_worker",
    "fbunlock_btn_confirm_delete_case",
    "fbunlock_btn_confirm_done",
    "fbunlock_btn_customer",
    "fbunlock_btn_customer_my_cases",
    "fbunlock_btn_daily_promo_disable",
    "fbunlock_btn_delete_case",
    "fbunlock_btn_dispute",
    "fbunlock_btn_hide_case",
    "fbunlock_btn_message_customer",
    "fbunlock_btn_message",
    "fbunlock_btn_message_worker",
    "fbunlock_btn_open_customer_chat",
    "fbunlock_btn_open_worker_chat",
    "fbunlock_btn_pay_quote",
    "fbunlock_btn_quote_case",
    "fbunlock_btn_quote_case_number",
    "fbunlock_btn_reopen_case",
    "fbunlock_btn_repost_case",
    "fbunlock_btn_reject_worker",
    "fbunlock_btn_worker",
    "fbunlock_btn_worker_apply",
    "fbunlock_btn_worker_cases",
    "fbunlock_btn_worker_paid_cases",
    "fbunlock_btn_worker_my_cases",
    "fbunlock_btn_worker_done",
    "fbunlock_btn_worker_failed",
    "orders_history_button",
    "orders_history_text",
    "start_btn_affiliate_register",
    "start_btn_help",
    "start_btn_language",
    "start_btn_orders",
    "start_btn_purchased",
    "start_btn_shop",
    "start_btn_topup",
    "start_btn_topup_history",
    "start_btn_viameta",
    "start_btn_wallet",
];

const FBUNLOCK_I18N_KEYS: &[&str] = &[
    "fbunlock_menu_text",
    "fbunlock_worker_menu_text",
    "fbunlock_daily_promo_text",
    "fbunlock_prompt_customer_username",
    "fbunlock_prompt_issue",
    "fbunlock_prompt_ownership",
    "fbunlock_btn_owned_yes",
    "fbunlock_btn_owned_no",
    "fbunlock_prompt_locked_duration",
    "fbunlock_prompt_case_note",
    "fbunlock_prompt_worker_username",
    "fbunlock_prompt_worker_services",
    "fbunlock_prompt_worker_rate",
    "fbunlock_prompt_quote_amount",
    "fbunlock_prompt_quote_note",
    "fbunlock_quote_amount_too_low",
    "fbunlock_prompt_dispute_reason",
    "fbunlock_case_created",
    "fbunlock_btn_accept_quote",
    "fbunlock_worker_registered_pending",
    "fbunlock_quote_sent_worker",
    "fbunlock_worker_cases_empty",
    "fbunlock_worker_cases_title",
    "fbunlock_worker_paid_cases_empty",
    "fbunlock_worker_paid_cases_title",
    "fbunlock_worker_my_cases_empty",
    "fbunlock_worker_my_cases_title",
    "fbunlock_customer_my_cases_empty",
    "fbunlock_customer_my_cases_title",
    "fbunlock_customer_action_delete_title",
    "fbunlock_customer_action_delete_empty",
    "fbunlock_customer_action_cancel_title",
    "fbunlock_customer_action_cancel_empty",
    "fbunlock_customer_action_message_title",
    "fbunlock_customer_action_message_empty",
    "fbunlock_customer_action_hint",
    "fbunlock_confirm_delete_case",
    "fbunlock_delete_case_done",
    "fbunlock_hide_case_done",
    "fbunlock_quote_notify_customer",
    "fbunlock_quote_accepted_customer",
    "fbunlock_case_paid_customer",
    "fbunlock_case_paid_worker",
    "fbunlock_worker_done_customer",
    "fbunlock_case_completed_customer",
    "fbunlock_cancel_unpaid_customer",
    "fbunlock_repost_case_done",
    "fbunlock_dispute_sent_customer",
];

const REQUIRED_SHOP_TEXT_KEYS: &[&str] = &[
    "manual_product_plan_prompt",
    "product_description_line",
    "product_qty_prompt",
    "shop_stock_auto",
    "shop_stock_manual",
    "uploaded_file_quantity_prompt",
];

fn required_start_button_default(key: &str) -> Option<&'static str> {
    match key {
        "fbunlock_btn_back" => Some("⬅️ Back"),
        "fbunlock_btn_admin_refund" => Some("💸 Refund customer"),
        "fbunlock_btn_admin_reject_refund" => Some("↩️ Reject refund"),
        "fbunlock_btn_approve_worker" => Some("✅ Approve worker"),
        "fbunlock_btn_cancel_case" => Some("❌ Cancel case"),
        "fbunlock_btn_cancel_request" => Some("❌ Request cancel"),
        "fbunlock_btn_contact_quote_worker" => Some("💬 Contact quote worker"),
        "fbunlock_btn_confirm_delete_case" => Some("✅ Delete"),
        "fbunlock_btn_confirm_done" => Some("✅ Confirm success"),
        "fbunlock_btn_customer" => Some("🙋 I need Facebook unlock"),
        "fbunlock_btn_customer_my_cases" => Some("🧾 My cases"),
        "fbunlock_btn_daily_promo_disable" => Some("🔕 Stop daily reminder"),
        "fbunlock_btn_delete_case" => Some("🗑 Delete case"),
        "fbunlock_btn_dispute" => Some("⚠️ Dispute"),
        "fbunlock_btn_hide_case" => Some("🧹 Hide case"),
        "fbunlock_btn_message_customer" => Some("💬 Message customer"),
        "fbunlock_btn_message" => Some("💬 Message"),
        "fbunlock_btn_message_worker" => Some("💬 Message worker"),
        "fbunlock_btn_open_customer_chat" => Some("💬 Open customer chat"),
        "fbunlock_btn_open_worker_chat" => Some("💬 Open worker chat"),
        "fbunlock_btn_pay_quote" => Some("💳 Escrow payment"),
        "fbunlock_btn_quote_case" => Some("💬 Quote case"),
        "fbunlock_btn_quote_case_number" => Some("💬 Quote #{number}"),
        "fbunlock_btn_reopen_case" => Some("🔁 Reopen case"),
        "fbunlock_btn_repost_case" => Some("🔁 Repost this case"),
        "fbunlock_btn_reject_worker" => Some("❌ Reject worker"),
        "fbunlock_btn_worker" => Some("🧑‍💻 I provide this service"),
        "fbunlock_btn_worker_apply" => Some("📝 Apply as service worker"),
        "fbunlock_btn_worker_cases" => Some("📋 View cases to quote"),
        "fbunlock_btn_worker_paid_cases" => Some("💰 Paid customer list"),
        "fbunlock_btn_worker_my_cases" => Some("🧾 My cases"),
        "fbunlock_btn_worker_done" => Some("✅ Mark completed"),
        "fbunlock_btn_worker_failed" => Some("⚠️ Cannot handle"),
        "orders_history_button" => Some("{memo} • {date} • {product}"),
        "orders_history_text" => Some("🧾 PURCHASE HISTORY\n\nTap an order to view details.\nEach button shows: memo • purchase date • product"),
        "start_btn_affiliate_register" => Some("🤝 Register affiliate"),
        "start_btn_help" => Some("Help"),
        "start_btn_language" => Some("🌐 Language"),
        "start_btn_orders" => Some("📋 Recent orders"),
        "start_btn_purchased" => Some("📦 Purchased"),
        "start_btn_shop" => Some("🛒 Shop"),
        "start_btn_topup" => Some("💰 Top up"),
        "start_btn_topup_history" => Some("📜 Top-up history"),
        "start_btn_viameta" => Some("✅ Verification"),
        "start_btn_wallet" => Some("💳 Wallet"),
        "manual_product_plan_prompt" => Some("✅ You selected {product} - {price}\n{description}ℹ️ This product requires activation information.\n\n📅 Choose a plan/month below:"),
        "product_description_line" => Some("📝 Description:\n{description}\n\n"),
        "product_qty_prompt" => Some("✅ You selected {product} - {price}\n📦 Stock left: {stock}\n{description}{requires_input}\n\n⌨️ Enter quantity to buy:"),
        "shop_stock_auto" => Some("{stock} left"),
        "shop_stock_manual" => Some("✅ available"),
        "uploaded_file_quantity_prompt" => Some("✅ You selected {product} - {price}\n📦 File stock left: {stock}\n{description}📎 Product files will be sent automatically after payment.\n\n⌨️ Enter the number of files to buy:"),
        "fbunlock_menu_text" => Some("🔓 <b>UNLOCK FACEBOOK</b>\n\nBot is the middleman between customers and service workers.\n\n• Customers create cases for free to receive quotes.\n• Multiple workers can quote the same case.\n• Customers choose a quote and pay escrow to the bot.\n• After payment, the selected worker handles the case."),
        "fbunlock_worker_menu_text" => Some("🧑‍💻 <b>SERVICE WORKER AREA</b>\n\nYou can view cases waiting for quotes and send your handling price.\n\nPlatform fee: <b>{fee_percent}%</b> on completed cases.\nExample quote 300,000đ, fee {fee_percent}%, expected receive {sample_payout}."),
        "fbunlock_daily_promo_text" => Some("🔓 Need Facebook support?\nCreate a case so multiple service workers can quote.\n\n🧑‍💻 Want to provide Facebook services?\nRegister to receive suitable cases."),
        "fbunlock_prompt_customer_username" => Some("Please enter your Telegram username:\nExample: @yourname\n\nNote: it must match the Telegram account currently using the bot."),
        "fbunlock_prompt_issue" => Some("📌 What problem is your Facebook account having?\n\nExample: checkpoint, 956 lock, safe, identity verification, lost 2FA..."),
        "fbunlock_prompt_ownership" => Some("Is this account yours?"),
        "fbunlock_btn_owned_yes" => Some("Yes"),
        "fbunlock_btn_owned_no" => Some("No"),
        "fbunlock_prompt_locked_duration" => Some("How long has your account been locked?"),
        "fbunlock_prompt_case_note" => Some("How would you like to describe your case title to service workers?\n\nExample: my acc is locked 956, but I have full documents, whoever can unlock please take this case!"),
        "fbunlock_prompt_worker_username" => Some("Please enter your Telegram username:\nExample: @yourname\n\nNote: it must match the Telegram account currently using the bot."),
        "fbunlock_prompt_worker_services" => Some("Services you can handle:\nExample: 282, 956, FAQ"),
        "fbunlock_prompt_worker_rate" => Some("Case receive rate:\nExample: 100%"),
        "fbunlock_prompt_quote_amount" => Some("💬 Enter a quote for case <code>{case_id}</code>.\n\nExamples: <code>300000</code>, <code>30k</code> = 30000, <code>1m</code> = 1000000"),
        "fbunlock_prompt_quote_note" => Some("Do you want to add extra content for the customer?\n\nCase: <code>{case_id}</code>\nPrice: <b>{amount}</b>\n\nExample: can handle quickly, or accept if documents are available.\n\nIf not, type: <code>no</code>"),
        "fbunlock_quote_amount_too_low" => Some("⚠️ Quote is too low.\nMinimum quote amount is <b>{min_amount}</b>.\n\nValid examples: <code>10k</code>, <code>30k</code>, <code>100000</code>"),
        "fbunlock_prompt_dispute_reason" => Some("⚠️ Please enter the dispute reason for case <code>{case_id}</code>.\n\nExample: Worker marked done but the account is still locked."),
        "fbunlock_case_created" => Some("✅ Facebook unlock case created.\n\nCase ID: <code>{case_id}</code>\nStatus: waiting for service workers to quote.\n\nYou have not been charged yet. When there is a quote, the bot will send it for you to choose and pay."),
        "fbunlock_btn_accept_quote" => Some("✅ Accept quote"),
        "fbunlock_worker_registered_pending" => Some("✅ Registration submitted. Please wait for admin approval."),
        "fbunlock_quote_sent_worker" => Some("✅ Quote sent for case <code>{case_id}</code>.\nPrice: <b>{amount}</b>\nThe bot will send this quote for the customer to choose."),
        "fbunlock_worker_cases_empty" => Some("There are no cases waiting for quote right now."),
        "fbunlock_worker_cases_title" => Some("🔓 <b>CASES WAITING FOR SERVICE</b>"),
        "fbunlock_worker_paid_cases_empty" => Some("No customer has paid for your cases yet."),
        "fbunlock_worker_paid_cases_title" => Some("💰 <b>PAID CUSTOMERS</b>"),
        "fbunlock_worker_my_cases_empty" => Some("You have no cases in progress."),
        "fbunlock_worker_my_cases_title" => Some("🧾 <b>MY CASES</b>"),
        "fbunlock_customer_my_cases_empty" => Some("You have no Facebook unlock cases."),
        "fbunlock_customer_my_cases_title" => Some("🧾 <b>MY CASES</b>"),
        "fbunlock_customer_action_delete_title" => Some("🗑 <b>CHOOSE CASE TO DELETE/HIDE</b>"),
        "fbunlock_customer_action_delete_empty" => Some("No case can be deleted or hidden."),
        "fbunlock_customer_action_cancel_title" => Some("❌ <b>CHOOSE CASE TO REQUEST CANCEL</b>"),
        "fbunlock_customer_action_cancel_empty" => Some("No case can request cancellation."),
        "fbunlock_customer_action_message_title" => Some("💬 <b>CHOOSE CASE TO MESSAGE</b>"),
        "fbunlock_customer_action_message_empty" => Some("No case can message service right now."),
        "fbunlock_customer_action_hint" => Some("Tap the correct case number."),
        "fbunlock_confirm_delete_case" => Some("Are you sure you want to delete case <code>{case_id}</code>?\nThe case will disappear from your list and stop being sent to service workers."),
        "fbunlock_delete_case_done" => Some("Deleted case <code>{case_id}</code> from your list."),
        "fbunlock_hide_case_done" => Some("Hidden case <code>{case_id}</code> from your list."),
        "fbunlock_quote_notify_customer" => Some("💬 Case <code>{case_id}</code> has a new quote.\n\nQuote worker: {worker_username}\nPrice: <b>{amount}</b>{note}\n\nYou can accept this quote to pay escrow through the bot.\nNote: Only pay through the bot so escrow protects the transaction."),
        "fbunlock_quote_accepted_customer" => Some("✅ You accepted quote <b>{amount}</b> for case <code>{case_id}</code>.\n\nPress payment so the bot can hold escrow, then the worker will process the case."),
        "fbunlock_case_paid_customer" => Some("✅ Paid case <code>{case_id}</code>.\nEscrow held by bot: <b>{amount}</b>\nRemaining balance: {balance}\n\nService worker in charge: {worker_username}\nYou can press message service to chat directly."),
        "fbunlock_case_paid_worker" => Some("💰 <b>CASE PAID</b>\n\nCase: <code>{case_id}</code>\nCustomer paid: <b>{amount}</b>\nPlatform fee: <b>{fee_percent}%</b> = {platform_fee}\nExpected receive when complete: <b>{worker_receive}</b>\n\nCustomer Telegram: {customer_username}\n\n<b>Problem:</b>\n{issue}\n\n<b>Case info:</b>\n<pre>{case_details}</pre>"),
        "fbunlock_worker_done_customer" => Some("✅ Service worker marked case <code>{case_id}</code> as completed.\nPlease check and confirm."),
        "fbunlock_case_completed_customer" => Some("✅ Case <code>{case_id}</code> completed. Thank you for confirming.\n\nWorker payout: <b>{worker_payout}</b>\nPlatform fee: <b>{platform_fee}</b>"),
        "fbunlock_cancel_unpaid_customer" => Some("Case cancelled. You were not charged, so no refund is needed.\n\nDo you want to repost this case for other workers to quote?"),
        "fbunlock_repost_case_done" => Some("✅ Case reposted.\n\nOld case: <code>{old_case_id}</code>\nNew case: <code>{new_case_id}</code>\nStatus: waiting for service workers to quote."),
        "fbunlock_dispute_sent_customer" => Some("Dispute with reason sent to admin."),
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
    keys.extend(FBUNLOCK_I18N_KEYS.iter().map(|key| (*key).to_string()));
    keys.extend(REQUIRED_SHOP_TEXT_KEYS.iter().map(|key| (*key).to_string()));
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
