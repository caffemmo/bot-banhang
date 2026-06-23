use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use futures::StreamExt;
use rand::{Rng, distributions::Alphanumeric};
use reqwest::header::CONTENT_TYPE;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use teloxide::payloads::{
    AnswerCallbackQuerySetters, SendDocumentSetters, SendMessageSetters, SendPhotoSetters,
};
use teloxide::prelude::*;
use teloxide::requests::Requester;
use teloxide::types::{
    BotCommand, CallbackQuery, ChatId, FileId, InlineKeyboardButton, InlineKeyboardMarkup,
    InputFile, Message, ParseMode,
};
use url::Url;
use uuid::Uuid;

use crate::app::AppContext;
use crate::bot::i18n;
use crate::bot::plugins::AppPlugin;
use crate::bot::{BotDialogue, State};
use crate::core::qr::vietqr_link;
use crate::domains::orders::models::{Order, OrderStatus};
use crate::domains::orders::repo as orders_repo;
use crate::domains::products::models::Product;
use crate::domains::wallet::repo as wallet_repo;

const BASE_URL_DEFAULT: &str = "https://viameta.co/bot";
const CATEGORY: &str = "Viameta";
const GETLINK_PRODUCT: &str = "VIAMETA - GetLink Facebook";
const UPTICK_FB_PRODUCT: &str = "VIAMETA - Up Tick Facebook";
const UPTICK_IG_PRODUCT: &str = "VIAMETA - Up Tick Instagram";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum ViametaService {
    GetlinkFb,
    UptickFb,
    UptickIg,
}

impl ViametaService {
    fn from_str(value: &str) -> Option<Self> {
        match value {
            "getlink_fb" => Some(Self::GetlinkFb),
            "uptick_fb" => Some(Self::UptickFb),
            "uptick_ig" => Some(Self::UptickIg),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Self::GetlinkFb => "getlink_fb",
            Self::UptickFb => "uptick_fb",
            Self::UptickIg => "uptick_ig",
        }
    }

    fn label(self) -> &'static str {
        match self {
            Self::GetlinkFb => "Get link Facebook",
            Self::UptickFb => "Up tích Facebook",
            Self::UptickIg => "Up tích Instagram",
        }
    }

    fn product_name(self) -> &'static str {
        match self {
            Self::GetlinkFb => GETLINK_PRODUCT,
            Self::UptickFb => UPTICK_FB_PRODUCT,
            Self::UptickIg => UPTICK_IG_PRODUCT,
        }
    }

    fn endpoint(self) -> &'static str {
        match self {
            Self::GetlinkFb => "/ajax/getlink.php",
            Self::UptickFb => "/ajax/uptick_fb.php",
            Self::UptickIg => "/ajax/uptick_ig.php",
        }
    }

    fn image_field(self) -> Option<&'static str> {
        match self {
            Self::GetlinkFb => None,
            Self::UptickFb => Some("image"),
            Self::UptickIg => Some("id_image"),
        }
    }

    fn price_key(self) -> &'static str {
        match self {
            Self::GetlinkFb => "viameta_getlink_fb_price",
            Self::UptickFb => "viameta_uptick_fb_price",
            Self::UptickIg => "viameta_uptick_ig_price",
        }
    }

    fn enabled_key(self) -> &'static str {
        match self {
            Self::GetlinkFb => "viameta_getlink_fb_enabled",
            Self::UptickFb => "viameta_uptick_fb_enabled",
            Self::UptickIg => "viameta_uptick_ig_enabled",
        }
    }

    fn description_key(self) -> &'static str {
        match self {
            Self::GetlinkFb => "viameta_getlink_fb_description",
            Self::UptickFb => "viameta_uptick_fb_description",
            Self::UptickIg => "viameta_uptick_ig_description",
        }
    }

    fn default_price(self) -> i64 {
        match self {
            Self::GetlinkFb => 15_000,
            Self::UptickFb => 20_000,
            Self::UptickIg => 40_000,
        }
    }

    fn cookie_hint(self) -> &'static str {
        match self {
            Self::GetlinkFb | Self::UptickFb => "Cookie Facebook phải có c_user.",
            Self::UptickIg => "Cookie Instagram phải có ds_user_id và sessionid.",
        }
    }

    fn default_description(self) -> &'static str {
        match self {
            Self::GetlinkFb => "Gửi cookie Facebook có c_user để hệ thống lấy link xác minh.",
            Self::UptickFb => "Gửi cookie Facebook có c_user, sau đó gửi ảnh giấy tờ JPG/PNG rõ nét dưới 5MB.",
            Self::UptickIg => "Gửi cookie Instagram có ds_user_id và sessionid, sau đó gửi ảnh giấy tờ JPG/PNG rõ nét dưới 5MB.",
        }
    }
}

#[derive(Debug, Clone, sqlx::FromRow)]
struct ViametaRequest {
    order_id: String,
    service: String,
    cookie: String,
    uid: Option<String>,
    image_path: Option<String>,
}

enum ViametaDelivery {
    Text(String),
    GetlinkFb {
        uid: String,
        link: String,
        deducted: Option<i64>,
    },
}

pub struct ViametaCommandPlugin;

