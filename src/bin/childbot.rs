use std::env;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendMessageSetters};
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatId, InlineKeyboardButton, InlineKeyboardMarkup, Message};
use tracing_subscriber::EnvFilter;

#[derive(Clone)]
struct ChildBotConfig {
    api_base_url: String,
    api_key: String,
    shop_name: String,
}

#[derive(Clone)]
struct ChildBotContext {
    http: Client,
    config: ChildBotConfig,
}

#[derive(Debug, Deserialize)]
struct ApiSuccess<T> {
    ok: bool,
    data: T,
}

#[derive(Debug, Deserialize)]
struct ApiErrorResponse {
    error: ApiErrorBody,
}

#[derive(Debug, Deserialize)]
struct ApiErrorBody {
    message: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductItem {
    id: i64,
    name: String,
    price: i64,
    category: Option<String>,
    description: Option<String>,
    delivery_type: String,
    stock_count: i64,
    plans: Vec<ProductPlan>,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductPlan {
    id: i64,
    label: String,
    months: i64,
    price: i64,
}

#[derive(Debug, Serialize)]
struct PurchaseRequestPayload {
    telegram_id: i64,
    chat_id: Option<i64>,
    product_id: i64,
    qty: Option<i64>,
    plan_id: Option<i64>,
    customer_input: Option<String>,
}

#[derive(Debug, Deserialize)]
struct PurchaseRequestResponse {
    request_id: i64,
    amount_display: String,
    confirmation_sent: bool,
    order_id: Option<String>,
    balance_after_display: Option<String>,
    delivered_data: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let token = env::var("CHILDBOT_TELOXIDE_TOKEN")
        .or_else(|_| env::var("TELOXIDE_TOKEN"))
        .map_err(|_| anyhow!("CHILDBOT_TELOXIDE_TOKEN is required"))?;
    let config = ChildBotConfig {
        api_base_url: env::var("CHILDBOT_API_BASE_URL")
            .or_else(|_| env::var("API_BASE_URL"))
            .unwrap_or_else(|_| "https://caffemmo.com".to_string())
            .trim_end_matches('/')
            .to_string(),
        api_key: env::var("CHILDBOT_API_KEY").map_err(|_| anyhow!("CHILDBOT_API_KEY is required"))?,
        shop_name: env::var("CHILDBOT_SHOP_NAME").unwrap_or_else(|_| "Shop CTV".to_string()),
    };
    let ctx = Arc::new(ChildBotContext {
        http: Client::new(),
        config,
    });
    let bot = Bot::new(token);
    let me = bot.get_me().await?;
    tracing::info!("Child bot started as @{}", me.user.username.unwrap_or_default());

    Dispatcher::builder(
        bot,
        dptree::entry()
            .branch(Update::filter_message().endpoint(handle_message))
            .branch(Update::filter_callback_query().endpoint(handle_callback)),
    )
    .dependencies(dptree::deps![ctx])
    .enable_ctrlc_handler()
    .build()
    .dispatch()
    .await;

    Ok(())
}

async fn handle_message(bot: Bot, msg: Message, ctx: Arc<ChildBotContext>) -> Result<()> {
    let text = msg.text().unwrap_or("").trim();
    if text.starts_with("/start") || text.starts_with("/shop") {
        send_home(&bot, &msg, &ctx).await?;
    }
    Ok(())
}

async fn handle_callback(bot: Bot, q: CallbackQuery, ctx: Arc<ChildBotContext>) -> Result<()> {
    let Some(data) = q.data.clone() else {
        return Ok(());
    };
    if data == "products" {
        let _ = bot.answer_callback_query(q.id.clone()).await;
        if let Some(msg) = &q.message {
            send_products(&bot, msg.chat().id, &ctx).await?;
        }
        return Ok(());
    }
    if let Some(product_id) = data.strip_prefix("buy:").and_then(|raw| raw.parse::<i64>().ok()) {
        create_purchase_request(&bot, &q, &ctx, product_id, None).await?;
        return Ok(());
    }
    if let Some(rest) = data.strip_prefix("buyplan:") {
        let mut parts = rest.split(':');
        let product_id = parts.next().and_then(|raw| raw.parse::<i64>().ok());
        let plan_id = parts.next().and_then(|raw| raw.parse::<i64>().ok());
        if let (Some(product_id), Some(plan_id)) = (product_id, plan_id) {
            create_purchase_request(&bot, &q, &ctx, product_id, Some(plan_id)).await?;
        }
        return Ok(());
    }
    Ok(())
}

async fn send_home(bot: &Bot, msg: &Message, ctx: &ChildBotContext) -> Result<()> {
    let text = format!(
        "{}\n\nChọn sản phẩm bên dưới. Mua hàng thành công sẽ nhận dữ liệu ngay tại bot này.",
        ctx.config.shop_name,
    );
    bot.send_message(msg.chat.id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![InlineKeyboardButton::callback(
            "🛒 Xem sản phẩm",
            "products",
        )]]))
        .await?;
    Ok(())
}

