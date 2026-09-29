use std::sync::Arc;

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    extract::{Path, State},
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use chrono::{Duration, Utc};
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};
use sqlx::FromRow;
use uuid::Uuid;

use crate::app::AppContext;
use crate::core::qr::vietqr_link;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::orders::api::RESERVE_TTL_MINUTES;
use crate::domains::orders::models::Order;
use crate::domains::orders::repo as orders_repo;
use crate::domains::products::models::{Product, ProductPlan};
use crate::domains::products::repo as products_repo;

const SHOP_SESSION_COOKIE: &str = "shop_session";
const SESSION_TTL_SECONDS: i64 = 30 * 24 * 60 * 60;
const MAX_ORDER_QUANTITY: i64 = 20;

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ShopClaims {
    sub: i64,
    username: String,
    exp: usize,
    iat: usize,
}

#[derive(Debug, Clone, FromRow)]
struct ShopCustomer {
    id: i64,
    username: String,
    password_hash: String,
    is_active: i64,
    created_at: String,
    last_login_at: Option<String>,
}

#[derive(Debug, Serialize)]
struct CustomerPublic {
    id: i64,
    username: String,
    created_at: String,
    last_login_at: Option<String>,
}

impl From<ShopCustomer> for CustomerPublic {
    fn from(value: ShopCustomer) -> Self {
        Self {
            id: value.id,
            username: value.username,
            created_at: value.created_at,
            last_login_at: value.last_login_at,
        }
    }
}

#[derive(Debug, Deserialize)]
struct CredentialsPayload {
    username: String,
    password: String,
}

#[derive(Debug, Deserialize)]
struct CreateOrderPayload {
    product_id: i64,
    quantity: Option<i64>,
    plan_id: Option<i64>,
    customer_input: Option<String>,
}

#[derive(Debug, Serialize)]
struct ProductPlanPublic {
    id: i64,
    label: String,
    months: i64,
    price: i64,
}

impl From<ProductPlan> for ProductPlanPublic {
    fn from(value: ProductPlan) -> Self {
        Self {
            id: value.id,
            label: value.label,
            months: value.months,
            price: value.price,
        }
    }
}

#[derive(Debug, Serialize)]
struct ShopProduct {
    id: i64,
    name: String,
    price: i64,
    description: Option<String>,
    image_url: Option<String>,
    category: Option<String>,
    category_emoji: Option<String>,
    delivery_type: Option<String>,
    requires_input: bool,
    input_prompt: Option<String>,
    stock_count: Option<i64>,
    plans: Vec<ProductPlanPublic>,
}

#[derive(Debug, Clone, FromRow)]
struct ShopOrderRow {
    id: String,
    product_id: i64,
    product_name: String,
    product_image_url: Option<String>,
    qty: i64,
    amount: i64,
    status: String,
    bank_memo: String,
    created_at: String,
    paid_at: Option<String>,
    plan_label: Option<String>,
    plan_months: Option<i64>,
    delivered_data: Option<String>,
}

#[derive(Debug, Serialize)]
struct ShopOrder {
    id: String,
    product_id: i64,
    product_name: String,
    product_image_url: Option<String>,
    quantity: i64,
    amount: i64,
    status: String,
    bank_memo: String,
    created_at: String,
    paid_at: Option<String>,
    expires_at: Option<String>,
    plan_label: Option<String>,
    plan_months: Option<i64>,
    delivered_data: Option<String>,
}

#[derive(Debug, Serialize)]
struct CreateOrderResponse {
    order: ShopOrder,
    payment: BankPayment,
}

#[derive(Debug, Serialize)]
struct ShopOrderDetail {
    order: ShopOrder,
    payment: Option<BankPayment>,
}

#[derive(Debug, Serialize)]
struct BankPayment {
    bank_name: String,
    account: String,
    account_name: Option<String>,
    amount: i64,
    memo: String,
    qr_url: String,
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new()
        .route("/api/shop/products", get(list_products))
        .route("/api/shop/auth/register", post(register))
        .route("/api/shop/auth/login", post(login))
        .route("/api/shop/auth/logout", post(logout))
        .route("/api/shop/auth/me", get(me))
        .route("/api/shop/orders", get(list_orders).post(create_order))
        .route("/api/shop/orders/:id", get(get_order))
}