#[async_trait::async_trait]
impl AppPlugin for ViametaCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdViameta"
    }

    async fn on_init(&self, pool: &crate::db::DbPool) -> Result<(), anyhow::Error> {
        ensure_viameta_schema(pool).await?;
        ensure_service_products(pool).await?;
        Ok(())
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "viameta".to_string(),
            description: "Dịch vụ tích xanh".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let lang = if let Some(user) = msg.from() {
            i18n::user_lang(&ctx, user.id.0 as i64, user.language_code.as_deref()).await
        } else {
            ctx.normalize_language_code(None)
        };
        let text = msg.text().unwrap_or("").trim();
        if text == "/viameta" || text.eq_ignore_ascii_case("✅ Dịch vụ tích xanh") {
            send_viameta_menu(&ctx, msg.chat.id, &lang).await?;
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }

        let Some(state) = dialogue.get().await? else {
            return Ok(false);
        };
        match state {
            State::ViametaCollectingCookie { service } => {
                let Some(service) = ViametaService::from_str(&service) else {
                    dialogue.update(State::Idle).await?;
                    return Ok(false);
                };
                if !service_enabled(&ctx, service) {
                    send_service_maintenance(&ctx, msg.chat.id).await?;
                    dialogue.update(State::Idle).await?;
                    return Ok(true);
                }
                if text.is_empty() {
                    prompt_cookie(&ctx, msg.chat.id, service).await?;
                    return Ok(true);
                }
                if let Some(reason) = validate_cookie(service, text) {
                    ctx.bot
                        .send_message(msg.chat.id, format!("❌ Cookie chưa đúng.\n{reason}"))
                        .await?;
                    return Ok(true);
                }
                if service.image_field().is_some() {
                    dialogue
                        .update(State::ViametaCollectingImage {
                            service: service.as_str().to_string(),
                            cookie: text.to_string(),
                        })
                        .await?;
                    ctx.bot
                        .send_message(
                            msg.chat.id,
                            "📎 Gửi ảnh giấy tờ JPG/PNG để tiếp tục.\nẢnh nên rõ, đủ thông tin và dưới 5MB.",
                        )
                        .await?;
                } else {
                    create_viameta_order_and_payment(
                        ctx.clone(),
                        msg.chat.id,
                        msg.from().map(|u| u.id.0 as i64).unwrap_or(msg.chat.id.0),
                        service,
                        text.to_string(),
                        None,
                    )
                    .await?;
                    dialogue.update(State::Idle).await?;
                }
                Ok(true)
            }
            State::ViametaCollectingImage { service, cookie } => {
                let Some(service) = ViametaService::from_str(&service) else {
                    dialogue.update(State::Idle).await?;
                    return Ok(false);
                };
                if !service_enabled(&ctx, service) {
                    send_service_maintenance(&ctx, msg.chat.id).await?;
                    dialogue.update(State::Idle).await?;
                    return Ok(true);
                }
                let Some((file_id, ext)) = viameta_image_file(&msg) else {
                    ctx.bot
                        .send_message(msg.chat.id, "❌ Vui lòng gửi ảnh giấy tờ dạng JPG hoặc PNG.")
                        .await?;
                    return Ok(true);
                };
                let image_path = download_telegram_file(&ctx, file_id, ext).await?;
                create_viameta_order_and_payment(
                    ctx.clone(),
                    msg.chat.id,
                    msg.from().map(|u| u.id.0 as i64).unwrap_or(msg.chat.id.0),
                    service,
                    cookie,
                    Some(image_path),
                )
                .await?;
                dialogue.update(State::Idle).await?;
                Ok(true)
            }
            _ => Ok(false),
        }
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
        if data == "viameta:menu" {
            let lang = i18n::user_lang(&ctx, q.from.id.0 as i64, q.from.language_code.as_deref()).await;
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                send_viameta_menu(&ctx, msg.chat().id, &lang).await?;
            }
            dialogue.update(State::Idle).await?;
            return Ok(true);
        }
        if let Some(raw) = data.strip_prefix("viameta:service:") {
            let Some(service) = ViametaService::from_str(raw) else {
                return Ok(false);
            };
            let _ = ctx.bot.answer_callback_query(q.id.clone()).await;
            if let Some(msg) = &q.message {
                if !service_enabled(&ctx, service) {
                    send_service_maintenance(&ctx, msg.chat().id).await?;
                    dialogue.update(State::Idle).await?;
                    return Ok(true);
                }
                let price = service_price(&ctx, service);
                send_viameta_message_without_preview(
                    &ctx,
                    msg.chat().id,
                    service_cookie_prompt_text(&ctx, service, price),
                )
                .await?;
                if let Some(notice) = getlink_free_retry_notice(service) {
                    send_viameta_message_without_preview(&ctx, msg.chat().id, notice).await?;
                }
                dialogue
                    .update(State::ViametaCollectingCookie {
                        service: service.as_str().to_string(),
                    })
                    .await?;
            }
            return Ok(true);
        }
        Ok(false)
    }

    async fn on_order_paid(
        &self,
        ctx: Arc<AppContext>,
        order: &Order,
        product: &Product,
    ) -> Result<Option<String>, anyhow::Error> {
        if product.category.as_deref() != Some(CATEGORY) {
            return Ok(None);
        }
        let Some(request) = load_request(&ctx.pool, &order.id).await? else {
            return Ok(Some(
                "Không tìm thấy dữ liệu yêu cầu. Admin cần kiểm tra thủ công.".to_string(),
            ));
        };

        let result = run_viameta_request(&ctx, &request).await;
        let (status, response, error, delivered) = match result {
            Ok(ViametaDelivery::Text(text)) => ("done", Some(text.clone()), None, text),
            Ok(ViametaDelivery::GetlinkFb { uid, link, deducted }) => {
                let free_retry_refund = if deducted == Some(0) {
                    refund_viameta_order(
                        &ctx,
                        order,
                        "UID này đã từng get link nên Viameta không trừ phí",
                    )
                    .await?
                } else {
                    None
                };
                let delivered =
                    send_getlink_delivery(&ctx, order, &uid, &link, deducted, free_retry_refund)
                        .await?;
                ("done", Some(link), None, delivered)
            }
            Err(err) => {
                let refund = refund_viameta_order(&ctx, order, &err.to_string()).await;
                let refund_line = match refund {
                    Ok(Some(balance_after)) => format!(
                        "✅ Đã hoàn {} về ví của bạn.\nSố dư hiện tại: {}",
                        format_vnd(order.amount),
                        format_vnd(balance_after)
                    ),
                    Ok(None) => "✅ Đơn này đã được hoàn tiền trước đó.".to_string(),
                    Err(refund_err) => {
                        tracing::error!(
                            "refund viameta order {} failed after service error: {refund_err}",
                            order.id
                        );
                        "Admin sẽ kiểm tra và hoàn tiền thủ công nếu cần.".to_string()
                    }
                };
                let text = format!(
                    "❌ Dịch vụ tích xanh chưa xử lý được\n\nĐơn: {}\nDịch vụ: {}\nLý do: {}\n\n{}",
                    order.bank_memo,
                    service_label_from_raw(&request.service),
                    friendly_error(&err.to_string()),
                    refund_line
                );
                ("error", None, Some(err.to_string()), text)
            }
        };
        if let Err(err) =
            update_request_result(&ctx.pool, &order.id, status, response.as_deref(), error.as_deref()).await
        {
            tracing::error!("update Viameta request result failed for order {}: {err}", order.id);
        }
        Ok(Some(delivered))
    }
}

