use std::{collections::HashMap, sync::Arc, time::Duration};

use axum::{
    Json,
    extract::{Multipart, Path, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sqlx::{FromRow, SqlitePool};
use teloxide::{
    payloads::{SendMessageSetters, SendPhotoSetters},
    prelude::Requester,
    types::{ChatId, InlineKeyboardButton, InlineKeyboardMarkup},
};
use tracing::{info, warn};

use crate::app::{AppContext, custom_emoji_map_from_values};
use crate::bot::{State as BotState, i18n, plugins::cmd_wallet::format_vnd};
use crate::core::responses::{Ack, ApiError, ApiResult, ok};
use crate::domains::{
    orders::api as orders_api,
    products::{models::Product, repo as products_repo},
    users::{models::Subscriber, repo as users_repo},
};

const BROADCAST_PRODUCT_PAGE_SIZE: i64 = 10;
const BROADCAST_BUTTON_NAME_MAX_CHARS: usize = 36;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BroadcastMode {
    MessageOnly,
    ViewShopButton,
    ProductList,
    NewProduct,
}

impl BroadcastMode {
    fn from_form_value(value: Option<&str>) -> Result<Self, ApiError> {
        match value.unwrap_or("message_only").trim() {
            "" | "message_only" => Ok(Self::MessageOnly),
            "view_shop" => Ok(Self::ViewShopButton),
            "product_list" => Ok(Self::ProductList),
            "new_product" => Ok(Self::NewProduct),
            _ => Err(ApiError::validation("broadcast mode is invalid")),
        }
    }
}

#[derive(Debug, Clone)]
struct BroadcastJob {
    text: String,
    image: Option<(Vec<u8>, String)>,
    emoji_prefix: Option<String>,
    custom_emojis: HashMap<String, String>,
    buttons_json: Option<String>,
    mode: BroadcastMode,
    product: Option<Product>,
}

#[derive(Debug, Clone, FromRow, Serialize)]
pub struct BroadcastTemplate {
    pub id: i64,
    pub name: String,
    pub text: String,
    pub mode: String,
    pub buttons_json: String,
    pub product_id: Option<i64>,
    pub sort_order: i64,
    pub updated_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct BroadcastTemplatePayload {
    pub name: String,
    pub text: String,
    pub mode: String,
    pub buttons_json: String,
    pub product_id: Option<i64>,
    pub sort_order: Option<i64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BroadcastTemplateSeed {
    name: &'static str,
    text: &'static str,
    mode: &'static str,
    buttons_json: &'static str,
    product_id: Option<i64>,
    sort_order: i64,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
struct BroadcastButtonSpec {
    text: String,
    callback_data: String,
}

pub async fn broadcast(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    mut multipart: Multipart,
) -> ApiResult<Ack> {
    let mut text: Option<String> = None;
    let mut image: Option<(Vec<u8>, String)> = None;
    let mut emoji_prefix = None;
    let mut custom_emojis = HashMap::new();
    let mut buttons_json: Option<String> = None;
    let mut mode: Option<String> = None;
    let mut product_id: Option<i64> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("parse multipart failed: {e}")))?
    {
        let name = field.name().unwrap_or("").to_string();
        if name == "text" {
            text = Some(field.text().await.unwrap_or_default());
        } else if name == "mode" {
            mode = Some(field.text().await.unwrap_or_default());
        } else if name == "product_id" {
            let raw = field.text().await.unwrap_or_default();
            product_id = raw.trim().parse::<i64>().ok();
        } else if name == "emoji_prefix" {
            let raw = field.text().await.unwrap_or_default();
            emoji_prefix = normalize_broadcast_emoji(&raw);
        } else if name == "custom_emojis" {
            let raw = field.text().await.unwrap_or_default();
            custom_emojis = parse_broadcast_custom_emojis(&raw);
        } else if name == "buttons_json" {
            let raw = field.text().await.unwrap_or_default();
            if !raw.trim().is_empty() {
                validate_broadcast_buttons_json(&raw)?;
                buttons_json = Some(raw);
            }
        } else if name == "image" {
            let filename = field.file_name().unwrap_or("image.jpg").to_string();
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::internal(format!("read image failed: {e}")))?;
            image = Some((data.to_vec(), filename));
        }
    }

    let Some(text) = text else {
        return Err(ApiError::validation("text is required"));
    };

    let text = text.trim();
    if text.is_empty() {
        return Err(ApiError::validation("text is required"));
    }

    let mode = BroadcastMode::from_form_value(mode.as_deref())?;
    let product = match mode {
        BroadcastMode::NewProduct => {
            let product_id =
                product_id.ok_or_else(|| ApiError::validation("product_id is required"))?;
            let product = products_repo::get_product(&ctx.pool, product_id)
                .await
                .map_err(|e| ApiError::internal(format!("load product failed: {e}")))?
                .ok_or_else(|| ApiError::not_found("product not found"))?;
            if product.is_active.unwrap_or(1) != 1 {
                return Err(ApiError::validation("product is not active"));
            }
            Some(product)
        }
        _ => None,
    };

    let subs: Vec<Subscriber> = users_repo::list_subscribers(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("load subscribers failed: {e}")))?;
    if subs.is_empty() {
        return Err(ApiError::new(
            StatusCode::BAD_REQUEST,
            "NO_SUBSCRIBER",
            "No subscribers stored yet",
        ));
    }

    spawn_broadcast_job(
        ctx,
        BroadcastJob {
            text: text.to_string(),
            image,
            emoji_prefix,
            custom_emojis,
            buttons_json,
            mode,
            product,
        },
        subs,
    );

    Ok(ok(Ack { success: true }))
}

fn spawn_broadcast_job(ctx: Arc<AppContext>, job: BroadcastJob, subscribers: Vec<Subscriber>) {
    tokio::spawn(async move {
        let total = subscribers.len();
        let mut sent = 0usize;
        for subscriber in subscribers {
            let chat_id = subscriber.chat_id;
            let result = send_broadcast_to_subscriber(ctx.clone(), &job, subscriber).await;
            if let Err(err) = result {
                warn!("broadcast failed for chat {}: {err}", chat_id);
            } else {
                sent += 1;
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        info!("broadcast finished: sent {sent}/{total}");
    });
}

fn default_broadcast_templates() -> Vec<BroadcastTemplateSeed> {
    vec![
        BroadcastTemplateSeed {
            name: "Hàng mới lên kho",
            text: "{5375135722514685501} HÀNG MỚI VỪA LÊN KHO\n━━━━━━━━━━━━\n\nSản phẩm hot vừa được nhập thêm.\nNhanh tay mua trước khi hết hàng.",
            mode: "view_shop",
            buttons_json: r#"[[{"text":"{5375135722514685501} Xem sản phẩm","callback_data":"start:shop"}]]"#,
            product_id: None,
            sort_order: 1,
        },
        BroadcastTemplateSeed {
            name: "Flash sale",
            text: "{5375135722514685501} FLASH SALE HÔM NAY\n━━━━━━━━━━━━\n\nMột số sản phẩm đang có giá tốt.\nVào shop để xem và đặt đơn ngay.",
            mode: "view_shop",
            buttons_json: r#"[[{"text":"{5375135722514685501} Xem sản phẩm","callback_data":"start:shop"}],[{"text":"{5420147074266044260} Nạp ví","callback_data":"wallet:topup"}]]"#,
            product_id: None,
            sort_order: 2,
        },
        BroadcastTemplateSeed {
            name: "Nạp ví bonus",
            text: "{5420147074266044260} NẠP VÍ NHẬN BONUS\n━━━━━━━━━━━━\n\nNạp ví trước để thanh toán nhanh hơn khi hàng mới lên kho.",
            mode: "message_only",
            buttons_json: r#"[[{"text":"{5420147074266044260} Nạp ví ngay","callback_data":"wallet:topup"},{"text":"Xem ví","callback_data":"start:wallet"}]]"#,
            product_id: None,
            sort_order: 3,
        },
        BroadcastTemplateSeed {
            name: "Sản phẩm hot còn ít",
            text: "{5375135722514685501} SẢN PHẨM HOT CÒN ÍT\n━━━━━━━━━━━━\n\nKho đang còn số lượng giới hạn.\nAi thanh toán trước sẽ được giao trước.",
            mode: "view_shop",
            buttons_json: r#"[[{"text":"{5375135722514685501} Mua ngay","callback_data":"start:shop"}]]"#,
            product_id: None,
            sort_order: 4,
        },
        BroadcastTemplateSeed {
            name: "Thông báo hỗ trợ",
            text: "THÔNG BÁO TỪ SHOP\n━━━━━━━━━━━━\n\nShop đang hỗ trợ xử lý đơn và nạp ví.\nBạn có thể xem đơn đã mua hoặc quay lại shop.",
            mode: "message_only",
            buttons_json: r#"[[{"text":"Xem đơn đã mua","callback_data":"start:orders"},{"text":"Xem sản phẩm","callback_data":"start:shop"}]]"#,
            product_id: None,
            sort_order: 5,
        },
        BroadcastTemplateSeed {
            name: "Cập nhật kho mới",
            text: "{5375135722514685501} KHO VỪA CẬP NHẬT\n━━━━━━━━━━━━\n\nShop vừa thêm hoặc bổ sung một số sản phẩm.\nBạn có thể xem danh sách hiện có và nạp ví trước khi đặt đơn.",
            mode: "product_list",
            buttons_json: r#"[[{"text":"Xem danh sách","callback_data":"start:shop"},{"text":"Nạp ví","callback_data":"wallet:topup"}]]"#,
            product_id: None,
            sort_order: 6,
        },
        BroadcastTemplateSeed {
            name: "Nhắc nạp ví",
            text: "{5420147074266044260} NHẮC NẠP VÍ\n━━━━━━━━━━━━\n\nBạn nên nạp ví sẵn để thanh toán nhanh khi sản phẩm cần mua còn hàng.",
            mode: "message_only",
            buttons_json: r#"[[{"text":"Nạp ví ngay","callback_data":"wallet:topup"},{"text":"Lịch sử nạp","callback_data":"wallet:topup_history"}]]"#,
            product_id: None,
            sort_order: 7,
        },
        BroadcastTemplateSeed {
            name: "Hướng dẫn mua hàng",
            text: "HƯỚNG DẪN MUA HÀNG\n━━━━━━━━━━━━\n\nNếu bạn cần xem cách đặt đơn hoặc quay lại danh sách sản phẩm, dùng các nút bên dưới.",
            mode: "message_only",
            buttons_json: r#"[[{"text":"Hướng dẫn","callback_data":"start:help"},{"text":"Xem shop","callback_data":"start:shop"}]]"#,
            product_id: None,
            sort_order: 8,
        },
        BroadcastTemplateSeed {
            name: "Kiểm tra đơn hàng",
            text: "KIỂM TRA ĐƠN HÀNG\n━━━━━━━━━━━━\n\nBạn có thể xem lại các đơn đã mua và số dư ví hiện tại.",
            mode: "message_only",
            buttons_json: r#"[[{"text":"Đơn đã mua","callback_data":"start:orders"},{"text":"Xem ví","callback_data":"start:wallet"}]]"#,
            product_id: None,
            sort_order: 9,
        },
        BroadcastTemplateSeed {
            name: "Kết nối API",
            text: "KẾT NỐI API SHOP\n━━━━━━━━━━━━\n\nDùng API để tích hợp mua hàng tự động hoặc tạo key API mới khi cần.",
            mode: "message_only",
            buttons_json: r#"[[{"text":"API của tôi","callback_data":"shop_api"},{"text":"Tạo API mới","callback_data":"shop_api_new"}]]"#,
            product_id: None,
            sort_order: 10,
        },
    ]
}

pub async fn list_broadcast_templates(pool: &SqlitePool) -> anyhow::Result<Vec<BroadcastTemplate>> {
    let templates = sqlx::query_as::<_, BroadcastTemplate>(
        r#"SELECT id, name, text, mode, buttons_json, product_id, sort_order, updated_at
        FROM broadcast_templates
        ORDER BY sort_order ASC, id ASC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(templates)
}

pub async fn get_broadcast_template(
    pool: &SqlitePool,
    id: i64,
) -> anyhow::Result<Option<BroadcastTemplate>> {
    let template = sqlx::query_as::<_, BroadcastTemplate>(
        r#"SELECT id, name, text, mode, buttons_json, product_id, sort_order, updated_at
        FROM broadcast_templates
        WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(template)
}

pub async fn update_broadcast_template(
    pool: &SqlitePool,
    id: i64,
    payload: &BroadcastTemplatePayload,
) -> anyhow::Result<BroadcastTemplate> {
    if let Err(err) = validate_broadcast_template_payload(payload) {
        anyhow::bail!(err.message);
    }
    let result = sqlx::query(
        r#"UPDATE broadcast_templates
        SET name = ?, text = ?, mode = ?, buttons_json = ?, product_id = ?, sort_order = ?, updated_at = datetime('now')
        WHERE id = ?"#,
    )
    .bind(payload.name.trim())
    .bind(payload.text.trim())
    .bind(payload.mode.trim())
    .bind(payload.buttons_json.trim())
    .bind(payload.product_id)
    .bind(payload.sort_order.unwrap_or(id))
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        anyhow::bail!("broadcast template not found");
    }
    get_broadcast_template(pool, id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("broadcast template not found"))
}

pub async fn enqueue_broadcast_template(
    ctx: Arc<AppContext>,
    template_id: i64,
) -> anyhow::Result<usize> {
    let template = get_broadcast_template(&ctx.pool, template_id)
        .await?
        .ok_or_else(|| anyhow::anyhow!("broadcast template not found"))?;
    let mode = BroadcastMode::from_form_value(Some(&template.mode))
        .map_err(|err| anyhow::anyhow!(err.message))?;
    let product = match mode {
        BroadcastMode::NewProduct => {
            let product_id = template
                .product_id
                .ok_or_else(|| anyhow::anyhow!("template product_id is required"))?;
            Some(
                products_repo::get_product(&ctx.pool, product_id)
                    .await?
                    .ok_or_else(|| anyhow::anyhow!("template product not found"))?,
            )
        }
        _ => None,
    };
    let subscribers = users_repo::list_subscribers(&ctx.pool).await?;
    let total = subscribers.len();
    if total == 0 {
        anyhow::bail!("No subscribers stored yet");
    }

    spawn_broadcast_job(
        ctx,
        BroadcastJob {
            text: template.text,
            image: None,
            emoji_prefix: None,
            custom_emojis: HashMap::new(),
            buttons_json: Some(template.buttons_json),
            mode,
            product,
        },
        subscribers,
    );
    Ok(total)
}

fn validate_broadcast_template_payload(payload: &BroadcastTemplatePayload) -> Result<(), ApiError> {
    if payload.name.trim().is_empty() || payload.name.chars().count() > 80 {
        return Err(ApiError::validation("template name must be 1..80 chars"));
    }
    if payload.text.trim().is_empty() || payload.text.chars().count() > 4000 {
        return Err(ApiError::validation("template text must be 1..4000 chars"));
    }
    BroadcastMode::from_form_value(Some(payload.mode.as_str()))?;
    validate_broadcast_buttons_json(&payload.buttons_json)?;
    Ok(())
}

fn validate_broadcast_buttons_json(buttons_json: &str) -> Result<(), ApiError> {
    if buttons_json.trim().is_empty() {
        return Ok(());
    }
    let rows = parse_broadcast_button_rows(buttons_json)?;
    if rows.len() > 8 {
        return Err(ApiError::validation(
            "buttons_json cannot have more than 8 rows",
        ));
    }
    for row in rows {
        if row.len() > 4 {
            return Err(ApiError::validation(
                "buttons_json cannot have more than 4 buttons per row",
            ));
        }
        for button in row {
            if button.text.trim().is_empty() || button.text.chars().count() > 64 {
                return Err(ApiError::validation("button text must be 1..64 chars"));
            }
            if !valid_template_callback_data(&button.callback_data) {
                return Err(ApiError::validation("button callback_data is invalid"));
            }
        }
    }
    Ok(())
}

fn valid_template_callback_data(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && value.len() <= 64
        && (value.starts_with("start:")
            || value.starts_with("wallet:")
            || value.starts_with("buy:")
            || value.starts_with("shop")
            || value == "shop_api")
}

fn parse_broadcast_button_rows(
    buttons_json: &str,
) -> Result<Vec<Vec<BroadcastButtonSpec>>, ApiError> {
    let trimmed = buttons_json.trim();
    if trimmed.is_empty() || trimmed == "[]" {
        return Ok(Vec::new());
    }
    serde_json::from_str::<Vec<Vec<BroadcastButtonSpec>>>(trimmed)
        .map_err(|_| ApiError::validation("buttons_json must be a JSON array of button rows"))
}

fn custom_broadcast_keyboard(
    ctx: &AppContext,
    buttons_json: &str,
) -> Option<(InlineKeyboardMarkup, Value)> {
    let rows = parse_broadcast_button_rows(buttons_json).ok()?;
    if rows.is_empty() {
        return None;
    }

    let mut typed_rows = Vec::new();
    let mut json_rows = Vec::new();
    for row in rows {
        let mut typed_row = Vec::new();
        let mut json_row = Vec::new();
        for button in row {
            let parts = i18n::button_parts_for_key(ctx, "broadcast_template_button", button.text);
            typed_row.push(InlineKeyboardButton::callback(
                parts.text.clone(),
                button.callback_data.clone(),
            ));
            let mut json_button = json!({
                "text": parts.text,
                "callback_data": button.callback_data,
            });
            if let Some(icon_id) = parts.icon_custom_emoji_id
                && let Some(obj) = json_button.as_object_mut()
            {
                obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
            }
            json_row.push(json_button);
        }
        typed_rows.push(typed_row);
        json_rows.push(json_row);
    }
    Some((
        InlineKeyboardMarkup::new(typed_rows),
        json!({ "inline_keyboard": json_rows }),
    ))
}

fn custom_broadcast_keyboard_json(ctx: &AppContext, buttons_json: &str) -> Option<Value> {
    custom_broadcast_keyboard(ctx, buttons_json).map(|(_, json)| json)
}

pub fn stock_auto_broadcast_enabled_value(value: &str) -> bool {
    matches!(
        value.trim().to_ascii_lowercase().as_str(),
        "1" | "true" | "on" | "yes" | "enabled"
    )
}

fn stock_auto_broadcast_enabled(ctx: &AppContext) -> bool {
    stock_auto_broadcast_enabled_value(&ctx.get_text("stock_auto_broadcast_enabled", "0"))
}

pub async fn notify_stock_added(
    ctx: Arc<AppContext>,
    product: Product,
    added_count: usize,
    current_stock: i64,
) {
    if added_count == 0
        || product.is_active.unwrap_or(1) != 1
        || orders_api::product_delivery_type(&product) == "manual_input"
        || !stock_auto_broadcast_enabled(&ctx)
    {
        return;
    }

    let subscribers = match users_repo::list_stock_notification_subscribers(&ctx.pool).await {
        Ok(subscribers) => subscribers,
        Err(err) => {
            warn!("failed to load subscribers for stock notification: {err}");
            return;
        }
    };
    if subscribers.is_empty() {
        return;
    }

    tokio::spawn(async move {
        let total = subscribers.len();
        let mut sent = 0usize;
        for subscriber in subscribers {
            let lang = ctx.normalize_language_code(
                subscriber
                    .preferred_language
                    .as_deref()
                    .or(subscriber.language_code.as_deref()),
            );
            let text = stock_restock_message(&ctx, &lang, &product, added_count, current_stock);
            let keyboard = stock_restock_keyboard_json(&ctx, &lang, product.id);
            match i18n::send_message_with_json_keyboard(
                &ctx,
                ChatId(subscriber.chat_id),
                "stock_restock_notification",
                text,
                keyboard,
            )
            .await
            {
                Ok(_) => sent += 1,
                Err(err) => warn!(
                    "stock notification failed for chat {} product {}: {err}",
                    subscriber.chat_id, product.id
                ),
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        info!(
            "stock notification finished for product {}: sent {sent}/{total}",
            product.id
        );
    });
}

fn stock_restock_message(
    ctx: &AppContext,
    lang: &str,
    product: &Product,
    added_count: usize,
    current_stock: i64,
) -> String {
    let unit = stock_unit_label(product);
    i18n::tr(
        ctx,
        lang,
        "stock_restock_notification",
        "📢 HÀNG MỚI VỪA LÊN KHO\n━━━━━━━━━━━━\n\n✨ {product}\n\n📦 Sản phẩm: {product}\n💵 Giá: {price}\n✅ Vừa thêm: {added} {unit}\n📊 Kho hiện tại: {stock} {unit}",
        &[
            ("product", product.name.clone()),
            ("price", format_vnd(product.price)),
            ("added", added_count.to_string()),
            ("stock", current_stock.max(0).to_string()),
            ("unit", unit.to_string()),
        ],
    )
}

fn stock_restock_keyboard_json(ctx: &AppContext, lang: &str, product_id: i64) -> Value {
    json!({
        "inline_keyboard": [
            [
                i18n::inline_button_callback_json(ctx, lang, "stock_notify_buy_now_btn", "🛒 Mua ngay", format!("buy:{product_id}")),
                i18n::inline_button_callback_json(ctx, lang, "stock_notify_view_more_btn", "📋 Xem SP khác", "start:shop"),
            ]
        ]
    })
}

fn stock_unit_label(product: &Product) -> &'static str {
    if orders_api::product_delivery_type(product) == "uploaded_file" {
        "file"
    } else {
        "tài khoản"
    }
}

async fn send_broadcast_to_subscriber(
    ctx: Arc<AppContext>,
    job: &BroadcastJob,
    subscriber: Subscriber,
) -> anyhow::Result<()> {
    let lang = ctx.normalize_language_code(
        subscriber
            .preferred_language
            .as_deref()
            .or(subscriber.language_code.as_deref()),
    );
    let keyboard = match job.mode {
        _ => job
            .buttons_json
            .as_deref()
            .and_then(|buttons_json| custom_broadcast_keyboard(&ctx, buttons_json)),
    }
    .or_else(|| match job.mode {
        BroadcastMode::MessageOnly | BroadcastMode::ProductList => None,
        BroadcastMode::ViewShopButton => Some((
            broadcast_view_shop_keyboard(&i18n::t(&ctx, &lang, "start_btn_shop", "🛒 Shop")),
            broadcast_view_shop_keyboard_json(&ctx, &lang),
        )),
        BroadcastMode::NewProduct => None,
    });

    send_broadcast_content(
        &ctx,
        ChatId(subscriber.chat_id),
        &job.text,
        job.emoji_prefix.as_deref(),
        &job.custom_emojis,
        job.image.as_ref(),
        keyboard,
    )
    .await?;

    if job.mode == BroadcastMode::ProductList {
        send_broadcast_product_list(ctx, ChatId(subscriber.chat_id), &lang).await?;
    } else if job.mode == BroadcastMode::NewProduct {
        if let Some(product) = job.product.as_ref() {
            send_broadcast_new_product_purchase_prompt(
                ctx,
                ChatId(subscriber.chat_id),
                &lang,
                product,
            )
            .await?;
        }
    }

    Ok(())
}

fn text_with_optional_emoji(text: &str, emoji: Option<&str>) -> String {
    match emoji.and_then(normalize_broadcast_emoji) {
        Some(emoji) => format!("{emoji} {text}"),
        None => text.to_string(),
    }
}

fn normalize_broadcast_emoji(raw: &str) -> Option<String> {
    let value = raw.trim();
    if value.is_empty()
        || value.chars().count() > 8
        || value.chars().any(char::is_control)
        || value.chars().all(|c| c.is_ascii_alphanumeric())
    {
        return None;
    }
    Some(value.to_string())
}

fn parse_broadcast_custom_emojis(raw: &str) -> HashMap<String, String> {
    serde_json::from_str::<HashMap<String, Value>>(raw)
        .map(custom_emoji_map_from_values)
        .unwrap_or_default()
}

fn broadcast_rich_text(
    text: &str,
    emoji_prefix: Option<&str>,
    custom_emojis: &HashMap<String, String>,
) -> i18n::RichText {
    let text = text_with_optional_emoji(text, emoji_prefix);
    let mut rich = i18n::render_custom_emoji_id_placeholders(&text, custom_emojis);
    let direct_entities = i18n::custom_emoji_entities_for_map(custom_emojis, &rich.text);
    i18n::merge_custom_emoji_entities(&mut rich.entities, direct_entities);
    rich
}

fn broadcast_rich_text_for_ctx(
    ctx: &AppContext,
    text: &str,
    emoji_prefix: Option<&str>,
    custom_emojis: &HashMap<String, String>,
) -> i18n::RichText {
    let text = text_with_optional_emoji(text, emoji_prefix);
    let placeholder_rich = i18n::render_custom_emoji_id_placeholders(&text, custom_emojis);
    let mut rich = i18n::rich_text_for_key(ctx, "broadcast_message", placeholder_rich.text);
    i18n::merge_custom_emoji_entities(&mut rich.entities, placeholder_rich.entities);
    let local_entities = i18n::custom_emoji_entities_for_map(custom_emojis, &rich.text);
    i18n::merge_custom_emoji_entities(&mut rich.entities, local_entities);
    rich
}

async fn send_broadcast_content(
    ctx: &Arc<AppContext>,
    chat_id: ChatId,
    text: &str,
    emoji_prefix: Option<&str>,
    custom_emojis: &HashMap<String, String>,
    image: Option<&(Vec<u8>, String)>,
    keyboard: Option<(InlineKeyboardMarkup, Value)>,
) -> anyhow::Result<()> {
    let rich = broadcast_rich_text_for_ctx(ctx, text, emoji_prefix, custom_emojis);
    if let Some((bytes, filename)) = image {
        let file = teloxide::types::InputFile::memory(bytes.clone()).file_name(filename.clone());
        let mut request = ctx.bot.send_photo(chat_id, file).caption(rich.text);
        if !rich.entities.is_empty() {
            request = request.caption_entities(rich.entities);
        }
        if let Some((keyboard, _)) = keyboard {
            request = request.reply_markup(keyboard);
        }
        request.await?;
    } else if let Some((_, reply_markup)) = keyboard {
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
        i18n::send_raw_telegram_method(ctx, "sendMessage", payload).await?;
    } else {
        let mut request = ctx.bot.send_message(chat_id, rich.text);
        if !rich.entities.is_empty() {
            request = request.entities(rich.entities);
        }
        request.await?;
    }
    Ok(())
}

async fn send_broadcast_product_list(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    lang: &str,
) -> anyhow::Result<()> {
    let products = products_repo::list_products(&ctx.pool, BROADCAST_PRODUCT_PAGE_SIZE, 0).await?;
    if products.is_empty() {
        let text = i18n::t(&ctx, lang, "no_products", "There are no products yet.");
        i18n::send_message_for_key(&ctx, chat_id, "no_products", text).await?;
        return Ok(());
    }

    let reply_markup = broadcast_product_list_keyboard_json(&ctx, lang, &products);
    let text = i18n::tr(
        &ctx,
        lang,
        "broadcast_product_list_title",
        "🛒 Product list:",
        &[],
    );
    i18n::send_message_with_json_keyboard(
        &ctx,
        chat_id,
        "broadcast_product_list_title",
        &text,
        reply_markup,
    )
    .await?;
    Ok(())
}

async fn send_broadcast_new_product_purchase_prompt(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    lang: &str,
    product: &Product,
) -> anyhow::Result<()> {
    let desc_text = broadcast_product_description_text(&ctx, lang, product);
    let delivery_type = orders_api::product_delivery_type(product);

    if delivery_type == "uploaded_file" {
        let stock = products_repo::count_product_items(&ctx.pool, product.id)
            .await
            .unwrap_or(0);
        if !uploaded_file_has_sellable_stock(stock) {
            let text = i18n::t(
                &ctx,
                lang,
                "uploaded_file_out_of_stock",
                "File stock is currently out. Please choose another product.",
            );
            i18n::send_message_for_key(&ctx, chat_id, "uploaded_file_out_of_stock", text).await?;
            return Ok(());
        }

        set_broadcast_dialogue_state(
            &ctx,
            chat_id,
            BotState::ChoosingQty {
                product_id: product.id,
            },
        )
        .await?;
        let text = broadcast_uploaded_file_purchase_prompt(&ctx, lang, product, stock, &desc_text);
        i18n::send_message_for_key(&ctx, chat_id, "uploaded_file_quantity_prompt", text)
            .reply_markup(broadcast_quantity_keyboard(&ctx, lang, false))
            .await?;
        return Ok(());
    }

    if delivery_type == "manual_input" {
        let plans = products_repo::list_product_plans(&ctx.pool, product.id).await?;
        if !plans.is_empty() {
            set_broadcast_dialogue_state(
                &ctx,
                chat_id,
                BotState::SelectingPlan {
                    product_id: product.id,
                },
            )
            .await?;
            let text = broadcast_manual_product_plan_prompt(&ctx, lang, product, &desc_text);
            i18n::send_message_for_key(&ctx, chat_id, "manual_product_plan_prompt", text)
                .reply_markup(broadcast_plan_keyboard(&ctx, lang, &plans))
                .await?;
            return Ok(());
        }
    }

    let stock = products_repo::count_product_items(&ctx.pool, product.id)
        .await
        .unwrap_or(0);
    set_broadcast_dialogue_state(
        &ctx,
        chat_id,
        BotState::ChoosingQty {
            product_id: product.id,
        },
    )
    .await?;
    let text = broadcast_stock_item_purchase_prompt(&ctx, lang, product, stock);
    i18n::send_message_for_key(&ctx, chat_id, "product_qty_prompt", text)
        .reply_markup(broadcast_quantity_keyboard(&ctx, lang, false))
        .await?;
    Ok(())
}

fn broadcast_view_shop_keyboard(label: &str) -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        label.to_string(),
        "start:shop",
    )]])
}