async fn list_products(State(ctx): State<Arc<AppContext>>) -> ApiResult<Vec<ShopProduct>> {
    let products = products_repo::list_products(&ctx.pool, 100, 0)
        .await
        .map_err(|e| ApiError::internal(format!("list shop products failed: {e}")))?;

    let mut result = Vec::with_capacity(products.len());
    for product in products {
        result.push(shop_product(&ctx, product).await?);
    }

    Ok(ok(result))
}

async fn register(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<CredentialsPayload>,
) -> Result<Response, ApiError> {
    let username = normalize_username(&payload.username)?;
    validate_password(&payload.password)?;
    let password_hash = hash_password(&payload.password)?;

    let customer = sqlx::query_as::<_, ShopCustomer>(
        r#"INSERT INTO web_customers (username, password_hash)
        VALUES (?, ?)
        RETURNING id, username, password_hash, is_active, created_at, last_login_at"#,
    )
    .bind(&username)
    .bind(password_hash)
    .fetch_one(&ctx.pool)
    .await
    .map_err(|e| {
        if e.to_string().to_ascii_lowercase().contains("unique") {
            ApiError::new(StatusCode::CONFLICT, "USERNAME_TAKEN", "username is already in use")
        } else {
            ApiError::internal(format!("create shop customer failed: {e}"))
        }
    })?;

    session_response(&ctx, customer)
}

async fn login(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<CredentialsPayload>,
) -> Result<Response, ApiError> {
    let username = normalize_username(&payload.username)?;
    let Some(customer) = sqlx::query_as::<_, ShopCustomer>(
        r#"SELECT id, username, password_hash, is_active, created_at, last_login_at
        FROM web_customers WHERE username = ?"#,
    )
    .bind(&username)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("load shop customer failed: {e}")))?
    else {
        return Err(ApiError::unauthorized());
    };

    if customer.is_active != 1 || !verify_password(&payload.password, &customer.password_hash)? {
        return Err(ApiError::unauthorized());
    }

    sqlx::query("UPDATE web_customers SET last_login_at = datetime('now') WHERE id = ?")
        .bind(customer.id)
        .execute(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("update shop login failed: {e}")))?;

    session_response(&ctx, ShopCustomer { last_login_at: Some(Utc::now().to_rfc3339()), ..customer })
}

async fn logout(State(ctx): State<Arc<AppContext>>) -> Response {
    let mut response = Json(serde_json::json!({
        "ok": true,
        "data": { "success": true }
    }))
    .into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        clear_session_cookie(ctx.config.admin_cookie_secure)
            .parse()
            .expect("valid shop cookie"),
    );
    response
}