async fn ensure_viameta_schema(pool: &crate::db::DbPool) -> Result<()> {
    sqlx::query(
        r#"CREATE TABLE IF NOT EXISTS viameta_requests (
            order_id TEXT PRIMARY KEY NOT NULL,
            service TEXT NOT NULL,
            cookie TEXT NOT NULL,
            uid TEXT,
            image_path TEXT,
            status TEXT NOT NULL DEFAULT 'pending',
            response TEXT,
            error TEXT,
            created_at TEXT DEFAULT CURRENT_TIMESTAMP,
            updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
            FOREIGN KEY(order_id) REFERENCES orders(id) ON DELETE CASCADE
        )"#,
    )
    .execute(pool)
    .await?;
    Ok(())
}

async fn ensure_service_products(pool: &crate::db::DbPool) -> Result<()> {
    for service in [
        ViametaService::GetlinkFb,
        ViametaService::UptickFb,
        ViametaService::UptickIg,
    ] {
        let existing = product_id_by_name(pool, service.product_name()).await?;
        if let Some(id) = existing {
            sqlx::query(
                r#"UPDATE products
                SET price = ?, is_active = 0, requires_input = 1, input_prompt = ?,
                    description = ?, delivery_type = 'manual_input', category = ?
                WHERE id = ?"#,
            )
            .bind(service.default_price())
            .bind(format!("Nhập cookie cho {}", service.label()))
            .bind("Sản phẩm ẩn dùng cho nút Dịch vụ tích xanh ngoài /shop")
            .bind(CATEGORY)
            .bind(id)
            .execute(pool)
            .await?;
        } else {
            sqlx::query(
                r#"INSERT INTO products
                (name, price, is_active, requires_input, input_prompt, description, delivery_type, category)
                VALUES (?, ?, 0, 1, ?, ?, 'manual_input', ?)"#,
            )
            .bind(service.product_name())
            .bind(service.default_price())
            .bind(format!("Nhập cookie cho {}", service.label()))
            .bind("Sản phẩm ẩn dùng cho nút Dịch vụ tích xanh ngoài /shop")
            .bind(CATEGORY)
            .execute(pool)
            .await?;
        }
    }
    Ok(())
}

async fn product_id_by_name(pool: &crate::db::DbPool, name: &str) -> Result<Option<i64>> {
    Ok(sqlx::query_scalar::<_, i64>("SELECT id FROM products WHERE name = ? LIMIT 1")
        .bind(name)
        .fetch_optional(pool)
        .await?)
}

async fn product_for_service(pool: &crate::db::DbPool, service: ViametaService) -> Result<Product> {
    sqlx::query_as::<_, Product>(
        r#"SELECT
            p.id,
            p.name,
            p.price,
            p.is_active,
            p.requires_input,
            p.input_prompt,
            p.description,
            p.image_url,
            p.delivery_type,
            p.file_path,
            p.file_name,
            p.file_mime,
            p.category_id,
            COALESCE(pc.name, p.category) AS category,
            pc.emoji AS category_emoji,
            pc.custom_emoji_id AS category_custom_emoji_id,
            p.button_emoji,
            p.button_custom_emoji_id,
            p.created_at,
            p.sort_order,
            p.show_sold_count
        FROM products p
        LEFT JOIN product_categories pc ON pc.id = p.category_id
        WHERE p.name = ?
        LIMIT 1"#,
    )
    .bind(service.product_name())
    .fetch_optional(pool)
    .await?
    .ok_or_else(|| anyhow!("missing Viameta product {}", service.product_name()))
}

fn viameta_menu_text(ctx: &AppContext) -> String {
    let title = ctx
        .get_text("viameta_menu_title", "⚡ Dịch vụ tích xanh")
        .trim()
        .to_string();
    let description = ctx
        .get_text("viameta_menu_description", "Chọn dịch vụ bạn muốn dùng:")
        .trim()
        .to_string();

    if description.is_empty() {
        title
    } else {
        format!("{title}\n\n{description}")
    }
}