fn broadcast_view_shop_keyboard_json(ctx: &AppContext, lang: &str) -> Value {
    json!({
        "inline_keyboard": [[
            i18n::inline_button_callback_json(ctx, lang, "start_btn_shop", "🛒 Shop", "start:shop")
        ]]
    })
}

fn broadcast_product_list_keyboard_json(
    ctx: &AppContext,
    lang: &str,
    products: &[Product],
) -> Value {
    let mut rows = products
        .iter()
        .map(|product| {
            let mut button = json!({
                "text": broadcast_product_button_label(product),
                "callback_data": format!("buy:{}", product.id),
            });
            let placeholder_custom_id = product_name_placeholder_custom_emoji_id(product);
            if let Some(custom_id) = broadcast_product_button_custom_emoji_id(product)
                .or(placeholder_custom_id.as_deref())
                && let Some(obj) = button.as_object_mut()
            {
                obj.insert(
                    "icon_custom_emoji_id".to_string(),
                    Value::String(custom_id.to_string()),
                );
            }
            vec![button]
        })
        .collect::<Vec<_>>();
    rows.push(vec![json!({
        "text": i18n::t(ctx, lang, "start_btn_wallet", "💳 Wallet"),
        "callback_data": "start:wallet",
    })]);
    json!({ "inline_keyboard": rows })
}