async fn me(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<CustomerPublic> {
    let customer = current_customer(&ctx, &headers).await?;
    Ok(ok(customer.into()))
}

async fn create_order(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Json(payload): Json<CreateOrderPayload>,
) -> Result<Json<crate::core::responses::ApiSuccess<CreateOrderResponse>>, ApiError> {
    let customer = current_customer(&ctx, &headers).await?;
    let Some(product) = products_repo::get_product(&ctx.pool, payload.product_id)
        .await
        .map_err(|e| ApiError::internal(format!("load shop product failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };
    if product.is_active == Some(0) {
        return Err(ApiError::new(StatusCode::CONFLICT, "PRODUCT_INACTIVE", "product is not available"));
    }

    let requested_qty = payload.quantity.unwrap_or(1);
    if !(1..=MAX_ORDER_QUANTITY).contains(&requested_qty) {
        return Err(ApiError::validation(format!("quantity must be 1..{MAX_ORDER_QUANTITY}")));
    }

    let input = payload
        .customer_input
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty());
    if product.requires_input.unwrap_or(0) == 1 && input.is_none() {
        return Err(ApiError::validation("this product requires your account information"));
    }
    if input.as_deref().is_some_and(|value| value.len() > 2000) {
        return Err(ApiError::validation("account information is too long"));
    }

    let (plan_label, plan_months, plan_price, amount) = if let Some(plan_id) = payload.plan_id {
        let Some(plan) = products_repo::get_product_plan(&ctx.pool, plan_id)
            .await
            .map_err(|e| ApiError::internal(format!("load product plan failed: {e}")))?
        else {
            return Err(ApiError::not_found("product plan not found"));
        };
        if plan.product_id != product.id {
            return Err(ApiError::validation("selected plan does not belong to this product"));
        }
        (Some(plan.label), Some(plan.months), Some(plan.price), plan.price)
    } else {
        (None, None, None, product.price * requested_qty)
    };

    let delivery_type = product.delivery_type.as_deref().unwrap_or("stock_item");
    let qty = if delivery_type == "manual_input" {
        plan_months.unwrap_or(requested_qty)
    } else {
        requested_qty
    };
    if delivery_type != "manual_input" {
        let stock = products_repo::count_product_items(&ctx.pool, product.id)
            .await
            .map_err(|e| ApiError::internal(format!("count product stock failed: {e}")))?;
        if stock < qty {
            return Err(ApiError::new(StatusCode::CONFLICT, "OUT_OF_STOCK", "this product does not have enough stock"));
        }
    }

    let bank_account = ctx.bank_account();
    if bank_account.trim().is_empty() {
        return Err(ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "PAYMENT_NOT_CONFIGURED", "bank payment is not configured yet"));
    }
    let memo = generate_memo(&ctx).await?;
    let mut order = Order::new(
        customer.id,
        customer.id,
        product.id,
        qty,
        amount,
        memo.clone(),
        input,
        payload.plan_id,
        plan_label,
        plan_months,
        plan_price,
    );

    if delivery_type == "manual_input" {
        let info = order.customer_input.clone().unwrap_or_else(|| "N/A".to_string());
        order.delivered_data = Some(format!("info: {info}"));
    }

    let mut tx = ctx
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(format!("start shop order failed: {e}")))?;
    orders_repo::insert_order_tx(&mut tx, &order)
        .await
        .map_err(|e| ApiError::internal(format!("insert shop order failed: {e}")))?;
    sqlx::query("UPDATE orders SET web_customer_id = ? WHERE id = ?")
        .bind(customer.id)
        .bind(&order.id)
        .execute(tx.as_mut())
        .await
        .map_err(|e| ApiError::internal(format!("link shop order failed: {e}")))?;
    tx.commit()
        .await
        .map_err(|e| ApiError::internal(format!("commit shop order failed: {e}")))?;

    let order_view = order_view_from_order(&order, &product);
    let payment = BankPayment {
        bank_name: ctx.bank_name(),
        account: bank_account.clone(),
        account_name: ctx.bank_account_name(),
        amount,
        memo: memo.clone(),
        qr_url: vietqr_link(&ctx.bank_name(), &bank_account, amount, &memo),
    };
    Ok(Json(crate::core::responses::ApiSuccess {
        ok: true,
        data: CreateOrderResponse {
            order: order_view,
            payment,
        },
    }))
}

async fn list_orders(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<Vec<ShopOrder>> {
    let customer = current_customer(&ctx, &headers).await?;
    let rows = sqlx::query_as::<_, ShopOrderRow>(
        r#"SELECT o.id, o.product_id, p.name AS product_name, p.image_url AS product_image_url,
            o.qty, o.amount, o.status, o.bank_memo, o.created_at, o.paid_at,
            o.plan_label, o.plan_months, o.delivered_data
        FROM orders o JOIN products p ON p.id = o.product_id
        WHERE o.web_customer_id = ?
        ORDER BY o.created_at DESC LIMIT 100"#,
    )
    .bind(customer.id)
    .fetch_all(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("list shop orders failed: {e}")))?;
    Ok(ok(rows.into_iter().map(order_view_from_row).collect()))
}

async fn get_order(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
    Path(order_id): Path<String>,
) -> ApiResult<ShopOrderDetail> {
    let customer = current_customer(&ctx, &headers).await?;
    let Some(row) = sqlx::query_as::<_, ShopOrderRow>(
        r#"SELECT o.id, o.product_id, p.name AS product_name, p.image_url AS product_image_url,
            o.qty, o.amount, o.status, o.bank_memo, o.created_at, o.paid_at,
            o.plan_label, o.plan_months, o.delivered_data
        FROM orders o JOIN products p ON p.id = o.product_id
        WHERE o.id = ? AND o.web_customer_id = ?"#,
    )
    .bind(order_id)
    .bind(customer.id)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("get shop order failed: {e}")))?
    else {
        return Err(ApiError::not_found("order not found"));
    };
    let order = order_view_from_row(row);
    let payment = if order.status == "pending" {
        let account = ctx.bank_account();
        Some(BankPayment {
            bank_name: ctx.bank_name(),
            account: account.clone(),
            account_name: ctx.bank_account_name(),
            amount: order.amount,
            memo: order.bank_memo.clone(),
            qr_url: vietqr_link(&ctx.bank_name(), &account, order.amount, &order.bank_memo),
        })
    } else {
        None
    };
    Ok(ok(ShopOrderDetail { order, payment }))
}