async fn send_viameta_menu(ctx: &AppContext, chat_id: ChatId, _lang: &str) -> Result<()> {
    ctx.bot
        .send_message(chat_id, viameta_menu_text(ctx))
        .reply_markup(InlineKeyboardMarkup::new(vec![
            vec![InlineKeyboardButton::callback(
                "🔗 Get link Facebook",
                "viameta:service:getlink_fb",
            )],
            vec![InlineKeyboardButton::callback(
                "🔵 Up tích Facebook",
                "viameta:service:uptick_fb",
            )],
            vec![InlineKeyboardButton::callback(
                "🟣 Up tích Instagram",
                "viameta:service:uptick_ig",
            )],
            vec![InlineKeyboardButton::callback("⬅️ Quay lại", "start:menu")],
        ]))
        .await?;
    Ok(())
}

fn viameta_back_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
        "⬅️ Quay lại",
        "viameta:menu",
    )]])
}

async fn prompt_cookie(ctx: &AppContext, chat_id: ChatId, service: ViametaService) -> Result<()> {
    let mut lines = Vec::new();
    if let Some(notice) = getlink_free_retry_notice(service) {
        lines.push(notice.to_string());
    }
    lines.push(format!(
        "Vui lòng gửi cookie.\n{}",
        service_description(ctx, service)
    ));
    send_viameta_message_without_preview(
        ctx,
        chat_id,
        lines.join("\n\n"),
    )
    .await?;
    Ok(())
}

fn service_cookie_prompt_text(ctx: &AppContext, service: ViametaService, price: i64) -> String {
    let mut lines = vec![
        service.label().to_string(),
        String::new(),
        format!("Giá: {}", format_vnd(price)),
    ];
    lines.push(service_description(ctx, service));
    lines.push(String::new());
    lines.push("Vui lòng gửi cookie để tạo đơn.".to_string());
    lines.join("\n")
}

fn getlink_free_retry_notice(service: ViametaService) -> Option<&'static str> {
    matches!(service, ViametaService::GetlinkFb)
        .then_some("💡 UID đã từng get link trên Bot sẽ không tính phí khi get lại.")
}

async fn send_viameta_message_without_preview(
    ctx: &AppContext,
    chat_id: ChatId,
    text: impl Into<String>,
) -> Result<()> {
    i18n::send_raw_telegram_method(
        ctx,
        "sendMessage",
        json!({
            "chat_id": chat_id.0,
            "text": text.into(),
            "reply_markup": {
                "inline_keyboard": [
                    [{ "text": "⬅️ Quay lại", "callback_data": "viameta:menu" }]
                ]
            },
            "link_preview_options": {
                "is_disabled": true
            },
            "disable_web_page_preview": true
        }),
    )
    .await?;
    Ok(())
}

async fn send_service_maintenance(ctx: &AppContext, chat_id: ChatId) -> Result<()> {
    ctx.bot
        .send_message(chat_id, format!("🛠 {}", maintenance_message(ctx)))
        .reply_markup(viameta_back_keyboard())
        .await?;
    Ok(())
}

fn validate_cookie(service: ViametaService, cookie: &str) -> Option<&'static str> {
    let lower = cookie.to_ascii_lowercase();
    match service {
        ViametaService::GetlinkFb | ViametaService::UptickFb => {
            (!lower.contains("c_user=")).then_some("Cookie Facebook cần có c_user.")
        }
        ViametaService::UptickIg => {
            (!(lower.contains("ds_user_id=") && lower.contains("sessionid=")))
                .then_some("Cookie Instagram cần có ds_user_id và sessionid.")
        }
    }
}

fn service_price(ctx: &AppContext, service: ViametaService) -> i64 {
    ctx.get_text(service.price_key(), &service.default_price().to_string())
        .trim()
        .parse::<i64>()
        .unwrap_or_else(|_| service.default_price())
}

fn service_description(ctx: &AppContext, service: ViametaService) -> String {
    ctx.get_text(service.description_key(), service.default_description())
        .trim()
        .to_string()
}

fn service_enabled(ctx: &AppContext, service: ViametaService) -> bool {
    config_bool(ctx, service.enabled_key(), true)
}

fn maintenance_message(ctx: &AppContext) -> String {
    ctx.get_text(
        "viameta_maintenance_message",
        "Dịch vụ này đang bảo trì, vui lòng quay lại sau.",
    )
    .trim()
    .to_string()
}

fn config_bool(ctx: &AppContext, key: &str, default: bool) -> bool {
    let default_value = if default { "1" } else { "0" };
    let value = ctx.get_text(key, default_value);
    let normalized = value.trim().to_ascii_lowercase();
    match normalized.as_str() {
        "1" | "true" | "on" | "yes" | "enabled" | "enable" | "bat" | "bật" => true,
        "0" | "false" | "off" | "no" | "disabled" | "disable" | "tat" | "tắt" => false,
        _ => default,
    }
}

fn viameta_api_key(ctx: &AppContext) -> Option<String> {
    let configured = ctx.get_text("viameta_api_key", "");
    if !configured.trim().is_empty() {
        return Some(configured.trim().to_string());
    }
    std::env::var("VIAMETA_API_KEY")
        .ok()
        .map(|v| v.trim().to_string())
        .filter(|v| !v.is_empty())
}

fn viameta_base_url(ctx: &AppContext) -> String {
    ctx.get_text("viameta_base_url", BASE_URL_DEFAULT)
        .trim()
        .trim_end_matches('/')
        .to_string()
}

fn viameta_image_file(msg: &Message) -> Option<(FileId, &'static str)> {
    if let Some(photo) = msg.photo().and_then(|photos| photos.last()) {
        return Some((photo.file.id.clone(), "jpg"));
    }
    let doc = msg.document()?;
    let file_name = doc.file_name.as_deref().unwrap_or("");
    let ext = extension_from_name(file_name)?;
    Some((doc.file.id.clone(), ext))
}

fn extension_from_name(file_name: &str) -> Option<&'static str> {
    let lower = file_name.to_ascii_lowercase();
    if lower.ends_with(".png") {
        Some("png")
    } else if lower.ends_with(".jpg") || lower.ends_with(".jpeg") {
        Some("jpg")
    } else {
        None
    }
}