fn broadcast_product_button_label(product: &Product) -> String {
    let (name, placeholder_custom_id) = render_button_custom_emoji_placeholders(&product.name);
    let name = truncate_button_text(&name, BROADCAST_BUTTON_NAME_MAX_CHARS);
    let label = format!("{} - {}", name, format_vnd(product.price));
    if broadcast_product_button_custom_emoji_id(product).is_some()
        || placeholder_custom_id.is_some()
    {
        return label;
    }
    match product
        .button_emoji
        .as_deref()
        .map(str::trim)
        .filter(|emoji| !emoji.is_empty())
    {
        Some(emoji) => format!("{emoji} {label}"),
        None => label,
    }
}

fn broadcast_product_button_custom_emoji_id(product: &Product) -> Option<&str> {
    product
        .button_custom_emoji_id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

fn product_name_placeholder_custom_emoji_id(product: &Product) -> Option<String> {
    let name = product.name.as_str();
    let mut byte_index = 0usize;
    while byte_index < name.len() {
        let remaining = &name[byte_index..];
        if let Some(rest) = remaining.strip_prefix('{')
            && let Some(close_index) = rest.find('}')
        {
            let candidate = &rest[..close_index];
            if (8..=64).contains(&candidate.len()) && candidate.chars().all(|c| c.is_ascii_digit())
            {
                return Some(candidate.to_string());
            }
        }
        if let Some(ch) = remaining.chars().next() {
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }
    None
}

fn broadcast_product_description_text(ctx: &AppContext, lang: &str, product: &Product) -> String {
    product
        .description
        .as_ref()
        .map(|desc| {
            i18n::tr(
                ctx,
                lang,
                "product_description_line",
                "📝 Mô tả:\n{description}\n\n",
                &[("description", desc.clone())],
            )
        })
        .unwrap_or_default()
}

fn broadcast_stock_item_purchase_prompt(
    ctx: &AppContext,
    lang: &str,
    product: &Product,
    stock: i64,
) -> String {
    let desc_text = broadcast_product_description_text(ctx, lang, product);
    let delivery_type = orders_api::product_delivery_type(product);
    i18n::tr(
        ctx,
        lang,
        "product_qty_prompt",
        "✅ Bạn chọn {product} - {price}\n📦 Còn lại: {stock}\n{description}{requires_input}\n\n⌨️ Nhập số lượng muốn mua:",
        &[
            ("product", product.name.clone()),
            ("price", format_vnd(product.price)),
            ("stock", stock.to_string()),
            ("description", desc_text),
            (
                "requires_input",
                if delivery_type == "manual_input" {
                    i18n::t(
                        ctx,
                        lang,
                        "product_requires_input_note",
                        "ℹ️ Sản phẩm này cần thông tin kích hoạt, bot sẽ hỏi ở bước tiếp theo.",
                    )
                } else {
                    "".to_string()
                },
            ),
        ],
    )
}

fn broadcast_uploaded_file_purchase_prompt(
    ctx: &AppContext,
    lang: &str,
    product: &Product,
    stock: i64,
    desc_text: &str,
) -> String {
    i18n::tr(
        ctx,
        lang,
        "uploaded_file_quantity_prompt",
        "✅ Bạn chọn {product} - {price}\n📦 Kho file còn: {stock}\n{description}📎 File sản phẩm sẽ được gửi tự động sau khi thanh toán.\n\n⌨️ Nhập số lượng file muốn mua:",
        &[
            ("product", product.name.clone()),
            ("price", format_vnd(product.price)),
            ("stock", stock.to_string()),
            ("description", desc_text.to_string()),
        ],
    )
}

fn broadcast_manual_product_plan_prompt(
    ctx: &AppContext,
    lang: &str,
    product: &Product,
    desc_text: &str,
) -> String {
    i18n::tr(
        ctx,
        lang,
        "manual_product_plan_prompt",
        "✅ Bạn chọn {product} - {price}\n{description}ℹ️ Sản phẩm cần thông tin kích hoạt.\n\n📅 Chọn gói/tháng bên dưới:",
        &[
            ("product", product.name.clone()),
            ("price", format_vnd(product.price)),
            ("description", desc_text.to_string()),
        ],
    )
}

fn broadcast_quantity_keyboard(
    ctx: &AppContext,
    lang: &str,
    require_input: bool,
) -> InlineKeyboardMarkup {
    let values = if require_input {
        vec![1, 6, 12]
    } else {
        vec![1, 2, 3, 5, 10]
    };
    let buttons = values
        .into_iter()
        .map(|v| InlineKeyboardButton::callback(v.to_string(), format!("qty:{v}")))
        .collect::<Vec<_>>();
    InlineKeyboardMarkup::new(vec![
        buttons,
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "start_btn_wallet",
            "💳 Wallet",
            "start:wallet",
        )],
        vec![i18n::inline_button_callback(
            ctx,
            lang,
            "back_btn",
            "⬅️ Back",
            "start:shop",
        )],
    ])
}

