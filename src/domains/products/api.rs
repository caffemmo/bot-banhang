use std::sync::Arc;

use axum::{
    Json,
    extract::{Multipart, Path, Query, State},
};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use tokio::fs;
use uuid::Uuid;

use crate::app::AppContext;
use crate::domains::products::models::{Product, ProductCategory, ProductItem, ProductPlan};
use crate::domains::products::repo;

use crate::core::pagination::normalize_pagination;
use crate::core::responses::{Ack, ApiError, ApiResult, PaginatedResponse, ok};
use crate::domains::orders::api::uploaded_file_delivery_payload;
use crate::domains::users::broadcast as users_broadcast;

#[derive(Debug, Deserialize)]
pub struct ListProductsQuery {
    pub limit: Option<i64>,
    pub offset: Option<i64>,
    pub active: Option<String>,
    pub query: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProductPayload {
    pub name: String,
    pub price: Option<i64>,
    pub is_active: Option<i64>,
    pub requires_input: Option<i64>,
    pub input_prompt: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub delivery_type: Option<String>,
    pub file_path: Option<String>,
    pub file_name: Option<String>,
    pub file_mime: Option<String>,
    pub category_id: Option<i64>,
    pub category: Option<String>,
    pub button_emoji: Option<String>,
    pub button_custom_emoji_id: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct ProductCategoryPayload {
    pub name: String,
    pub emoji: Option<String>,
    pub custom_emoji_id: Option<String>,
    pub sort_order: Option<i64>,
    pub is_active: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct ToggleProductPayload {
    pub is_active: i64,
}

#[derive(Debug, Serialize)]
pub struct ProductListItem {
    pub id: i64,
    pub name: String,
    pub price: i64,
    pub is_active: Option<i64>,
    pub requires_input: Option<i64>,
    pub input_prompt: Option<String>,
    pub description: Option<String>,
    pub image_url: Option<String>,
    pub delivery_type: Option<String>,
    pub file_path: Option<String>,
    pub file_name: Option<String>,
    pub file_mime: Option<String>,
    pub category_id: Option<i64>,
    pub category: Option<String>,
    pub category_emoji: Option<String>,
    pub category_custom_emoji_id: Option<String>,
    pub button_emoji: Option<String>,
    pub button_custom_emoji_id: Option<String>,
    pub created_at: Option<String>,
    pub sort_order: Option<i64>,
    pub stock_count: i64,
}

impl From<Product> for ProductListItem {
    fn from(value: Product) -> Self {
        Self {
            id: value.id,
            name: value.name,
            price: value.price,
            is_active: value.is_active,
            requires_input: value.requires_input,
            input_prompt: value.input_prompt,
            description: value.description,
            image_url: value.image_url,
            delivery_type: value.delivery_type,
            file_path: value.file_path,
            file_name: value.file_name,
            file_mime: value.file_mime,
            category_id: value.category_id,
            category: value.category,
            category_emoji: value.category_emoji,
            category_custom_emoji_id: value.category_custom_emoji_id,
            button_emoji: value.button_emoji,
            button_custom_emoji_id: value.button_custom_emoji_id,
            created_at: value.created_at,
            sort_order: value.sort_order,
            stock_count: 0,
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct ProductItemsPayload {
    pub items: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct PlanPayload {
    pub label: String,
    pub months: i64,
    pub price: i64,
    pub sort_order: Option<i64>,
}

pub async fn list_products(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Query(params): Query<ListProductsQuery>,
) -> ApiResult<PaginatedResponse<ProductListItem>> {
    let (limit, offset) = normalize_pagination(params.limit, params.offset);
    let active = parse_active_filter(params.active.as_deref());

    let products =
        repo::list_products_filtered(&ctx.pool, limit, offset, active, params.query.as_deref())
            .await
            .map_err(|e| ApiError::internal(format!("list products failed: {e}")))?;
    let total = repo::count_products_filtered(&ctx.pool, active, params.query.as_deref())
        .await
        .map_err(|e| ApiError::internal(format!("count products failed: {e}")))?;

    let mut items = Vec::new();
    for p in products {
        let mut item = ProductListItem::from(p.clone());
        item.stock_count = repo::count_product_items(&ctx.pool, p.id)
            .await
            .unwrap_or(0);
        items.push(item);
    }

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

pub async fn get_product_handler(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Product> {
    let Some(product) = repo::get_product(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    Ok(ok(product))
}

pub async fn list_product_categories(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
) -> ApiResult<Vec<ProductCategory>> {
    let categories = repo::list_product_categories(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("list product categories failed: {e}")))?;

    Ok(ok(categories))
}

pub async fn create_product_category(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Json(payload): Json<ProductCategoryPayload>,
) -> ApiResult<ProductCategory> {
    validate_product_category_payload(&payload)?;
    let name = normalize_category_name(&payload.name)?;
    let emoji = normalize_optional_text(payload.emoji.as_deref(), 16);
    let custom_emoji_id = normalize_custom_emoji_id(payload.custom_emoji_id.as_deref());
    let is_active = normalize_active_value(payload.is_active)?;

    let category = repo::insert_product_category(
        &ctx.pool,
        &name,
        emoji.as_deref(),
        custom_emoji_id.as_deref(),
        payload.sort_order,
        Some(is_active),
    )
    .await
    .map_err(|e| ApiError::internal(format!("create product category failed: {e}")))?;

    Ok(ok(category))
}

pub async fn update_product_category_handler(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(payload): Json<ProductCategoryPayload>,
) -> ApiResult<ProductCategory> {
    validate_product_category_payload(&payload)?;
    let name = normalize_category_name(&payload.name)?;
    let emoji = normalize_optional_text(payload.emoji.as_deref(), 16);
    let custom_emoji_id = normalize_custom_emoji_id(payload.custom_emoji_id.as_deref());
    let is_active = normalize_active_value(payload.is_active)?;

    let category = repo::update_product_category(
        &ctx.pool,
        id,
        &name,
        emoji.as_deref(),
        custom_emoji_id.as_deref(),
        payload.sort_order,
        Some(is_active),
    )
    .await
    .map_err(|e| ApiError::internal(format!("update product category failed: {e}")))?;

    let Some(category) = category else {
        return Err(ApiError::not_found("product category not found"));
    };

    Ok(ok(category))
}

pub async fn delete_product_category(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Ack> {
    let updated = repo::set_product_category_active(&ctx.pool, id, 0)
        .await
        .map_err(|e| ApiError::internal(format!("delete product category failed: {e}")))?;
    if !updated {
        return Err(ApiError::not_found("product category not found"));
    }

    Ok(ok(Ack { success: true }))
}

pub async fn create_product(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Json(payload): Json<ProductPayload>,
) -> ApiResult<Product> {
    validate_product_payload(&payload)?;
    let is_active = normalize_active_value(payload.is_active)?;
    let legacy_requires_input = normalize_bool_value(payload.requires_input, 0)?;
    let delivery_type =
        normalize_delivery_type(payload.delivery_type.as_deref(), legacy_requires_input)?;
    let requires_input = requires_input_for_delivery_type(&delivery_type);
    let input_prompt = payload.input_prompt.as_deref();
    let description = payload.description.as_deref();
    let image_url = payload.image_url.as_deref();
    let (category_id, category) = resolve_product_category(&ctx.pool, &payload).await?;
    let button_emoji = normalize_optional_text(payload.button_emoji.as_deref(), 16);
    let button_custom_emoji_id =
        normalize_custom_emoji_id(payload.button_custom_emoji_id.as_deref());
    let price_val = normalize_price(payload.price, &delivery_type)?;
    let product = repo::insert_product(
        &ctx.pool,
        &payload.name,
        price_val,
        Some(is_active),
        Some(requires_input),
        input_prompt,
        description,
        image_url,
        Some(&delivery_type),
        payload.file_path.as_deref(),
        payload.file_name.as_deref(),
        payload.file_mime.as_deref(),
        category_id,
        category.as_deref(),
        button_emoji.as_deref(),
        button_custom_emoji_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("create product failed: {e}")))?;

    Ok(ok(product))
}

pub async fn update_product_handler(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(payload): Json<ProductPayload>,
) -> ApiResult<Product> {
    validate_product_payload(&payload)?;
    let is_active = normalize_active_value(payload.is_active)?;
    let legacy_requires_input = normalize_bool_value(payload.requires_input, 0)?;
    let delivery_type =
        normalize_delivery_type(payload.delivery_type.as_deref(), legacy_requires_input)?;
    let requires_input = requires_input_for_delivery_type(&delivery_type);
    let existing = repo::get_product(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?;
    let Some(existing) = existing else {
        return Err(ApiError::not_found("product not found"));
    };
    let input_prompt = payload.input_prompt.as_deref();
    let description = payload.description.as_deref();
    let image_url = payload
        .image_url
        .as_deref()
        .or(existing.image_url.as_deref());
    let (category_id, category) = resolve_product_category(&ctx.pool, &payload).await?;
    let button_emoji = normalize_optional_text(payload.button_emoji.as_deref(), 16);
    let button_custom_emoji_id =
        normalize_custom_emoji_id(payload.button_custom_emoji_id.as_deref());
    let price_val = normalize_price(payload.price, &delivery_type)?;
    let product = repo::update_product(
        &ctx.pool,
        id,
        &payload.name,
        price_val,
        Some(is_active),
        Some(requires_input),
        input_prompt,
        description,
        image_url,
        Some(&delivery_type),
        existing.file_path.as_deref(),
        existing.file_name.as_deref(),
        existing.file_mime.as_deref(),
        category_id,
        category.as_deref(),
        button_emoji.as_deref(),
        button_custom_emoji_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("update product failed: {e}")))?;

    let Some(product) = product else {
        return Err(ApiError::not_found("product not found"));
    };

    Ok(ok(product))
}

pub async fn toggle_product_active(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(payload): Json<ToggleProductPayload>,
) -> ApiResult<Ack> {
    let is_active = normalize_active_value(Some(payload.is_active))?;
    let updated = repo::set_product_active(&ctx.pool, id, is_active)
        .await
        .map_err(|e| ApiError::internal(format!("update product failed: {e}")))?;
    if !updated {
        return Err(ApiError::not_found("product not found"));
    }

    Ok(ok(Ack { success: true }))
}

pub async fn delete_product(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Ack> {
    let updated = repo::set_product_active(&ctx.pool, id, 0)
        .await
        .map_err(|e| ApiError::internal(format!("delete product failed: {e}")))?;
    if !updated {
        return Err(ApiError::not_found("product not found"));
    }

    Ok(ok(Ack { success: true }))
}

pub async fn add_product_items(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(payload): Json<ProductItemsPayload>,
) -> ApiResult<Ack> {
    let items: Vec<String> = payload
        .items
        .into_iter()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();
    if items.is_empty() {
        return Err(ApiError::validation("items cannot be empty"));
    }

    let inserted = repo::insert_product_items(&ctx.pool, id, &items)
        .await
        .map_err(|e| ApiError::internal(format!("insert items failed: {e}")))?;
    if inserted > 0
        && let Some(product) = repo::get_product(&ctx.pool, id)
            .await
            .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?
    {
        let current_stock = repo::count_product_items(&ctx.pool, id).await.unwrap_or(0);
        users_broadcast::notify_stock_added(ctx.clone(), product, inserted, current_stock).await;
    }
    Ok(ok(Ack {
        success: inserted > 0,
    }))
}

pub async fn list_product_items(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Query(paging): Query<ListProductsQuery>,
) -> ApiResult<PaginatedResponse<ProductItem>> {
    let (limit, offset) = normalize_pagination(paging.limit, paging.offset);
    let items = repo::list_product_items(&ctx.pool, id, limit, offset)
        .await
        .map_err(|e| ApiError::internal(format!("list items failed: {e}")))?;
    let total = repo::count_product_items(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("count items failed: {e}")))?;

    Ok(ok(PaginatedResponse {
        items,
        limit,
        offset,
        total,
    }))
}

pub async fn product_stock(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<serde_json::Value> {
    let count = repo::count_product_items(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("count stock failed: {e}")))?;
    Ok(ok(serde_json::json!({ "count": count })))
}

pub async fn list_product_plans(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Vec<ProductPlan>> {
    let plans = repo::list_product_plans(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("list plans failed: {e}")))?;
    Ok(ok(plans))
}

pub async fn add_product_plan(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    Json(payload): Json<PlanPayload>,
) -> ApiResult<ProductPlan> {
    validate_plan_payload(&payload)?;
    let plan = repo::insert_product_plan(
        &ctx.pool,
        id,
        &payload.label,
        payload.months,
        payload.price,
        payload.sort_order,
    )
    .await
    .map_err(|e| ApiError::internal(format!("create plan failed: {e}")))?;
    Ok(ok(plan))
}

pub async fn update_product_plan(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(plan_id): Path<i64>,
    Json(payload): Json<PlanPayload>,
) -> ApiResult<ProductPlan> {
    validate_plan_payload(&payload)?;
    let Some(plan) = repo::update_product_plan(
        &ctx.pool,
        plan_id,
        &payload.label,
        payload.months,
        payload.price,
        payload.sort_order,
    )
    .await
    .map_err(|e| ApiError::internal(format!("update plan failed: {e}")))?
    else {
        return Err(ApiError::not_found("plan not found"));
    };
    Ok(ok(plan))
}

pub async fn delete_product_plan(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(plan_id): Path<i64>,
) -> ApiResult<Ack> {
    let deleted = repo::delete_product_plan(&ctx.pool, plan_id)
        .await
        .map_err(|e| ApiError::internal(format!("delete plan failed: {e}")))?;
    if deleted == 0 {
        return Err(ApiError::not_found("plan not found"));
    }
    Ok(ok(Ack { success: true }))
}

pub async fn delete_product_item_handler(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path((id, item_id)): Path<(i64, i64)>,
) -> ApiResult<Ack> {
    let deleted = repo::delete_product_item(&ctx.pool, id, item_id)
        .await
        .map_err(|e| ApiError::internal(format!("delete item failed: {e}")))?;
    Ok(ok(Ack {
        success: deleted > 0,
    }))
}

fn parse_active_filter(raw: Option<&str>) -> Option<i64> {
    match raw {
        Some("1") => Some(1),
        Some("0") => Some(0),
        _ => None,
    }
}

fn validate_product_payload(payload: &ProductPayload) -> Result<(), ApiError> {
    if payload.name.trim().is_empty() || payload.name.len() > 200 {
        return Err(ApiError::validation("name must be 1..200 chars"));
    }

    normalize_active_value(payload.is_active)?;
    let legacy_requires_input = normalize_bool_value(payload.requires_input, 0)?;
    let delivery_type =
        normalize_delivery_type(payload.delivery_type.as_deref(), legacy_requires_input)?;
    if let Some(prompt) = &payload.input_prompt
        && prompt.len() > 200
    {
        return Err(ApiError::validation("input_prompt must be <= 200 chars"));
    }
    if let Some(category) = &payload.category
        && category.trim().chars().count() > 64
    {
        return Err(ApiError::validation("category must be <= 64 chars"));
    }
    if let Some(button_emoji) = &payload.button_emoji
        && button_emoji.trim().chars().count() > 16
    {
        return Err(ApiError::validation("button_emoji must be <= 16 chars"));
    }
    if let Some(custom_id) = &payload.button_custom_emoji_id
        && !custom_id.trim().chars().all(|c| c.is_ascii_digit())
    {
        return Err(ApiError::validation(
            "button_custom_emoji_id must contain digits only",
        ));
    }
    normalize_price(payload.price, &delivery_type)?;
    Ok(())
}

fn validate_product_category_payload(payload: &ProductCategoryPayload) -> Result<(), ApiError> {
    normalize_category_name(&payload.name)?;
    normalize_active_value(payload.is_active)?;
    if let Some(emoji) = &payload.emoji
        && emoji.trim().chars().count() > 16
    {
        return Err(ApiError::validation("emoji must be <= 16 chars"));
    }
    if let Some(custom_id) = &payload.custom_emoji_id
        && !custom_id.trim().chars().all(|c| c.is_ascii_digit())
    {
        return Err(ApiError::validation(
            "custom_emoji_id must contain digits only",
        ));
    }
    Ok(())
}

fn normalize_category_name(value: &str) -> Result<String, ApiError> {
    let trimmed = value.trim();
    if trimmed.is_empty() || trimmed.chars().count() > 64 {
        return Err(ApiError::validation("category name must be 1..64 chars"));
    }
    Ok(trimmed.to_string())
}

async fn resolve_product_category(
    pool: &SqlitePool,
    payload: &ProductPayload,
) -> Result<(Option<i64>, Option<String>), ApiError> {
    let Some(category_id) = payload.category_id.filter(|id| *id > 0) else {
        return Ok((
            None,
            normalize_optional_text(payload.category.as_deref(), 64),
        ));
    };

    let Some(category) = repo::get_product_category(pool, category_id)
        .await
        .map_err(|e| ApiError::internal(format!("get product category failed: {e}")))?
    else {
        return Err(ApiError::validation("category_id is invalid"));
    };

    Ok((Some(category.id), Some(category.name)))
}

fn normalize_custom_emoji_id(value: Option<&str>) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .filter(|s| s.chars().all(|c| c.is_ascii_digit()))
        .map(ToOwned::to_owned)
}

fn normalize_optional_text(value: Option<&str>, max_chars: usize) -> Option<String> {
    value
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(|s| s.chars().take(max_chars).collect())
}

fn normalize_active_value(value: Option<i64>) -> Result<i64, ApiError> {
    match value {
        Some(0) => Ok(0),
        Some(1) | None => Ok(1),
        Some(_) => Err(ApiError::validation("is_active must be 0 or 1")),
    }
}

fn normalize_bool_value(value: Option<i64>, default: i64) -> Result<i64, ApiError> {
    match value {
        Some(0) => Ok(0),
        Some(1) => Ok(1),
        None => Ok(default),
        Some(_) => Err(ApiError::validation("value must be 0 or 1")),
    }
}

pub fn normalize_delivery_type(raw: Option<&str>, requires_input: i64) -> Result<String, ApiError> {
    let value = raw
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(if requires_input == 1 {
            "manual_input"
        } else {
            "stock_item"
        });
    match value {
        "stock_item" | "uploaded_file" | "manual_input" => Ok(value.to_string()),
        _ => Err(ApiError::validation(
            "delivery_type must be stock_item, uploaded_file, or manual_input",
        )),
    }
}

pub fn requires_input_for_delivery_type(delivery_type: &str) -> i64 {
    if delivery_type == "manual_input" {
        1
    } else {
        0
    }
}

fn normalize_price(price: Option<i64>, delivery_type: &str) -> Result<i64, ApiError> {
    if delivery_type == "manual_input" {
        // giá sẽ lấy theo plan; lưu 0 để tránh yêu cầu nhập
        Ok(price.unwrap_or(0))
    } else {
        let p = price.unwrap_or(0);
        if p <= 0 {
            return Err(ApiError::validation("price must be > 0"));
        }
        Ok(p)
    }
}

fn sanitize_storage_filename(original: &str) -> String {
    let sanitized: String = original
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let trimmed = sanitized.trim_matches('_').trim_matches('.');
    if trimmed.is_empty() {
        "file".to_string()
    } else {
        trimmed.chars().take(120).collect()
    }
}

fn validate_plan_payload(payload: &PlanPayload) -> Result<(), ApiError> {
    if payload.label.trim().is_empty() || payload.label.len() > 100 {
        return Err(ApiError::validation("label must be 1..100 chars"));
    }
    if payload.months <= 0 {
        return Err(ApiError::validation("months must be > 0"));
    }
    if payload.price <= 0 {
        return Err(ApiError::validation("price must be > 0"));
    }
    Ok(())
}

pub async fn upload_product_image(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> ApiResult<Product> {
    let mut image_data: Option<(Vec<u8>, String)> = None;

    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("parse multipart failed: {e}")))?
    {
        if field.name() == Some("image") {
            let filename = field.file_name().unwrap_or("image.jpg").to_string();
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::internal(format!("read image failed: {e}")))?;
            image_data = Some((data.to_vec(), filename));
            break;
        }
    }

    let Some((bytes, original_filename)) = image_data else {
        return Err(ApiError::validation("image is required"));
    };

    fs::create_dir_all("storage/uploads")
        .await
        .map_err(|e| ApiError::internal(format!("prepare upload dir failed: {e}")))?;

    let ext = std::path::Path::new(&original_filename)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg");
    let filename = format!("product_{}_{}.{}", id, chrono::Utc::now().timestamp(), ext);
    let filepath = format!("storage/uploads/{}", filename);
    let image_url = format!("/uploads/{}", filename);

    fs::write(&filepath, bytes)
        .await
        .map_err(|e| ApiError::internal(format!("save image failed: {e}")))?;

    let product = repo::get_product(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?;

    let Some(p) = product else {
        return Err(ApiError::not_found("product not found"));
    };

    let updated = repo::update_product(
        &ctx.pool,
        id,
        &p.name,
        p.price,
        p.is_active,
        p.requires_input,
        p.input_prompt.as_deref(),
        p.description.as_deref(),
        Some(&image_url),
        p.delivery_type.as_deref(),
        p.file_path.as_deref(),
        p.file_name.as_deref(),
        p.file_mime.as_deref(),
        p.category_id,
        p.category.as_deref(),
        p.button_emoji.as_deref(),
        p.button_custom_emoji_id.as_deref(),
    )
    .await
    .map_err(|e| ApiError::internal(format!("update product image failed: {e}")))?;

    Ok(ok(updated.unwrap()))
}

fn uploaded_image_storage_path(image_url: &str) -> Option<String> {
    let relative = image_url.trim().trim_start_matches('/');
    let filename = relative.strip_prefix("uploads/")?;
    if filename.is_empty()
        || filename.contains('/')
        || filename.contains('\\')
        || filename == "."
        || filename == ".."
    {
        return None;
    }
    Some(format!("storage/uploads/{filename}"))
}

pub async fn delete_product_image(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Product> {
    let Some(existing) = repo::get_product(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    if let Some(image_url) = existing.image_url.as_deref()
        && let Some(path) = uploaded_image_storage_path(image_url)
    {
        let _ = fs::remove_file(path).await;
    }

    let Some(product) = repo::update_product_image_url(&ctx.pool, id, None)
        .await
        .map_err(|e| ApiError::internal(format!("clear product image failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    Ok(ok(product))
}

pub async fn upload_product_file(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
    mut multipart: Multipart,
) -> ApiResult<Product> {
    let Some(existing) = repo::get_product(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    let mut uploaded: Vec<(Vec<u8>, String, Option<String>)> = Vec::new();
    while let Some(field) = multipart
        .next_field()
        .await
        .map_err(|e| ApiError::internal(format!("parse multipart failed: {e}")))?
    {
        if field.name() == Some("file") {
            let filename = field.file_name().unwrap_or("file").to_string();
            let mime = field.content_type().map(|s| s.to_string());
            let data = field
                .bytes()
                .await
                .map_err(|e| ApiError::internal(format!("read file failed: {e}")))?;
            uploaded.push((data.to_vec(), filename, mime));
        }
    }

    if uploaded.is_empty() {
        return Err(ApiError::validation("file is required"));
    }

    fs::create_dir_all("storage/product_files")
        .await
        .map_err(|e| ApiError::internal(format!("create storage failed: {e}")))?;

    let mut item_payloads = Vec::new();
    let mut first_file: Option<(String, String, Option<String>)> = None;
    for (bytes, original_filename, mime) in uploaded {
        let safe_name = sanitize_storage_filename(&original_filename);
        let filename = format!("product_{}_{}_{}", id, Uuid::new_v4(), safe_name);
        let filepath = format!("storage/product_files/{filename}");

        fs::write(&filepath, bytes)
            .await
            .map_err(|e| ApiError::internal(format!("save file failed: {e}")))?;

        let payload =
            uploaded_file_delivery_payload(&filepath, &original_filename, mime.as_deref())
                .map_err(|e| ApiError::internal(format!("serialize file metadata failed: {e}")))?;
        item_payloads.push(payload);

        if first_file.is_none() {
            first_file = Some((filepath, original_filename, mime));
        }
    }

    let inserted = repo::insert_product_items(&ctx.pool, id, &item_payloads)
        .await
        .map_err(|e| ApiError::internal(format!("insert uploaded file items failed: {e}")))?;

    let Some((first_path, first_name, first_mime)) = first_file else {
        return Err(ApiError::validation("file is required"));
    };
    let file_path = existing.file_path.as_deref().unwrap_or(&first_path);
    let file_name = existing.file_name.as_deref().unwrap_or(&first_name);
    let file_mime = existing.file_mime.as_deref().or(first_mime.as_deref());

    let updated = repo::update_product_file_metadata(
        &ctx.pool,
        id,
        Some(file_path),
        Some(file_name),
        file_mime,
    )
    .await
    .map_err(|e| ApiError::internal(format!("update product file failed: {e}")))?;

    let Some(product) = updated else {
        return Err(ApiError::not_found("product not found"));
    };
    let current_stock = repo::count_product_items(&ctx.pool, id).await.unwrap_or(0);
    users_broadcast::notify_stock_added(ctx.clone(), product.clone(), inserted, current_stock)
        .await;
    Ok(ok(product))
}

pub async fn delete_product_file(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Path(id): Path<i64>,
) -> ApiResult<Product> {
    let Some(existing) = repo::get_product(&ctx.pool, id)
        .await
        .map_err(|e| ApiError::internal(format!("get product failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    if let Some(path) = existing.file_path.as_deref() {
        let _ = fs::remove_file(path).await;
    }

    let Some(product) = repo::update_product_file_metadata(&ctx.pool, id, None, None, None)
        .await
        .map_err(|e| ApiError::internal(format!("clear product file failed: {e}")))?
    else {
        return Err(ApiError::not_found("product not found"));
    };

    Ok(ok(product))
}

#[derive(Debug, Deserialize)]
pub struct ReorderItem {
    pub id: i64,
    pub sort_order: i64,
}

#[derive(Debug, Deserialize)]
pub struct ReorderPayload {
    pub items: Vec<ReorderItem>,
}

pub async fn reorder_products(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Json(payload): Json<ReorderPayload>,
) -> ApiResult<Ack> {
    let mut tx = ctx
        .pool
        .begin()
        .await
        .map_err(|e| ApiError::internal(format!("begin transaction failed: {e}")))?;

    for item in payload.items {
        sqlx::query("UPDATE products SET sort_order = ? WHERE id = ?")
            .bind(item.sort_order)
            .bind(item.id)
            .execute(&mut *tx)
            .await
            .map_err(|e| ApiError::internal(format!("update sort_order failed: {e}")))?;
    }

    tx.commit()
        .await
        .map_err(|e| ApiError::internal(format!("commit transaction failed: {e}")))?;

    Ok(ok(Ack { success: true }))
}

use axum::Router;
use axum::routing::{delete, get, post, put};

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route(
            "/api/admin/product-categories",
            get(list_product_categories).post(create_product_category),
        )
        .route(
            "/api/admin/product-categories/:id",
            put(update_product_category_handler).delete(delete_product_category),
        )
        .route("/api/admin/products/reorder", post(reorder_products))
        .route(
            "/api/admin/products",
            get(list_products).post(create_product),
        )
        .route(
            "/api/admin/products/:id",
            get(get_product_handler)
                .put(update_product_handler)
                .delete(delete_product),
        )
        .route(
            "/api/admin/products/:id/toggle",
            post(toggle_product_active),
        )
        .route(
            "/api/admin/products/:id/image",
            post(upload_product_image).delete(delete_product_image),
        )
        .route(
            "/api/admin/products/:id/file",
            post(upload_product_file).delete(delete_product_file),
        )
        .route("/api/admin/products/:id/stock", get(product_stock))
        .route(
            "/api/admin/products/:id/items",
            get(list_product_items).post(add_product_items),
        )
        .route(
            "/api/admin/products/:id/items/:item_id",
            delete(delete_product_item_handler),
        )
        .route(
            "/api/admin/products/:id/plans",
            get(list_product_plans).post(add_product_plan),
        )
        .route(
            "/api/admin/products/:id/plans/:plan_id",
            put(update_product_plan).delete(delete_product_plan),
        )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uploaded_image_storage_path_only_allows_upload_filenames() {
        assert_eq!(
            uploaded_image_storage_path("/uploads/product_1.jpg"),
            Some("storage/uploads/product_1.jpg".to_string())
        );
        assert_eq!(uploaded_image_storage_path("/other/product_1.jpg"), None);
        assert_eq!(uploaded_image_storage_path("/uploads/../shop.db"), None);
        assert_eq!(
            uploaded_image_storage_path("/uploads/nested/file.jpg"),
            None
        );
    }
}