async fn download_telegram_file(
    ctx: &AppContext,
    file_id: FileId,
    extension: &str,
) -> Result<String> {
    let file = ctx.bot.get_file(file_id).await?;
    let url = format!(
        "https://api.telegram.org/file/bot{}/{}",
        ctx.config.telegram_token, file.path
    );
    let bytes = reqwest::Client::new()
        .get(url)
        .send()
        .await?
        .error_for_status()?
        .bytes()
        .await?;
    if bytes.len() > 5 * 1024 * 1024 {
        return Err(anyhow!("Ảnh giấy tờ vượt quá 5MB"));
    }
    let dir = Path::new("storage").join("viameta");
    tokio::fs::create_dir_all(&dir).await?;
    let path = dir.join(format!("{}.{}", Uuid::new_v4(), extension));
    tokio::fs::write(&path, bytes).await?;
    Ok(path.to_string_lossy().to_string())
}

async fn create_viameta_order_and_payment(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    service: ViametaService,
    cookie: String,
    image_path: Option<String>,
) -> Result<()> {
    ensure_service_products(&ctx.pool).await?;
    let mut product = product_for_service(&ctx.pool, service).await?;
    product.price = service_price(&ctx, service);
    let amount = product.price;
    let memo = generate_memo(&ctx).await?;
    let mut order = Order::new(
        user_id,
        chat_id.0,
        product.id,
        1,
        amount,
        memo.clone(),
        Some(format!("viameta:{}", service.as_str())),
        None,
        None,
        None,
        None,
    );
    order.delivered_data = Some(format!(
        "Đã nhận yêu cầu {}. Hệ thống sẽ xử lý sau khi thanh toán.",
        service.label()
    ));

    let mut tx = ctx.pool.begin().await?;
    orders_repo::insert_order_tx(&mut tx, &order).await?;
    sqlx::query(
        r#"INSERT INTO viameta_requests
        (order_id, service, cookie, uid, image_path, status, updated_at)
        VALUES (?, ?, ?, ?, ?, 'pending', datetime('now'))"#,
    )
    .bind(&order.id)
    .bind(service.as_str())
    .bind(cookie)
    .bind(Option::<String>::None)
    .bind(image_path)
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;

    send_checkout(ctx, chat_id, user_id, &order, &product).await
}

async fn send_checkout(
    ctx: Arc<AppContext>,
    chat_id: ChatId,
    user_id: i64,
    order: &Order,
    product: &Product,
) -> Result<()> {
    let amount = order.amount;
    let confirm_text = format!(
        "🧾 XÁC NHẬN ĐƠN\n\nDịch vụ: {}\nMã đơn: {}\nTổng tiền: {}\n\nVui lòng thanh toán để hệ thống bắt đầu xử lý.",
        display_product_name(&product.name),
        order.bank_memo,
        format_vnd(amount)
    );
    ctx.bot.send_message(chat_id, confirm_text).await?;

    let bank_line = if let Some(name) = &ctx.bank_account_name() {
        format!("{} - {}", html_escape(&ctx.bank_name()), html_escape(name))
    } else {
        html_escape(&ctx.bank_name())
    };
    let pay_text = format!(
        "💰 Amount: {}\n🏦 Bank: {}\n📱 Account: <code>{}</code>\n📝 Transfer memo: <code>{}</code>\n\n⚠️ Chuyển đúng nội dung để bot tự xử lý.",
        format_vnd(amount),
        bank_line,
        html_escape(&ctx.bank_account()),
        html_escape(&order.bank_memo)
    );
    let qr_url: Url =
        vietqr_link(&ctx.bank_name(), &ctx.bank_account(), amount, &order.bank_memo).parse()?;
    let wallet_balance = wallet_repo::get_or_create_wallet(&ctx.pool, user_id)
        .await
        .map(|w| w.balance)
        .unwrap_or(0);
    ctx.bot
        .send_photo(chat_id, InputFile::url(qr_url))
        .caption(pay_text)
        .parse_mode(ParseMode::Html)
        .reply_markup(viameta_checkout_keyboard(wallet_balance, amount, &order.id))
        .await?;
    Ok(())
}

fn viameta_checkout_keyboard(
    wallet_balance: i64,
    amount: i64,
    order_id: &str,
) -> InlineKeyboardMarkup {
    let mut rows = Vec::new();
    if wallet_balance >= amount {
        rows.push(vec![InlineKeyboardButton::callback(
            format!("💳 Thanh toán ví ({})", format_vnd(wallet_balance)),
            format!("paywallet:{order_id}"),
        )]);
    }
    rows.push(vec![InlineKeyboardButton::callback(
        "⬅️ Dịch vụ tích xanh",
        "viameta:menu",
    )]);
    rows.push(vec![InlineKeyboardButton::callback("💳 Ví tiền", "start:wallet")]);
    InlineKeyboardMarkup::new(rows)
}