fn broadcast_plan_keyboard(
    ctx: &AppContext,
    lang: &str,
    plans: &[crate::domains::products::models::ProductPlan],
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    for chunk in plans.chunks(2) {
        let mut row = Vec::new();
        for p in chunk {
            row.push(InlineKeyboardButton::callback(
                format!("{} - {}", p.label, format_vnd(p.price)),
                format!("plan:{}", p.id),
            ));
        }
        rows.push(row);
    }
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "start_btn_wallet",
        "💳 Wallet",
        "start:wallet",
    )]);
    rows.push(vec![i18n::inline_button_callback(
        ctx,
        lang,
        "back_btn",
        "⬅️ Back",
        "start:shop",
    )]);
    InlineKeyboardMarkup::new(rows)
}

fn uploaded_file_has_sellable_stock(stock: i64) -> bool {
    stock > 0
}

async fn set_broadcast_dialogue_state(
    ctx: &AppContext,
    chat_id: ChatId,
    state: BotState,
) -> anyhow::Result<()> {
    let state_json = serde_json::to_string(&state)?;
    sqlx::query(
        "INSERT INTO dialogue_states (chat_id, state_json) VALUES (?, ?)
         ON CONFLICT(chat_id) DO UPDATE SET state_json = excluded.state_json",
    )
    .bind(chat_id.0)
    .bind(state_json)
    .execute(&ctx.pool)
    .await?;
    Ok(())
}