async fn shop_product(ctx: &AppContext, product: Product) -> Result<ShopProduct, ApiError> {
    let plans = products_repo::list_product_plans(&ctx.pool, product.id)
        .await
        .map_err(|e| ApiError::internal(format!("list product plans failed: {e}")))?
        .into_iter()
        .map(ProductPlanPublic::from)
        .collect();
    let stock_count = if product.delivery_type.as_deref() == Some("manual_input") {
        None
    } else {
        Some(
            products_repo::count_product_items(&ctx.pool, product.id)
                .await
                .map_err(|e| ApiError::internal(format!("count product stock failed: {e}")))?,
        )
    };
    Ok(ShopProduct {
        id: product.id,
        name: product.name,
        price: product.price,
        description: product.description,
        image_url: product.image_url,
        category: product.category,
        category_emoji: product.category_emoji,
        delivery_type: product.delivery_type,
        requires_input: product.requires_input.unwrap_or(0) == 1,
        input_prompt: product.input_prompt,
        stock_count,
        plans,
    })
}

fn order_view_from_row(row: ShopOrderRow) -> ShopOrder {
    let delivered_data = if row.status == "paid" { row.delivered_data } else { None };
    ShopOrder {
        id: row.id,
        product_id: row.product_id,
        product_name: row.product_name,
        product_image_url: row.product_image_url,
        quantity: row.qty,
        amount: row.amount,
        status: row.status.clone(),
        bank_memo: row.bank_memo,
        created_at: row.created_at.clone(),
        paid_at: row.paid_at,
        expires_at: if row.status == "pending" {
            DateTimeString::from_created(&row.created_at)
        } else {
            None
        },
        plan_label: row.plan_label,
        plan_months: row.plan_months,
        delivered_data,
    }
}

struct DateTimeString;

impl DateTimeString {
    fn from_created(created_at: &str) -> Option<String> {
        chrono::DateTime::parse_from_rfc3339(created_at)
            .ok()
            .map(|value| (value.with_timezone(&Utc) + Duration::minutes(RESERVE_TTL_MINUTES)).to_rfc3339())
    }
}

fn order_view_from_order(order: &Order, product: &Product) -> ShopOrder {
    ShopOrder {
        id: order.id.clone(),
        product_id: product.id,
        product_name: product.name.clone(),
        product_image_url: product.image_url.clone(),
        quantity: order.qty,
        amount: order.amount,
        status: order.status.to_string(),
        bank_memo: order.bank_memo.clone(),
        created_at: order.created_at.clone(),
        paid_at: order.paid_at.clone(),
        expires_at: Some((Utc::now() + Duration::minutes(RESERVE_TTL_MINUTES)).to_rfc3339()),
        plan_label: order.plan_label.clone(),
        plan_months: order.plan_months,
        delivered_data: None,
    }
}