async fn load_request(
    pool: &crate::db::DbPool,
    order_id: &str,
) -> Result<Option<ViametaRequest>> {
    Ok(sqlx::query_as::<_, ViametaRequest>(
        r#"SELECT order_id, service, cookie, uid, image_path
        FROM viameta_requests
        WHERE order_id = ?"#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?)
}

async fn update_request_result(
    pool: &crate::db::DbPool,
    order_id: &str,
    status: &str,
    response: Option<&str>,
    error: Option<&str>,
) -> Result<()> {
    sqlx::query(
        r#"UPDATE viameta_requests
        SET status = ?, response = ?, error = ?, updated_at = datetime('now')
        WHERE order_id = ?"#,
    )
    .bind(status)
    .bind(response)
    .bind(error)
    .bind(order_id)
    .execute(pool)
    .await?;
    Ok(())
}

async fn run_viameta_request(ctx: &AppContext, request: &ViametaRequest) -> Result<ViametaDelivery> {
    let service = ViametaService::from_str(&request.service)
        .ok_or_else(|| anyhow!("unknown Viameta service {}", request.service))?;
    if !service_enabled(ctx, service) {
        return Err(anyhow!(maintenance_message(ctx)));
    }
    match service {
        ViametaService::GetlinkFb => run_getlink(ctx, request).await,
        ViametaService::UptickFb | ViametaService::UptickIg => run_uptick(ctx, request, service).await,
    }
}

async fn run_getlink(ctx: &AppContext, request: &ViametaRequest) -> Result<ViametaDelivery> {
    let api_key = viameta_api_key(ctx).ok_or_else(|| anyhow!("missing viameta_api_key"))?;
    let url = format!("{}{}", viameta_base_url(ctx), ViametaService::GetlinkFb.endpoint());
    let mut payload = json!({
        "cookie": request.cookie,
        "confirm": true,
    });
    if let Some(uid) = request.uid.as_deref().filter(|v| !v.trim().is_empty()) {
        payload["uid"] = Value::String(uid.to_string());
    }
    let value: Value = reqwest::Client::new()
        .post(url)
        .header("X-Api-Key", api_key)
        .json(&payload)
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let success = value
        .get("success")
        .and_then(Value::as_bool)
        .or_else(|| {
            value
                .get("status")
                .and_then(Value::as_str)
                .map(|status| status.eq_ignore_ascii_case("success"))
        })
        .unwrap_or(false);
    if !success {
        return Err(anyhow!(api_error_message(&value)));
    }

    let uid = json_string(&value, "uid").unwrap_or("-").to_string();
    let link = json_string(&value, "link")
        .filter(|v| !v.trim().is_empty() && *v != "-")
        .ok_or_else(|| anyhow!("Không nhận được link kết quả"))?
        .to_string();
    let deducted = value
        .get("balance_deducted")
        .or_else(|| value.get("data").and_then(|data| data.get("balance_deducted")))
        .and_then(Value::as_i64);

    Ok(ViametaDelivery::GetlinkFb { uid, link, deducted })
}

async fn run_uptick(
    ctx: &AppContext,
    request: &ViametaRequest,
    service: ViametaService,
) -> Result<ViametaDelivery> {
    let api_key = viameta_api_key(ctx).ok_or_else(|| anyhow!("missing viameta_api_key"))?;
    let image_path = request
        .image_path
        .as_deref()
        .filter(|v| !v.trim().is_empty())
        .ok_or_else(|| anyhow!("missing image_path"))?;
    let image_bytes = tokio::fs::read(image_path).await?;
    let file_name = Path::new(image_path)
        .file_name()
        .and_then(|v| v.to_str())
        .unwrap_or("document.jpg")
        .to_string();
    let field = service.image_field().ok_or_else(|| anyhow!("service has no image field"))?;
    let form = reqwest::multipart::Form::new()
        .text("cookie", request.cookie.clone())
        .text("confirm", "true")
        .part(field.to_string(), reqwest::multipart::Part::bytes(image_bytes).file_name(file_name));
    let url = format!("{}{}", viameta_base_url(ctx), service.endpoint());
    let response = reqwest::Client::new()
        .post(url)
        .header("X-Api-Key", api_key)
        .multipart(form)
        .send()
        .await?
        .error_for_status()?;
    parse_uptick_response(response, service).await.map(ViametaDelivery::Text)
}

async fn parse_uptick_response(response: reqwest::Response, service: ViametaService) -> Result<String> {
    let is_sse = response
        .headers()
        .get(CONTENT_TYPE)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.to_ascii_lowercase().contains("text/event-stream"))
        .unwrap_or(false);
    if is_sse {
        return parse_sse_response(response, service).await;
    }

    let text = response.text().await?;
    parse_uptick_text_response(&text, service)
}

fn parse_uptick_text_response(text: &str, service: ViametaService) -> Result<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return Err(anyhow!("Dịch vụ trả về phản hồi trống"));
    }

    if trimmed.lines().any(|line| line.trim_start().starts_with("data:")) {
        return parse_sse_text_response(trimmed, service);
    }

    if let Ok(value) = serde_json::from_str::<Value>(trimmed) {
        return parse_json_uptick_response(&value, service);
    }

    Ok(format_uptick_delivery_message(service, trimmed.to_string()))
}

async fn parse_sse_response(response: reqwest::Response, service: ViametaService) -> Result<String> {
    let mut stream = response.bytes_stream();
    let mut buffer = String::new();
    let mut last_logs = Vec::new();
    while let Some(chunk) = stream.next().await {
        let chunk = chunk?;
        buffer.push_str(&String::from_utf8_lossy(&chunk));
        while let Some(pos) = buffer.find('\n') {
            let line = buffer[..pos].trim().to_string();
            buffer = buffer[pos + 1..].to_string();
            if !line.starts_with("data:") {
                continue;
            }
            let raw = line.trim_start_matches("data:").trim();
            if raw.is_empty() || raw == "[DONE]" {
                continue;
            }
            let event: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!({}));
            if let Some(result) = parse_json_event_result(&event, service, &mut last_logs)? {
                return Ok(result);
            }
        }
    }
    if !buffer.trim().is_empty() {
        return parse_uptick_text_response(&buffer, service);
    }
    Err(anyhow!(
        "Dịch vụ kết thúc nhưng chưa có kết quả. Log cuối: {}",
        last_logs.join(" | ")
    ))
}