fn truncate_button_text(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    format!(
        "{}...",
        trimmed
            .chars()
            .take(max_chars)
            .collect::<String>()
            .trim_end()
    )
}

fn render_button_custom_emoji_placeholders(value: &str) -> (String, Option<String>) {
    let mut rendered = String::with_capacity(value.len());
    let mut first_custom_id = None;
    let mut byte_index = 0usize;
    while byte_index < value.len() {
        let remaining = &value[byte_index..];
        if let Some(rest) = remaining.strip_prefix('{')
            && let Some(close_index) = rest.find('}')
        {
            let candidate = &rest[..close_index];
            if (8..=64).contains(&candidate.len()) && candidate.chars().all(|c| c.is_ascii_digit())
            {
                rendered.push('✨');
                first_custom_id.get_or_insert_with(|| candidate.to_string());
                byte_index += close_index + 2;
                continue;
            }
        }
        if let Some(ch) = remaining.chars().next() {
            rendered.push(ch);
            byte_index += ch.len_utf8();
        } else {
            break;
        }
    }
    (rendered, first_custom_id)
}

use axum::Router;
use axum::routing::{get, post, put};

pub async fn list_broadcast_templates_handler(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
) -> ApiResult<Vec<BroadcastTemplate>> {
    let templates = list_broadcast_templates(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("list broadcast templates failed: {e}")))?;
    Ok(ok(templates))
}