async fn send_products(bot: &Bot, chat_id: ChatId, ctx: &ChildBotContext) -> Result<()> {
    let products = api_get::<Vec<ProductItem>>(ctx, "/api/childbot/products").await?;
    if products.is_empty() {
        bot.send_message(chat_id, "Hiện chưa có sản phẩm.").await?;
        return Ok(());
    }

    let mut rows = Vec::new();
    let mut lines = vec!["🛒 Danh sách sản phẩm".to_string(), String::new()];
    for product in products.iter().take(30) {
        let category = product.category.clone().unwrap_or_else(|| "Khác".to_string());
        let stock_note = if product.delivery_type == "manual_input" {
            "dịch vụ".to_string()
        } else {
            format!("còn {}", product.stock_count)
        };
        lines.push(format!(
            "#{} | {} | {} | {} | {}",
            product.id,
            product.name,
            format_vnd(product.price),
            category,
            stock_note,
        ));

        if product.delivery_type == "manual_input" && !product.plans.is_empty() {
            for plan in product.plans.iter().take(3) {
                rows.push(vec![InlineKeyboardButton::callback(
                    format!("{} - {}", product.name, format_vnd(plan.price)),
                    format!("buyplan:{}:{}", product.id, plan.id),
                )]);
            }
        } else if product.stock_count > 0 {
            rows.push(vec![InlineKeyboardButton::callback(
                format!("Mua {}", product.name),
                format!("buy:{}", product.id),
            )]);
        }
    }
    if rows.is_empty() {
        rows.push(vec![InlineKeyboardButton::callback("Tải lại", "products")]);
    }
    bot.send_message(chat_id, lines.join("\n"))
        .reply_markup(InlineKeyboardMarkup::new(rows))
        .await?;
    Ok(())
}

async fn create_purchase_request(
    bot: &Bot,
    q: &CallbackQuery,
    ctx: &ChildBotContext,
    product_id: i64,
    plan_id: Option<i64>,
) -> Result<()> {
    let telegram_id = q.from.id.0 as i64;
    let chat_id = q.message.as_ref().map(|msg| msg.chat().id.0);
    let payload = PurchaseRequestPayload {
        telegram_id,
        chat_id,
        product_id,
        qty: Some(1),
        plan_id,
        customer_input: None,
    };

    match api_post::<PurchaseRequestPayload, PurchaseRequestResponse>(
        ctx,
        "/api/childbot/purchase-requests",
        &payload,
    )
    .await
    {
        Ok(response) => {
            let _ = bot
                .answer_callback_query(q.id.clone())
                .text("Mua hàng thành công")
                .await;
            if let Some(msg) = &q.message {
                let order_id = response.order_id.unwrap_or_else(|| "-".to_string());
                let balance_after = response
                    .balance_after_display
                    .unwrap_or_else(|| "-".to_string());
                let delivered_data = response
                    .delivered_data
                    .unwrap_or_else(|| "Không có dữ liệu giao hàng.".to_string());
                bot.send_message(
                    msg.chat().id,
                    format!(
                        "✅ Mua hàng thành công\n\nĐơn: {}\nSố tiền: {}\nSố dư CTV còn lại: {}\n\nDữ liệu giao hàng:\n{}",
                        order_id,
                        response.amount_display,
                        balance_after,
                        delivered_data,
                    ),
                )
                .await?;
            }
        }
        Err(err) => {
            let _ = bot
                .answer_callback_query(q.id.clone())
                .text("Không mua được sản phẩm")
                .await;
            if let Some(msg) = &q.message {
                bot.send_message(msg.chat().id, format!("❌ {err}")).await?;
            }
        }
    }
    Ok(())
}

async fn api_get<T>(ctx: &ChildBotContext, path: &str) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let url = format!("{}{}", ctx.config.api_base_url, path);
    let response = ctx
        .http
        .get(url)
        .bearer_auth(&ctx.config.api_key)
        .send()
        .await?;
    parse_api_response(response).await
}

async fn api_post<P, T>(ctx: &ChildBotContext, path: &str, payload: &P) -> Result<T>
where
    P: Serialize,
    T: for<'de> Deserialize<'de>,
{
    let url = format!("{}{}", ctx.config.api_base_url, path);
    let response = ctx
        .http
        .post(url)
        .bearer_auth(&ctx.config.api_key)
        .json(payload)
        .send()
        .await?;
    parse_api_response(response).await
}

async fn parse_api_response<T>(response: reqwest::Response) -> Result<T>
where
    T: for<'de> Deserialize<'de>,
{
    let status = response.status();
    let bytes = response.bytes().await?;
    if status.is_success() {
        let body: ApiSuccess<T> = serde_json::from_slice(&bytes)?;
        return Ok(body.data);
    }
    let message = serde_json::from_slice::<ApiErrorResponse>(&bytes)
        .map(|body| body.error.message)
        .unwrap_or_else(|_| String::from_utf8_lossy(&bytes).to_string());
    Err(anyhow!(message))
}

fn format_vnd(amount: i64) -> String {
    let raw = amount.abs().to_string();
    let mut grouped = String::with_capacity(raw.len() + raw.len() / 3);
    for (index, ch) in raw.chars().rev().enumerate() {
        if index > 0 && index % 3 == 0 {
            grouped.push('.');
        }
        grouped.push(ch);
    }
    let mut value = grouped.chars().rev().collect::<String>();
    if amount < 0 {
        value.insert(0, '-');
    }
    format!("{value}đ")
}

fn init_tracing() {
    let env_filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();
}