fn parse_sse_text_response(text: &str, service: ViametaService) -> Result<String> {
    let mut last_logs = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.starts_with("data:") {
            continue;
        }
        let raw = line.trim_start_matches("data:").trim();
        if raw.is_empty() || raw == "[DONE]" {
            continue;
        }
        let event: Value = serde_json::from_str(raw).unwrap_or_else(|_| json!({}));
        if let Some(result) = parse_json_event_result(&event, service, &mut last_logs)? {
            return Ok(result);
        }
    }
    Err(anyhow!(
        "Dịch vụ kết thúc nhưng chưa có kết quả. Log cuối: {}",
        last_logs.join(" | ")
    ))
}

fn parse_json_uptick_response(value: &Value, service: ViametaService) -> Result<String> {
    let success = value
        .get("success")
        .and_then(Value::as_bool)
        .or_else(|| {
            value.get("ok").and_then(Value::as_bool)
        })
        .or_else(|| {
            value
                .get("status")
                .and_then(Value::as_str)
                .map(|status| {
                    matches!(
                        status.to_ascii_lowercase().as_str(),
                        "success" | "done" | "ok" | "completed"
                    )
                })
        });
    if success == Some(false) || json_has_error_status(value) || value.get("error").is_some() {
        return Err(anyhow!(api_error_message(value)));
    }

    let message = json_message(value).unwrap_or_else(|| "Meta đã nhận yêu cầu của bạn.".to_string());
    Ok(format_uptick_delivery_message(service, message))
}

fn parse_json_event_result(
    event: &Value,
    service: ViametaService,
    last_logs: &mut Vec<String>,
) -> Result<Option<String>> {
    let event_type = event
        .get("type")
        .or_else(|| event.get("status"))
        .and_then(Value::as_str)
        .unwrap_or("");
    let message = event_message(event).unwrap_or_default();
    if event_type == "log" && !message.is_empty() {
        last_logs.push(message);
        if last_logs.len() > 5 {
            last_logs.remove(0);
        }
        return Ok(None);
    }
    if event_type == "done" {
        let final_message = if message.is_empty() {
            "Meta đã nhận yêu cầu của bạn.".to_string()
        } else {
            message
        };
        return Ok(Some(format_uptick_delivery_message(service, final_message)));
    }
    if event_type == "error" {
        let final_message = if message.is_empty() {
            "Dịch vụ trả lỗi không rõ nội dung".to_string()
        } else {
            message
        };
        return Err(anyhow!(final_message));
    }
    Ok(None)
}

fn format_uptick_delivery_message(service: ViametaService, message: String) -> String {
    format!(
        "✅ {} đã gửi yêu cầu\n\n{}\n\n⏳ Vui lòng chờ Viameta/Meta xử lý. Trạng thái tích xanh có thể chưa cập nhật ngay trong app.",
        service.label(),
        message
    )
}

fn json_has_error_status(value: &Value) -> bool {
    value
        .get("status")
        .and_then(Value::as_str)
        .map(|status| {
            matches!(
                status.to_ascii_lowercase().as_str(),
                "error" | "failed" | "fail" | "false"
            )
        })
        .unwrap_or(false)
}

fn json_message(value: &Value) -> Option<String> {
    value
        .get("message")
        .or_else(|| value.get("msg"))
        .or_else(|| value.get("result"))
        .or_else(|| value.get("response"))
        .and_then(Value::as_str)
        .or_else(|| value.get("data").and_then(Value::as_str))
        .or_else(|| {
            value
                .get("data")
                .and_then(|data| data.get("message").or_else(|| data.get("msg")))
                .and_then(Value::as_str)
        })
        .map(ToString::to_string)
}

fn event_message(event: &Value) -> Option<String> {
    event
        .get("payload")
        .and_then(|payload| payload.get("message").or_else(|| payload.get("msg")))
        .and_then(Value::as_str)
        .or_else(|| event.get("message").and_then(Value::as_str))
        .or_else(|| event.get("msg").and_then(Value::as_str))
        .map(ToString::to_string)
}

async fn send_getlink_delivery(
    ctx: &AppContext,
    order: &Order,
    uid: &str,
    link: &str,
    deducted: Option<i64>,
    free_retry_refund: Option<i64>,
) -> Result<String> {
    let chat_id = ChatId(order.chat_id);
    let fee_note = getlink_fee_note(deducted, free_retry_refund, order.amount);
    let text = format!(
        "✅ Get link Facebook thành công\n\n{fee_note}\n\nKết quả đã được gửi trong file đính kèm."
    );
    let mut sent = false;
    if let Ok(url) = Url::parse(link) {
        if ctx
            .bot
            .send_message(chat_id, text.clone())
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::url(
                "🔗 Mở link Facebook",
                url,
            )]]))
            .await
            .is_ok()
        {
            sent = true;
        }
    }
    if !sent {
        ctx.bot.send_message(chat_id, text).await?;
    }

    let file_content = format!(
        "GET LINK FACEBOOK THÀNH CÔNG\n\nMã đơn: {}\nUID: {}\n{}\n\nLink:\n{}\n",
        order.bank_memo, uid, fee_note, link
    );
    ctx.bot
        .send_document(
            chat_id,
            InputFile::memory(file_content.into_bytes())
                .file_name(format!("getlink_{}.txt", order.bank_memo)),
        )
        .caption("📄 File link Facebook của bạn")
        .await?;

    Ok(format!(
        "✅ Get link Facebook thành công\n\nUID: {}\n{}\n\nKết quả đã gửi bằng file getlink_{}.txt",
        uid, fee_note, order.bank_memo
    ))
}

