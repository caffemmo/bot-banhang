use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::path::Path;
use std::sync::Arc;

use anyhow::{Result, anyhow};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use teloxide::payloads::{AnswerCallbackQuerySetters, SendAnimationSetters, SendMessageSetters};
use teloxide::prelude::*;
use teloxide::types::{CallbackQuery, ChatId, FileId, InlineKeyboardButton, InlineKeyboardMarkup, InputFile, Message};
use tracing_subscriber::EnvFilter;
use url::Url;

const PRODUCT_BUTTON_NAME_MAX_CHARS: usize = 46;
const CATEGORY_PRODUCT_LIMIT: usize = 16;
const ALL_PRODUCTS_TEXT_LIMIT: usize = 80;

#[derive(Clone)]
struct ChildBotConfig {
    telegram_token: String,
    api_base_url: String,
    api_key: String,
    shop_name: String,
    settings_path: String,
    welcome_animation: Option<String>,
}

#[derive(Clone)]
struct ChildBotContext {
    http: Client,
    config: ChildBotConfig,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
struct ChildBotSettings {
    shop_name: Option<String>,
    intro: Option<String>,
    bank: Option<String>,
    contact: Option<String>,
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

#[derive(Debug, Deserialize)]
struct ChildBotInfoResponse {
    child_bot_id: i64,
    owner_user_id: i64,
    bot_username: Option<String>,
    shop_name: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ChildBotBalanceResponse {
    owner_user_id: i64,
    balance: i64,
    balance_display: String,
}

#[derive(Debug, Deserialize)]
struct ChildBotOrderItem {
    order_id: String,
    buyer_user_id: i64,
    product_name: String,
    amount: i64,
    amount_display: String,
    created_at: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ProductItem {
    id: i64,
    name: String,
    price: i64,
    category: Option<String>,
    category_emoji: Option<String>,
    category_custom_emoji_id: Option<String>,
    button_emoji: Option<String>,
    button_custom_emoji_id: Option<String>,
    description: Option<String>,
    image_url: Option<String>,
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

#[derive(Debug, Clone)]
struct CategorySummary {
    name: String,
    count: usize,
    emoji: Option<String>,
    custom_emoji_id: Option<String>,
}

#[derive(Debug, Clone)]
struct ButtonSpec {
    text: String,
    callback_data: String,
    icon_custom_emoji_id: Option<String>,
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenvy::dotenv().ok();
    init_tracing();

    let token = env::var("CHILDBOT_TELOXIDE_TOKEN")
        .or_else(|_| env::var("TELOXIDE_TOKEN"))
        .map_err(|_| anyhow!("CHILDBOT_TELOXIDE_TOKEN is required"))?;
    let config = ChildBotConfig {
        telegram_token: token.clone(),
        api_base_url: env::var("CHILDBOT_API_BASE_URL")
            .or_else(|_| env::var("API_BASE_URL"))
            .unwrap_or_else(|_| "https://caffemmo.com".to_string())
            .trim_end_matches('/')
            .to_string(),
        api_key: env::var("CHILDBOT_API_KEY").map_err(|_| anyhow!("CHILDBOT_API_KEY is required"))?,
        shop_name: env::var("CHILDBOT_SHOP_NAME").unwrap_or_else(|_| "Shop CTV".to_string()),
        settings_path: env::var("CHILDBOT_SETTINGS_PATH")
            .unwrap_or_else(|_| "./childbot_settings.json".to_string()),
        welcome_animation: env::var("CHILDBOT_WELCOME_ANIMATION")
            .ok()
            .map(|value| value.trim().to_string())
            .filter(|value| !value.is_empty()),
    };
    let ctx = Arc::new(ChildBotContext {
        http: Client::new(),
        config,
    });
    let bot = Bot::new(token);
    let me = bot.get_me().await?;
    register_command_menu(&ctx).await?;
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
    let command = text.split_whitespace().next().unwrap_or("").split('@').next().unwrap_or("");
    match command {
        "/start" | "/menu" => send_home_to_chat(&bot, msg.chat.id, &ctx).await?,
        "/shop" => send_categories(&bot, msg.chat.id, &ctx).await?,
        "/admin" => send_admin_menu(&bot, msg.chat.id, &msg, &ctx).await?,
        "/settings" => send_settings(&bot, msg.chat.id, &msg, &ctx).await?,
        "/mybalance" => send_my_balance(&bot, msg.chat.id, &msg, &ctx).await?,
        "/myorders" => send_my_orders(&bot, msg.chat.id, &msg, &ctx).await?,
        "/setname" => set_setting_value(&bot, &msg, &ctx, "shop_name", text).await?,
        "/setintro" => set_setting_value(&bot, &msg, &ctx, "intro", text).await?,
        "/setbank" => set_setting_value(&bot, &msg, &ctx, "bank", text).await?,
        "/setcontact" => set_setting_value(&bot, &msg, &ctx, "contact", text).await?,
        _ => {}
    }
    Ok(())
}

async fn handle_callback(bot: Bot, q: CallbackQuery, ctx: Arc<ChildBotContext>) -> Result<()> {
    let Some(data) = q.data.clone() else {
        return Ok(());
    };
    if data == "home" {
        let _ = bot.answer_callback_query(q.id.clone()).await;
        if let Some(msg) = &q.message {
            send_home_to_chat(&bot, msg.chat().id, &ctx).await?;
        }
        return Ok(());
    }
    if data == "products" {
        let _ = bot.answer_callback_query(q.id.clone()).await;
        if let Some(msg) = &q.message {
            send_categories(&bot, msg.chat().id, &ctx).await?;
        }
        return Ok(());
    }
    if data == "admin" || data == "admin:settings" || data == "admin:balance" || data == "admin:orders" {
        let _ = bot.answer_callback_query(q.id.clone()).await;
        let Some(msg) = &q.message else { return Ok(()); };
        if !is_owner_user(&ctx, q.from.id.0 as i64).await? {
            bot.send_message(msg.chat().id, "❌ Lệnh này chỉ dành cho chủ bot con.").await?;
            return Ok(());
        }
        match data.as_str() {
            "admin:balance" => send_balance_text(&bot, msg.chat().id, &ctx).await?,
            "admin:orders" => send_orders_text(&bot, msg.chat().id, &ctx).await?,
            _ => send_admin_menu_for_owner(&bot, msg.chat().id).await?,
        }
        return Ok(());
    }
    if let Some(category_index) = data.strip_prefix("cat:") {
        let _ = bot.answer_callback_query(q.id.clone()).await;
        if let Some(msg) = &q.message {
            send_category_products(&bot, msg.chat().id, &ctx, category_index).await?;
        }
        return Ok(());
    }
    if let Some(product_id) = data.strip_prefix("prod:").and_then(|raw| raw.parse::<i64>().ok()) {
        let _ = bot.answer_callback_query(q.id.clone()).await;
        if let Some(msg) = &q.message {
            send_product_detail(&bot, msg.chat().id, &ctx, product_id).await?;
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

async fn send_home_to_chat(bot: &Bot, chat_id: ChatId, ctx: &ChildBotContext) -> Result<()> {
    let settings = load_settings(ctx);
    let shop_name = effective_shop_name(ctx, &settings);
    let intro = settings.intro.as_deref().map(str::trim).filter(|value| !value.is_empty());
    let text = if let Some(intro) = intro {
        format!("🏪 {shop_name}\n\n{intro}")
    } else {
        format!(
            "🏪 {shop_name}\n\n⚡ Bot bán hàng tự động 24/7\n🛒 Chọn sản phẩm, thanh toán bằng số dư CTV\n📦 Mua thành công nhận hàng ngay tại bot này"
        )
    };
    let keyboard = home_keyboard();
    if let Some(animation) = &ctx.config.welcome_animation {
        match input_file_from_value(animation) {
            Ok(input_file) => {
                match bot
                    .send_animation(chat_id, input_file)
                    .caption(text.clone())
                    .reply_markup(keyboard.clone())
                    .await
                {
                    Ok(_) => return Ok(()),
                    Err(err) => tracing::warn!("failed to send child bot welcome animation: {err}"),
                }
            }
            Err(err) => tracing::warn!("invalid child bot welcome animation: {err}"),
        }
    }

    bot.send_message(chat_id, text)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

fn home_keyboard() -> InlineKeyboardMarkup {
    InlineKeyboardMarkup::new(vec![
        vec![InlineKeyboardButton::callback("🛒 Xem sản phẩm", "products")],
        vec![InlineKeyboardButton::callback("🔥 Hàng nổi bật", "cat:all")],
        vec![InlineKeyboardButton::callback("⚙️ Quản trị", "admin")],
        vec![InlineKeyboardButton::callback("🏠 Menu chính", "home")],
    ])
}

async fn send_admin_menu(bot: &Bot, chat_id: ChatId, msg: &Message, ctx: &ChildBotContext) -> Result<()> {
    if !ensure_owner_message(bot, chat_id, msg, ctx).await? {
        return Ok(());
    }
    send_admin_menu_for_owner(bot, chat_id).await
}

async fn send_admin_menu_for_owner(bot: &Bot, chat_id: ChatId) -> Result<()> {
    let text = "⚙️ Admin bot con\n\n/setname Tên shop\n/setintro Nội dung giới thiệu\n/setbank Thông tin ngân hàng\n/setcontact Thông tin hỗ trợ\n/mybalance Xem số dư CTV\n/myorders Xem đơn bot con\n/settings Xem cấu hình";
    let keyboard = InlineKeyboardMarkup::new(vec![
        vec![
            InlineKeyboardButton::callback("💰 Số dư", "admin:balance"),
            InlineKeyboardButton::callback("🧾 Đơn hàng", "admin:orders"),
        ],
        vec![InlineKeyboardButton::callback("⚙️ Cấu hình", "admin:settings")],
        vec![InlineKeyboardButton::callback("🏠 Menu chính", "home")],
    ]);
    bot.send_message(chat_id, text).reply_markup(keyboard).await?;
    Ok(())
}

async fn send_settings(bot: &Bot, chat_id: ChatId, msg: &Message, ctx: &ChildBotContext) -> Result<()> {
    if !ensure_owner_message(bot, chat_id, msg, ctx).await? {
        return Ok(());
    }
    let settings = load_settings(ctx);
    bot.send_message(chat_id, settings_text(ctx, &settings)).await?;
    Ok(())
}

async fn send_my_balance(bot: &Bot, chat_id: ChatId, msg: &Message, ctx: &ChildBotContext) -> Result<()> {
    if !ensure_owner_message(bot, chat_id, msg, ctx).await? {
        return Ok(());
    }
    send_balance_text(bot, chat_id, ctx).await
}

async fn send_balance_text(bot: &Bot, chat_id: ChatId, ctx: &ChildBotContext) -> Result<()> {
    let balance = api_get::<ChildBotBalanceResponse>(ctx, "/api/childbot/balance").await?;
    bot.send_message(
        chat_id,
        format!("💰 Số dư CTV\n\nID: {}\nSố dư: {}", balance.owner_user_id, balance.balance_display),
    )
    .await?;
    Ok(())
}

async fn send_my_orders(bot: &Bot, chat_id: ChatId, msg: &Message, ctx: &ChildBotContext) -> Result<()> {
    if !ensure_owner_message(bot, chat_id, msg, ctx).await? {
        return Ok(());
    }
    send_orders_text(bot, chat_id, ctx).await
}

async fn send_orders_text(bot: &Bot, chat_id: ChatId, ctx: &ChildBotContext) -> Result<()> {
    let orders = api_get::<Vec<ChildBotOrderItem>>(ctx, "/api/childbot/orders").await?;
    if orders.is_empty() {
        bot.send_message(chat_id, "🧾 Chưa có đơn nào từ bot con.").await?;
        return Ok(());
    }
    let mut lines = vec!["🧾 Đơn bot con gần đây".to_string(), String::new()];
    for order in orders.iter().take(10) {
        lines.push(format!(
            "• {} — {} — khách {} — {}",
            order.product_name, order.amount_display, order.buyer_user_id, order.created_at
        ));
    }
    bot.send_message(chat_id, lines.join("\n")).await?;
    Ok(())
}

async fn set_setting_value(bot: &Bot, msg: &Message, ctx: &ChildBotContext, key: &str, text: &str) -> Result<()> {
    let chat_id = msg.chat.id;
    if !ensure_owner_message(bot, chat_id, msg, ctx).await? {
        return Ok(());
    }
    let value = text.split_once(' ').map(|(_, value)| value.trim()).unwrap_or("");
    if value.is_empty() {
        let usage = match key {
            "shop_name" => "/setname Tên shop",
            "intro" => "/setintro Nội dung giới thiệu",
            "bank" => "/setbank Ngân hàng - STK - Chủ tài khoản",
            "contact" => "/setcontact @username hoặc số điện thoại",
            _ => "/settings",
        };
        bot.send_message(chat_id, format!("Vui lòng nhập theo mẫu:\n{usage}")).await?;
        return Ok(());
    }
    let mut settings = load_settings(ctx);
    match key {
        "shop_name" => settings.shop_name = Some(value.to_string()),
        "intro" => settings.intro = Some(value.to_string()),
        "bank" => settings.bank = Some(value.to_string()),
        "contact" => settings.contact = Some(value.to_string()),
        _ => {}
    }
    save_settings(ctx, &settings)?;
    bot.send_message(chat_id, format!("✅ Đã lưu cấu hình\n\n{}", settings_text(ctx, &settings))).await?;
    Ok(())
}

async fn ensure_owner_message(bot: &Bot, chat_id: ChatId, msg: &Message, ctx: &ChildBotContext) -> Result<bool> {
    let user_id = msg.from.as_ref().map(|user| user.id.0 as i64).unwrap_or(0);
    if is_owner_user(ctx, user_id).await? {
        return Ok(true);
    }
    bot.send_message(chat_id, "❌ Lệnh này chỉ dành cho chủ bot con.").await?;
    Ok(false)
}

async fn is_owner_user(ctx: &ChildBotContext, user_id: i64) -> Result<bool> {
    if user_id <= 0 {
        return Ok(false);
    }
    let info = api_get::<ChildBotInfoResponse>(ctx, "/api/childbot/me").await?;
    Ok(info.owner_user_id == user_id)
}

fn settings_text(ctx: &ChildBotContext, settings: &ChildBotSettings) -> String {
    format!(
        "⚙️ Cấu hình bot con\n\nTên shop: {}\nGiới thiệu: {}\nNgân hàng: {}\nLiên hệ: {}",
        effective_shop_name(ctx, settings),
        settings.intro.as_deref().unwrap_or("Chưa cài"),
        settings.bank.as_deref().unwrap_or("Chưa cài"),
        settings.contact.as_deref().unwrap_or("Chưa cài"),
    )
}

fn effective_shop_name(ctx: &ChildBotContext, settings: &ChildBotSettings) -> String {
    settings
        .shop_name
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or(&ctx.config.shop_name)
        .to_string()
}

fn load_settings(ctx: &ChildBotContext) -> ChildBotSettings {
    let path = Path::new(&ctx.config.settings_path);
    let Ok(content) = fs::read_to_string(path) else {
        return ChildBotSettings::default();
    };
    serde_json::from_str(&content).unwrap_or_default()
}

fn save_settings(ctx: &ChildBotContext, settings: &ChildBotSettings) -> Result<()> {
    let path = Path::new(&ctx.config.settings_path);
    if let Some(parent) = path.parent().filter(|parent| !parent.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, serde_json::to_string_pretty(settings)?)?;
    Ok(())
}

async fn register_command_menu(ctx: &ChildBotContext) -> Result<()> {
    let commands = json!([
        {"command":"start","description":"Bắt đầu"},
        {"command":"shop","description":"Xem sản phẩm"},
        {"command":"admin","description":"Admin bot con"},
        {"command":"settings","description":"Xem cấu hình"},
        {"command":"setname","description":"Đổi tên shop"},
        {"command":"setintro","description":"Đổi giới thiệu"},
        {"command":"setbank","description":"Cài thông tin ngân hàng"},
        {"command":"setcontact","description":"Cài thông tin hỗ trợ"},
        {"command":"mybalance","description":"Xem số dư CTV"},
        {"command":"myorders","description":"Xem đơn bot con"}
    ]);
    send_raw_telegram_method(ctx, "setMyCommands", json!({ "commands": commands })).await
}

fn input_file_from_value(value: &str) -> Result<InputFile> {
    if value.starts_with("http://") || value.starts_with("https://") {
        Ok(InputFile::url(Url::parse(value)?))
    } else {
        Ok(InputFile::file_id(FileId(value.to_string())))
    }
}

async fn send_categories(bot: &Bot, chat_id: ChatId, ctx: &ChildBotContext) -> Result<()> {
    let products = api_get::<Vec<ProductItem>>(ctx, "/api/childbot/products").await?;
    if products.is_empty() {
        bot.send_message(chat_id, "Hiện chưa có sản phẩm.").await?;
        return Ok(());
    }

    let categories = category_counts(&products);
    let mut rows = Vec::new();
    rows.push(vec![button_spec(
        format!("📦 Tất cả sản phẩm ({})", products.len()),
        "cat:all",
        None,
    )]);

    let mut row = Vec::new();
    for (index, category) in categories.iter().enumerate() {
        row.push(category_button(category, format!("cat:{index}")));
        if row.len() == 2 {
            rows.push(row);
            row = Vec::new();
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows.push(vec![button_spec("🏠 Menu chính", "home", None)]);

    send_message_json_keyboard(
        ctx,
        chat_id,
        "🛒 Danh mục sản phẩm\n\nChọn danh mục bạn muốn xem:",
        rows,
    )
    .await?;
    Ok(())
}

async fn send_category_products(
    _bot: &Bot,
    chat_id: ChatId,
    ctx: &ChildBotContext,
    category_index: &str,
) -> Result<()> {
    let products = api_get::<Vec<ProductItem>>(ctx, "/api/childbot/products").await?;
    let categories = category_counts(&products);
    let selected_category = if category_index == "all" {
        None
    } else {
        let index = category_index.parse::<usize>().ok();
        index.and_then(|i| categories.get(i).map(|category| category.name.clone()))
    };

    let filtered = products
        .iter()
        .filter(|product| {
            selected_category
                .as_ref()
                .map(|category| product_category(product) == *category)
                .unwrap_or(true)
        })
        .collect::<Vec<_>>();

    if filtered.is_empty() {
        send_message_json_keyboard(
            ctx,
            chat_id,
            "Danh mục này chưa có sản phẩm.",
            vec![vec![button_spec("⬅️ Quay lại danh mục", "products", None)]],
        )
        .await?;
        return Ok(());
    }

    if selected_category.is_none() {
        let rows = category_keyboard_rows(&categories);
        let text = all_products_text(&products, &categories);
        send_message_json_keyboard(ctx, chat_id, text, rows).await?;
        return Ok(());
    }

    let title = selected_category.unwrap_or_else(|| "Tất cả sản phẩm".to_string());
    let mut lines = vec![format!("{} {title}", category_icon(&title)), String::new()];
    let mut rows = Vec::new();
    for product in filtered.iter().take(CATEGORY_PRODUCT_LIMIT) {
        let stock_note = product_stock_note(product);
        lines.push(format!(
            "• {} — {} ({})",
            product.name,
            format_vnd(product.price),
            stock_note,
        ));
        rows.push(vec![product_button(product, format!("prod:{}", product.id))]);
    }
    if filtered.len() > CATEGORY_PRODUCT_LIMIT {
        lines.push(format!(
            "\nĐang hiển thị {CATEGORY_PRODUCT_LIMIT}/{} sản phẩm đầu tiên.",
            filtered.len(),
        ));
    }
    rows.push(vec![
        button_spec("⬅️ Danh mục", "products", None),
        button_spec("🏠 Menu", "home", None),
    ]);

    send_message_json_keyboard(ctx, chat_id, lines.join("\n"), rows).await?;
    Ok(())
}

async fn send_product_detail(
    bot: &Bot,
    chat_id: ChatId,
    ctx: &ChildBotContext,
    product_id: i64,
) -> Result<()> {
    let products = api_get::<Vec<ProductItem>>(ctx, "/api/childbot/products").await?;
    let Some(product) = products.into_iter().find(|item| item.id == product_id) else {
        bot.send_message(chat_id, "Sản phẩm không tồn tại hoặc đã ngừng bán.")
            .await?;
        return Ok(());
    };

    let category = product_category(&product);
    let stock_note = if product.delivery_type == "manual_input" {
        "🧾 Dịch vụ xử lý theo yêu cầu".to_string()
    } else {
        format!("📦 Kho còn: {}", product.stock_count)
    };
    let description = product
        .description
        .clone()
        .filter(|value| !value.trim().is_empty())
        .unwrap_or_else(|| "Không có mô tả.".to_string());
    let text = format!(
        "{} {}\n\n💵 Giá: {}\n{}\n🏷️ Danh mục: {}\n\n{}",
        product_static_icon(&product),
        product.name,
        format_vnd(product.price),
        stock_note,
        category,
        description,
    );

    let mut rows = Vec::new();
    if product.delivery_type == "manual_input" && !product.plans.is_empty() {
        for plan in product.plans.iter().take(8) {
            rows.push(vec![button_spec(
                format!("✅ {} - {}", truncate_label(&plan.label, 24), format_vnd(plan.price)),
                format!("buyplan:{}:{}", product.id, plan.id),
                None,
            )]);
        }
    } else if product.stock_count > 0 {
        rows.push(vec![button_spec(
            "Mua ngay",
            format!("buy:{}", product.id),
            product_icon_custom_id(&product),
        )]);
    } else {
        rows.push(vec![button_spec("⛔ Hết hàng", "products", None)]);
    }
    rows.push(vec![
        button_spec("⬅️ Danh mục", "products", None),
        button_spec("🏠 Menu", "home", None),
    ]);

    send_message_json_keyboard(ctx, chat_id, text, rows).await?;
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
                        "✅ Mua hàng thành công\n\n🧾 Đơn: {}\n💵 Số tiền: {}\n👛 Số dư CTV còn lại: {}\n\n📦 Dữ liệu giao hàng:\n{}",
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

fn all_products_text(products: &[ProductItem], categories: &[CategorySummary]) -> String {
    let mut lines = vec!["✨ Tất cả sản phẩm".to_string(), String::new()];
    let mut shown = 0usize;
    for category in categories {
        if shown >= ALL_PRODUCTS_TEXT_LIMIT {
            break;
        }
        lines.push(format!("{} {}", category_text_icon(category), category.name));
        for product in products.iter().filter(|item| product_category(item) == category.name) {
            if shown >= ALL_PRODUCTS_TEXT_LIMIT {
                break;
            }
            lines.push(format!(
                "• {} — {} ({})",
                product.name,
                format_vnd(product.price),
                product_stock_note(product),
            ));
            shown += 1;
        }
        lines.push(String::new());
    }
    if shown < products.len() {
        lines.push(format!("Đang hiển thị {shown}/{} sản phẩm đầu tiên.", products.len()));
    }
    trim_trailing_empty_lines(&mut lines);
    lines.join("\n")
}

fn category_keyboard_rows(categories: &[CategorySummary]) -> Vec<Vec<ButtonSpec>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    for (index, category) in categories.iter().enumerate() {
        row.push(category_button(category, format!("cat:{index}")));
        if row.len() == 3 {
            rows.push(row);
            row = Vec::new();
        }
    }
    if !row.is_empty() {
        rows.push(row);
    }
    rows.push(vec![button_spec("🔨 Tích hợp API", "home", None)]);
    rows.push(vec![button_spec("⬅️ Quay lại", "products", None)]);
    rows
}

fn trim_trailing_empty_lines(lines: &mut Vec<String>) {
    while lines.last().map(|line| line.is_empty()).unwrap_or(false) {
        lines.pop();
    }
}

fn product_stock_note(product: &ProductItem) -> String {
    if product.delivery_type == "manual_input" {
        "dịch vụ".to_string()
    } else {
        format!("còn {}", product.stock_count)
    }
}

fn category_counts(products: &[ProductItem]) -> Vec<CategorySummary> {
    let mut counts = BTreeMap::<String, CategorySummary>::new();
    for product in products {
        let name = product_category(product);
        let entry = counts.entry(name.clone()).or_insert_with(|| CategorySummary {
            name,
            count: 0,
            emoji: product.category_emoji.clone(),
            custom_emoji_id: product.category_custom_emoji_id.clone(),
        });
        entry.count += 1;
        if entry.emoji.is_none() {
            entry.emoji = product.category_emoji.clone();
        }
        if entry.custom_emoji_id.is_none() {
            entry.custom_emoji_id = product.category_custom_emoji_id.clone();
        }
    }
    counts.into_values().collect()
}

fn product_category(product: &ProductItem) -> String {
    product
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("Khác")
        .to_string()
}

fn category_icon(category: &str) -> &'static str {
    let normalized = category.to_ascii_uppercase();
    if normalized.contains("CANVA") {
        "🎨"
    } else if normalized.contains("CAPCUT") || normalized.contains("VIDEO") {
        "🎬"
    } else if normalized.contains("CHATGPT") || normalized.contains("GEMINI") || normalized.contains("AI") {
        "🤖"
    } else if normalized.contains("PROXY") || normalized.contains("IPV6") {
        "🌐"
    } else if normalized.contains("TUT") || normalized.contains("TRICK") {
        "📚"
    } else if normalized.contains("INSTAGRAM") {
        "📸"
    } else if normalized.contains("META") || normalized.contains("FACEBOOK") || normalized.contains("ACC") {
        "✅"
    } else if normalized.contains("KEY") || normalized.contains("HMA") {
        "🔑"
    } else if normalized.contains("SUPPORT") || normalized.contains("CHAT") {
        "💬"
    } else if normalized.contains("CLONE") {
        "📦"
    } else if normalized.contains("TẤT CẢ") || normalized.contains("TAT CA") {
        "🧰"
    } else {
        "✨"
    }
}

fn category_text_icon(category: &CategorySummary) -> String {
    clean_optional(&category.emoji).unwrap_or_else(|| category_icon(&category.name).to_string())
}

fn category_button(category: &CategorySummary, callback_data: String) -> ButtonSpec {
    let label = if category.count > 0 {
        format!("{} ({})", truncate_label(&category.name, 20), category.count)
    } else {
        truncate_label(&category.name, 20)
    };
    if let Some(icon_id) = clean_optional(&category.custom_emoji_id) {
        return button_spec(label, callback_data, Some(icon_id));
    }
    button_spec(format!("{} {label}", category_text_icon(category)), callback_data, None)
}

fn product_button(product: &ProductItem, callback_data: String) -> ButtonSpec {
    let label = truncate_label(&product.name, PRODUCT_BUTTON_NAME_MAX_CHARS);
    if let Some(icon_id) = product_icon_custom_id(product) {
        return button_spec(label, callback_data, Some(icon_id));
    }
    button_spec(format!("{} {label}", product_static_icon(product)), callback_data, None)
}

fn product_static_icon(product: &ProductItem) -> String {
    clean_optional(&product.button_emoji)
        .or_else(|| clean_optional(&product.category_emoji))
        .unwrap_or_else(|| category_icon(&product_category(product)).to_string())
}

fn product_icon_custom_id(product: &ProductItem) -> Option<String> {
    clean_optional(&product.button_custom_emoji_id)
        .or_else(|| clean_optional(&product.category_custom_emoji_id))
}

fn clean_optional(value: &Option<String>) -> Option<String> {
    value
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
}

fn button_spec(
    text: impl Into<String>,
    callback_data: impl Into<String>,
    icon_custom_emoji_id: Option<String>,
) -> ButtonSpec {
    ButtonSpec {
        text: text.into(),
        callback_data: callback_data.into(),
        icon_custom_emoji_id,
    }
}

fn json_keyboard(rows: Vec<Vec<ButtonSpec>>) -> Value {
    let inline_keyboard = rows
        .into_iter()
        .map(|row| {
            row.into_iter()
                .map(|button| {
                    let mut value = json!({
                        "text": button.text,
                        "callback_data": button.callback_data,
                    });
                    if let Some(icon_id) = button.icon_custom_emoji_id {
                        if let Some(obj) = value.as_object_mut() {
                            obj.insert("icon_custom_emoji_id".to_string(), Value::String(icon_id));
                        }
                    }
                    value
                })
                .collect::<Vec<_>>()
        })
        .collect::<Vec<_>>();
    json!({ "inline_keyboard": inline_keyboard })
}

async fn send_message_json_keyboard(
    ctx: &ChildBotContext,
    chat_id: ChatId,
    text: impl Into<String>,
    rows: Vec<Vec<ButtonSpec>>,
) -> Result<()> {
    let payload = json!({
        "chat_id": chat_id.0,
        "text": text.into(),
        "reply_markup": json_keyboard(rows),
    });
    send_raw_telegram_method(ctx, "sendMessage", payload).await
}

async fn send_raw_telegram_method(
    ctx: &ChildBotContext,
    method: &str,
    payload: Value,
) -> Result<()> {
    let url = format!(
        "https://api.telegram.org/bot{}/{}",
        ctx.config.telegram_token, method,
    );
    let response = ctx.http.post(url).json(&payload).send().await?;
    let status = response.status();
    let body = response.text().await?;
    if !status.is_success() {
        return Err(anyhow!("Telegram API {method} failed with {status}: {body}"));
    }
    let parsed: Value = serde_json::from_str(&body)?;
    if parsed.get("ok").and_then(Value::as_bool) == Some(true) {
        Ok(())
    } else {
        Err(anyhow!("Telegram API {method} returned error: {body}"))
    }
}

fn truncate_label(value: &str, max_chars: usize) -> String {
    let mut result = String::new();
    for (index, ch) in value.chars().enumerate() {
        if index >= max_chars {
            result.push('…');
            return result;
        }
        result.push(ch);
    }
    result
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