pub async fn update_broadcast_template_handler(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(payload): Json<BroadcastTemplatePayload>,
) -> ApiResult<BroadcastTemplate> {
    let template = update_broadcast_template(&ctx.pool, id, &payload)
        .await
        .map_err(|e| ApiError::internal(format!("update broadcast template failed: {e}")))?;
    Ok(ok(template))
}

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route("/api/admin/broadcast", post(broadcast))
        .route(
            "/api/admin/broadcast/templates",
            get(list_broadcast_templates_handler),
        )
        .route(
            "/api/admin/broadcast/templates/:id",
            put(update_broadcast_template_handler),
        )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bot::texts::BotTexts;
    use crate::config::Config;
    use crate::domains::products::models::Product;
    use sqlx::sqlite::SqlitePoolOptions;

    #[test]
    fn broadcast_mode_from_form_value_defaults_to_message_only() {
        assert_eq!(
            BroadcastMode::from_form_value(None).unwrap(),
            BroadcastMode::MessageOnly
        );
        assert_eq!(
            BroadcastMode::from_form_value(Some("view_shop")).unwrap(),
            BroadcastMode::ViewShopButton
        );
        assert_eq!(
            BroadcastMode::from_form_value(Some("product_list")).unwrap(),
            BroadcastMode::ProductList
        );
        assert_eq!(
            BroadcastMode::from_form_value(Some("new_product")).unwrap(),
            BroadcastMode::NewProduct
        );
    }

    #[test]
    fn broadcast_mode_rejects_unknown_values() {
        assert!(BroadcastMode::from_form_value(Some("bad_mode")).is_err());
    }

    #[test]
    fn broadcast_emoji_prefix_is_normalized_and_added_to_text() {
        assert_eq!(
            text_with_optional_emoji("Noi dung", Some("  📣  ")),
            "📣 Noi dung"
        );
        assert_eq!(
            text_with_optional_emoji("Noi dung", Some("abcdefghijk")),
            "Noi dung"
        );
    }

    #[test]
    fn broadcast_rich_text_marks_multiple_dynamic_emojis_in_notification() {
        let custom_emojis = std::collections::HashMap::from([
            ("🔥".to_string(), "5368324170671202286".to_string()),
            ("🎁".to_string(), "5368324170671202287".to_string()),
        ]);

        let rich = broadcast_rich_text("Sale 🔥 hom nay 🎁", Some("📣"), &custom_emojis);

        assert_eq!(rich.text, "📣 Sale 🔥 hom nay 🎁");
        assert_eq!(rich.entities.len(), 2);
        assert_eq!(rich.entities[0].offset, "📣 Sale ".encode_utf16().count());
        assert_eq!(
            rich.entities[1].offset,
            "📣 Sale 🔥 hom nay ".encode_utf16().count()
        );
    }

    #[test]
    fn broadcast_rich_text_places_custom_emoji_id_placeholders_inline() {
        let custom_emojis = std::collections::HashMap::from([
            ("🔥".to_string(), "5368324170671202286".to_string()),
            ("🎁".to_string(), "5368324170671202287".to_string()),
        ]);

        let rich = broadcast_rich_text(
            "Dong 1 {5368324170671202286}\nDong 2 {5368324170671202287}",
            None,
            &custom_emojis,
        );

        assert_eq!(rich.text, "Dong 1 🔥\nDong 2 🎁");
        assert_eq!(rich.entities.len(), 2);
        assert_eq!(rich.entities[0].offset, "Dong 1 ".encode_utf16().count());
        assert_eq!(
            rich.entities[1].offset,
            "Dong 1 🔥\nDong 2 ".encode_utf16().count()
        );
    }

    #[test]
    fn broadcast_rich_text_places_unconfigured_custom_emoji_id_placeholders_inline() {
        let rich = broadcast_rich_text(
            "test {5375135722514685501} emoji ty nha {5420147074266044260}",
            None,
            &std::collections::HashMap::new(),
        );

        assert_eq!(rich.text, "test ✨ emoji ty nha ✨");
        assert_eq!(rich.entities.len(), 2);
        assert_eq!(rich.entities[0].offset, "test ".encode_utf16().count());
        assert_eq!(
            rich.entities[1].offset,
            "test ✨ emoji ty nha ".encode_utf16().count()
        );
    }

    #[test]
    fn view_shop_keyboard_points_to_shop_callback() {
        let keyboard = broadcast_view_shop_keyboard("Xem san pham");
        let json = serde_json::to_value(keyboard).unwrap();

        assert_eq!(json["inline_keyboard"][0][0]["text"], "Xem san pham");
        assert_eq!(json["inline_keyboard"][0][0]["callback_data"], "start:shop");
    }

    #[tokio::test]
    async fn new_product_stock_item_prompt_matches_purchase_screen() {
        let ctx = test_ctx();
        let mut product = product(42, "Plus", 2_000);
        product.description = Some("Test bot".to_string());

        let text = broadcast_stock_item_purchase_prompt(&ctx, "vi", &product, 4);

        assert!(text.contains("✅ Bạn chọn Plus - 2.000đ"));
        assert!(text.contains("📦 Còn lại: 4"));
        assert!(text.contains("📝 Mô tả:\nTest bot"));
        assert!(text.contains("⌨️ Nhập số lượng muốn mua:"));
    }

    #[tokio::test]
    async fn new_product_purchase_keyboard_uses_quantity_callbacks() {
        let ctx = test_ctx();
        let keyboard = broadcast_quantity_keyboard(&ctx, "vi", false);
        let json = serde_json::to_value(keyboard).unwrap();

        assert_eq!(json["inline_keyboard"][0][0]["callback_data"], "qty:1");
        assert_eq!(json["inline_keyboard"][0][4]["callback_data"], "qty:10");
        assert_eq!(
            json["inline_keyboard"][1][0]["callback_data"],
            "start:wallet"
        );
        assert_eq!(json["inline_keyboard"][2][0]["callback_data"], "start:shop");
    }

    #[tokio::test]
    async fn broadcast_product_list_keyboard_keeps_product_custom_emoji_icon() {
        let ctx = test_ctx();
        let mut product = product(42, "Plus", 2_000);
        product.button_emoji = Some("🛒".to_string());
        product.button_custom_emoji_id = Some("5368324170671202286".to_string());

        let json = broadcast_product_list_keyboard_json(&ctx, "vi", &[product]);

        assert_eq!(json["inline_keyboard"][0][0]["text"], "Plus - 2.000đ");
        assert_eq!(json["inline_keyboard"][0][0]["callback_data"], "buy:42");
        assert_eq!(
            json["inline_keyboard"][0][0]["icon_custom_emoji_id"],
            "5368324170671202286"
        );
    }

    #[tokio::test]
    async fn broadcast_product_list_keyboard_uses_title_placeholder_as_custom_icon() {
        let ctx = test_ctx();
        let product = product(42, "Plus {5375135722514685501}", 2_000);

        let json = broadcast_product_list_keyboard_json(&ctx, "vi", &[product]);

        assert_eq!(json["inline_keyboard"][0][0]["text"], "Plus ✨ - 2.000đ");
        assert_eq!(
            json["inline_keyboard"][0][0]["icon_custom_emoji_id"],
            "5375135722514685501"
        );
    }

    #[test]
    fn default_broadcast_templates_are_ten_reusable_slots() {
        let templates = default_broadcast_templates();

        assert_eq!(templates.len(), 10);
        assert!(
            templates
                .iter()
                .any(|template| template.name == "Hàng mới lên kho")
        );
        assert!(
            templates
                .iter()
                .any(|template| template.text.contains("{5375135722514685501}"))
        );
        assert!(
            templates
                .iter()
                .all(|template| !template.text.contains("emoji_prefix"))
        );
    }

    #[tokio::test]
    async fn custom_broadcast_keyboard_json_keeps_callback_and_custom_emoji_icon() {
        let ctx = test_ctx();
        let buttons_json =
            r#"[ [{"text":"{5375135722514685501} Xem sản phẩm","callback_data":"start:shop"}] ]"#;

        let keyboard = custom_broadcast_keyboard_json(&ctx, buttons_json).unwrap();

        assert_eq!(keyboard["inline_keyboard"][0][0]["text"], "Xem sản phẩm");
        assert_eq!(
            keyboard["inline_keyboard"][0][0]["callback_data"],
            "start:shop"
        );
        assert_eq!(
            keyboard["inline_keyboard"][0][0]["icon_custom_emoji_id"],
            "5375135722514685501"
        );
    }

    #[tokio::test]
    async fn broadcast_template_repo_lists_seeded_templates() {
        let pool = test_pool().await;

        let templates = list_broadcast_templates(&pool).await.unwrap();

        assert_eq!(templates.len(), 10);
        assert_eq!(templates[0].id, 1);
        assert_eq!(templates[9].id, 10);
        assert_eq!(templates[0].mode, "view_shop");
        assert!(templates[0].buttons_json.contains("start:shop"));
        assert!(
            templates
                .iter()
                .any(|template| template.mode == "message_only")
        );
    }

    #[tokio::test]
    async fn broadcast_template_repo_updates_template_content_and_buttons() {
        let pool = test_pool().await;
        let updated = BroadcastTemplatePayload {
            name: "Flash sale".to_string(),
            text: "Sale {5375135722514685501} hôm nay".to_string(),
            mode: "message_only".to_string(),
            buttons_json: r#"[ [{"text":"Mở shop","callback_data":"start:shop"}] ]"#.to_string(),
            product_id: None,
            sort_order: Some(2),
        };

        let template = update_broadcast_template(&pool, 2, &updated).await.unwrap();

        assert_eq!(template.name, "Flash sale");
        assert_eq!(template.mode, "message_only");
        assert!(template.text.contains("{5375135722514685501}"));
        assert!(template.buttons_json.contains("start:shop"));
    }

    #[test]
    fn stock_auto_broadcast_enabled_value_accepts_admin_toggle_values() {
        assert!(stock_auto_broadcast_enabled_value("1"));
        assert!(stock_auto_broadcast_enabled_value("on"));
        assert!(stock_auto_broadcast_enabled_value("TRUE"));
        assert!(!stock_auto_broadcast_enabled_value("0"));
        assert!(!stock_auto_broadcast_enabled_value(""));
    }

    #[tokio::test]
    async fn stock_restock_message_mentions_product_price_added_and_current_stock() {
        let ctx = test_ctx();
        let product = product(42, "CHATGPT PLUS", 160_000);

        let text = stock_restock_message(&ctx, "vi", &product, 10, 10);

        assert!(text.contains("HÀNG MỚI VỪA LÊN KHO"));
        assert!(text.contains("CHATGPT PLUS"));
        assert!(text.contains("Sản phẩm: CHATGPT PLUS"));
        assert!(text.contains("Giá: 160.000đ"));
        assert!(text.contains("Vừa thêm: 10"));
        assert!(text.contains("Kho hiện tại: 10"));
    }

    #[tokio::test]
    async fn stock_restock_keyboard_points_to_buy_and_shop_only() {
        let ctx = test_ctx();

        let json = stock_restock_keyboard_json(&ctx, "vi", 42);

        assert_eq!(json["inline_keyboard"][0][0]["callback_data"], "buy:42");
        assert_eq!(json["inline_keyboard"][0][1]["callback_data"], "start:shop");
        let callbacks = json["inline_keyboard"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|row| row.as_array().unwrap())
            .filter_map(|button| button["callback_data"].as_str())
            .collect::<Vec<_>>();
        assert!(!callbacks.contains(&"stocknotify:off"));
    }

    fn test_ctx() -> Arc<AppContext> {
        let pool = SqlitePoolOptions::new()
            .connect_lazy("sqlite::memory:")
            .unwrap();
        AppContext::new(
            teloxide::Bot::new("test-token"),
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
            std::collections::HashMap::new(),
            BotTexts::default(),
            vec![],
        )
    }

    fn product(id: i64, name: &str, price: i64) -> Product {
        Product {
            id,
            name: name.to_string(),
            price,
            is_active: Some(1),
            requires_input: Some(0),
            input_prompt: None,
            description: None,
            image_url: None,
            delivery_type: Some("stock_item".to_string()),
            file_path: None,
            file_name: None,
            file_mime: None,
            category_id: None,
            category: None,
            category_emoji: None,
            category_custom_emoji_id: None,
            button_emoji: None,
            button_custom_emoji_id: None,
            created_at: None,
            sort_order: None,
            show_sold_count: Some(0),
        }
    }

    async fn test_pool() -> sqlx::SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }
}