fn getlink_fee_note(deducted: Option<i64>, free_retry_refund: Option<i64>, order_amount: i64) -> String {
    match (deducted, free_retry_refund) {
        (Some(0), Some(balance_after)) => format!(
            "ℹ️ UID này đã từng get link trên Viameta nên lần get lại không tính phí.\n✅ Bot đã hoàn {} vào ví của bạn.\n💳 Số dư ví hiện tại: {}",
            format_vnd(order_amount),
            format_vnd(balance_after)
        ),
        (Some(0), None) => {
            "ℹ️ UID này đã từng get link trên Viameta nên lần get lại không tính phí. Đơn này đã được hoàn tiền trước đó.".to_string()
        }
        (Some(amount), _) => format!("Phí xử lý Viameta: {}", format_vnd(amount)),
        (None, _) => "Phí xử lý Viameta: chưa có thông tin từ API.".to_string(),
    }
}

async fn refund_viameta_order(
    ctx: &AppContext,
    order: &Order,
    reason: &str,
) -> Result<Option<i64>> {
    let already_refunded = sqlx::query_scalar::<_, i64>(
        "SELECT COUNT(1) FROM wallet_transactions WHERE order_id = ? AND type = 'refund'",
    )
    .bind(&order.id)
    .fetch_one(&ctx.pool)
    .await
    .unwrap_or(0);
    if already_refunded > 0 {
        return Ok(None);
    }

    let mut tx = ctx.pool.begin().await?;
    let note = format!("Hoàn tiền dịch vụ tích xanh: {}", friendly_error(reason));
    let balance_after = wallet_repo::credit_wallet(
        &mut tx,
        order.user_id,
        order.amount,
        "refund",
        Some(&order.id),
        None,
        Some(&note),
    )
    .await?;
    tx.commit().await?;
    Ok(Some(balance_after))
}

fn json_string<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value
        .get(key)
        .and_then(Value::as_str)
        .or_else(|| value.get("data").and_then(|data| data.get(key)).and_then(Value::as_str))
}

fn api_error_message(value: &Value) -> String {
    value
        .get("message")
        .or_else(|| value.get("error"))
        .or_else(|| value.get("data").and_then(|data| data.get("message")))
        .or_else(|| value.get("data").and_then(|data| data.get("error")))
        .and_then(Value::as_str)
        .unwrap_or("Dịch vụ trả lỗi không rõ nội dung")
        .to_string()
}

fn friendly_error(value: &str) -> String {
    value
        .replace("Viameta API", "Dịch vụ")
        .replace("Viameta", "Dịch vụ")
        .replace("viameta", "dịch vụ")
        .replace("API", "dịch vụ")
}

fn service_label_from_raw(raw: &str) -> &'static str {
    ViametaService::from_str(raw)
        .map(ViametaService::label)
        .unwrap_or("Dịch vụ tích xanh")
}

fn display_product_name(name: &str) -> String {
    name.strip_prefix("VIAMETA - ")
        .unwrap_or(name)
        .to_string()
}

async fn generate_memo(ctx: &AppContext) -> Result<String> {
    let prefix = ctx.order_memo_prefix();
    let random_len = ctx.order_memo_length();
    for _ in 0..5 {
        let suffix: String = rand::thread_rng()
            .sample_iter(&Alphanumeric)
            .filter(|c| c.is_ascii_alphanumeric())
            .map(char::from)
            .take(random_len)
            .collect::<String>()
            .to_uppercase();
        let memo = format!("{prefix}{suffix}");
        let exists =
            sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM orders WHERE bank_memo = ?")
                .bind(&memo)
                .fetch_one(&ctx.pool)
                .await
                .unwrap_or(0);
        if exists == 0 {
            return Ok(memo);
        }
    }
    Err(anyhow!("Không tạo được memo unique"))
}

fn format_vnd(amount: i64) -> String {
    let s = amount.abs().to_string();
    let mut with_sep = String::new();
    for (i, ch) in s.chars().rev().enumerate() {
        if i > 0 && i % 3 == 0 {
            with_sep.push('.');
        }
        with_sep.push(ch);
    }
    let formatted: String = with_sep.chars().rev().collect();
    if amount < 0 {
        format!("-{}đ", formatted)
    } else {
        format!("{}đ", formatted)
    }
}

fn html_escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn getlink_notice_mentions_free_retry() {
        let notice = getlink_free_retry_notice(ViametaService::GetlinkFb).unwrap();

        assert!(notice.contains("UID đã từng get link trên Bot"));
        assert!(notice.contains("không tính phí khi get lại"));
    }

    #[test]
    fn non_getlink_services_do_not_show_free_retry_notice() {
        assert!(getlink_free_retry_notice(ViametaService::UptickFb).is_none());
        assert!(getlink_free_retry_notice(ViametaService::UptickIg).is_none());
    }

    #[test]
    fn uptick_parser_accepts_json_success() {
        let text = r#"{"success":true,"message":"queued"}"#;
        let parsed = parse_uptick_text_response(text, ViametaService::UptickFb).unwrap();

        assert!(parsed.contains("queued"));
        assert!(parsed.contains("đã gửi yêu cầu"));
    }

    #[test]
    fn uptick_parser_accepts_sse_text() {
        let text = r#"data: {"type":"log","payload":{"message":"uploading"}}
data: {"type":"done","payload":{"message":"done"}}"#;
        let parsed = parse_uptick_text_response(text, ViametaService::UptickFb).unwrap();

        assert!(parsed.contains("done"));
        assert!(parsed.contains("chưa cập nhật ngay"));
    }

    #[test]
    fn uptick_parser_rejects_json_error() {
        let text = r#"{"success":false,"message":"bad cookie"}"#;
        let parsed = parse_uptick_text_response(text, ViametaService::UptickFb);

        assert!(parsed.is_err());
    }
}