async fn current_customer(ctx: &AppContext, headers: &HeaderMap) -> Result<ShopCustomer, ApiError> {
    let claims = decode_session_cookie(ctx, headers).ok_or_else(ApiError::unauthorized)?;
    let Some(customer) = sqlx::query_as::<_, ShopCustomer>(
        r#"SELECT id, username, password_hash, is_active, created_at, last_login_at
        FROM web_customers WHERE id = ?"#,
    )
    .bind(claims.sub)
    .fetch_optional(&ctx.pool)
    .await
    .map_err(|e| ApiError::internal(format!("load shop session failed: {e}")))?
    else {
        return Err(ApiError::unauthorized());
    };
    if customer.is_active != 1 || customer.username != claims.username {
        return Err(ApiError::unauthorized());
    }
    Ok(customer)
}

fn session_response(ctx: &AppContext, customer: ShopCustomer) -> Result<Response, ApiError> {
    let expires_at = Utc::now().timestamp() + SESSION_TTL_SECONDS;
    let claims = ShopClaims {
        sub: customer.id,
        username: customer.username.clone(),
        exp: expires_at as usize,
        iat: Utc::now().timestamp() as usize,
    };
    let secret = format!("shop:{}", ctx.config.admin_jwt_secret);
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .map_err(|e| ApiError::internal(format!("encode shop session failed: {e}")))?;
    let mut response = ok(CustomerPublic::from(customer)).into_response();
    response.headers_mut().insert(
        header::SET_COOKIE,
        session_cookie(&token, ctx.config.admin_cookie_secure)
            .parse()
            .expect("valid shop cookie"),
    );
    Ok(response)
}

fn decode_session_cookie(ctx: &AppContext, headers: &HeaderMap) -> Option<ShopClaims> {
    let token = cookie_value(headers, SHOP_SESSION_COOKIE)?;
    let secret = format!("shop:{}", ctx.config.admin_jwt_secret);
    decode::<ShopClaims>(
        &token,
        &DecodingKey::from_secret(secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
}

fn cookie_value(headers: &HeaderMap, name: &str) -> Option<String> {
    headers.get(header::COOKIE)?.to_str().ok()?.split(';').find_map(|part| {
        let (key, value) = part.trim().split_once('=')?;
        (key == name).then(|| value.to_string())
    })
}

fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{SHOP_SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={SESSION_TTL_SECONDS}{secure}"
    )
}

fn clear_session_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{SHOP_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{secure}")
}

fn normalize_username(raw: &str) -> Result<String, ApiError> {
    let username = raw.trim().to_ascii_lowercase();
    if !(3..=64).contains(&username.len()) {
        return Err(ApiError::validation("username must be 3..64 characters"));
    }
    if !username
        .chars()
        .all(|value| value.is_ascii_alphanumeric() || matches!(value, '_' | '-' | '.'))
    {
        return Err(ApiError::validation("username may contain letters, numbers, dot, dash, underscore"));
    }
    Ok(username)
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if !(8..=256).contains(&password.len()) {
        return Err(ApiError::validation("password must be 8..256 characters"));
    }
    Ok(())
}

fn hash_password(password: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| ApiError::internal(format!("hash shop password failed: {e}")))
}

fn verify_password(password: &str, password_hash: &str) -> Result<bool, ApiError> {
    let parsed = PasswordHash::new(password_hash)
        .map_err(|e| ApiError::internal(format!("parse shop password failed: {e}")))?;
    Ok(Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok())
}

async fn generate_memo(ctx: &AppContext) -> Result<String, ApiError> {
    for _ in 0..8 {
        let suffix = Uuid::new_v4()
            .simple()
            .to_string()
            .chars()
            .take(ctx.order_memo_length())
            .collect::<String>()
            .to_ascii_uppercase();
        let memo = format!("{}{suffix}", ctx.order_memo_prefix());
        let exists = sqlx::query_scalar::<_, i64>("SELECT COUNT(1) FROM orders WHERE bank_memo = ?")
            .bind(&memo)
            .fetch_one(&ctx.pool)
            .await
            .map_err(|e| ApiError::internal(format!("check order memo failed: {e}")))?;
        if exists == 0 {
            return Ok(memo);
        }
    }
    Err(ApiError::internal("could not create a unique payment memo"))
}
