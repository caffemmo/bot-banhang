use anyhow::{Result, anyhow};
use chrono::{DateTime, Utc};
use sqlx::{Executor, FromRow, QueryBuilder, Sqlite, SqlitePool, Transaction};

use super::models::ProductPlan;
use super::models::{Product, ProductCategory, ProductItem};
use crate::domains::orders::models::{Order, OrderReservationMode, OrderStatus, OrderWithProduct};
use crate::domains::users::models::Subscriber;

#[derive(Debug, FromRow)]
struct OrderJoinRow {
    id: String,
    user_id: i64,
    chat_id: i64,
    product_id: i64,
    qty: i64,
    amount: i64,
    status: String,
    bank_memo: String,
    created_at: String,
    paid_at: Option<String>,
    payment_tx_id: Option<String>,
    delivered_data: Option<String>,
    reserved_item_ids: Option<String>,
    customer_input: Option<String>,
    plan_id: Option<i64>,
    plan_label: Option<String>,
    plan_months: Option<i64>,
    plan_price: Option<i64>,
    reservation_mode: OrderReservationMode,
    p_id: i64,
    p_name: String,
    p_price: i64,
    p_is_active: Option<i64>,
    p_requires_input: Option<i64>,
    p_input_prompt: Option<String>,
    p_description: Option<String>,
    p_image_url: Option<String>,
    p_delivery_type: Option<String>,
    p_file_path: Option<String>,
    p_file_name: Option<String>,
    p_file_mime: Option<String>,
    p_category_id: Option<i64>,
    p_category: Option<String>,
    p_category_emoji: Option<String>,
    p_category_custom_emoji_id: Option<String>,
    p_button_emoji: Option<String>,
    p_button_custom_emoji_id: Option<String>,
    p_created_at: Option<String>,
    p_show_sold_count: Option<i64>,
}

const PRODUCT_SELECT: &str = r#"p.id,
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
        p.show_sold_count"#;

const PRODUCT_FROM: &str =
    " FROM products p LEFT JOIN product_categories pc ON pc.id = p.category_id";

fn apply_product_filters(
    builder: &mut QueryBuilder<'_, Sqlite>,
    active: Option<i64>,
    query: Option<&str>,
) {
    let mut has_where = false;
    if let Some(active) = active {
        builder
            .push(" WHERE IFNULL(p.is_active, 1) = ")
            .push_bind(active);
        has_where = true;
    }

    if let Some(q) = query.filter(|s| !s.trim().is_empty()) {
        let pattern = format!("%{q}%");
        builder
            .push(if has_where { " AND " } else { " WHERE " })
            .push("p.name LIKE ")
            .push_bind(pattern);
    }
}

fn apply_order_filters(
    builder: &mut QueryBuilder<'_, Sqlite>,
    status: Option<OrderStatus>,
    query: Option<String>,
    from: Option<String>,
    to: Option<String>,
) {
    let mut has_where = false;
    if let Some(status) = status {
        builder
            .push(" WHERE o.status = ")
            .push_bind(status.to_string());
        has_where = true;
    }

    if let Some(from) = from {
        builder
            .push(if has_where { " AND " } else { " WHERE " })
            .push("o.created_at >= ")
            .push_bind(from);
        has_where = true;
    }

    if let Some(to) = to {
        builder
            .push(if has_where { " AND " } else { " WHERE " })
            .push("o.created_at <= ")
            .push_bind(to);
        has_where = true;
    }

    if let Some(q) = query.filter(|s| !s.trim().is_empty()) {
        let pattern = format!("%{q}%");
        builder
            .push(if has_where { " AND (" } else { " WHERE (" })
            .push("o.id LIKE ")
            .push_bind(pattern.clone())
            .push(" OR o.bank_memo LIKE ")
            .push_bind(pattern);

        if let Ok(user_id) = q.parse::<i64>() {
            builder.push(" OR o.user_id = ").push_bind(user_id);
        }
        builder.push(")");
    }
}

pub async fn list_products(pool: &SqlitePool, limit: i64, offset: i64) -> Result<Vec<Product>> {
    let sql = format!(
        r#"SELECT {PRODUCT_SELECT}
        {PRODUCT_FROM}
        WHERE IFNULL(p.is_active, 1) = 1
        ORDER BY p.sort_order ASC, p.id ASC
        LIMIT ? OFFSET ?"#
    );
    let products = sqlx::query_as::<sqlx::Sqlite, Product>(&sql)
        .bind(limit)
        .bind(offset)
        .fetch_all(pool)
        .await?;

    Ok(products)
}

pub async fn list_products_by_category(pool: &SqlitePool, category: &str) -> Result<Vec<Product>> {
    let sql = format!(
        r#"SELECT {PRODUCT_SELECT}
        {PRODUCT_FROM}
        WHERE IFNULL(p.is_active, 1) = 1
            AND TRIM(IFNULL(COALESCE(pc.name, p.category), '')) = ?
        ORDER BY p.sort_order ASC, p.id ASC"#
    );
    let products = sqlx::query_as::<sqlx::Sqlite, Product>(&sql)
        .bind(category.trim())
        .fetch_all(pool)
        .await?;

    Ok(products)
}

pub async fn search_products(pool: &SqlitePool, query: &str, limit: i64) -> Result<Vec<Product>> {
    let pattern = format!("%{}%", query.trim());
    let limit = limit.clamp(1, 50);
    let sql = format!(
        r#"SELECT {PRODUCT_SELECT}
        {PRODUCT_FROM}
        WHERE IFNULL(p.is_active, 1) = 1
            AND (
                p.name LIKE ?
                OR IFNULL(p.description, '') LIKE ?
                OR IFNULL(COALESCE(pc.name, p.category), '') LIKE ?
            )
        ORDER BY p.sort_order ASC, p.id ASC
        LIMIT ?"#
    );
    let products = sqlx::query_as::<sqlx::Sqlite, Product>(&sql)
        .bind(pattern.clone())
        .bind(pattern.clone())
        .bind(pattern)
        .bind(limit)
        .fetch_all(pool)
        .await?;

    Ok(products)
}

pub async fn get_product(pool: &SqlitePool, id: i64) -> Result<Option<Product>> {
    let sql = format!(
        r#"SELECT {PRODUCT_SELECT}
        {PRODUCT_FROM}
        WHERE p.id = ?"#
    );
    let product = sqlx::query_as::<sqlx::Sqlite, Product>(&sql)
        .bind(id)
        .fetch_optional(pool)
        .await?;

    Ok(product)
}

pub async fn count_products(pool: &SqlitePool) -> Result<i64> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(1) FROM products WHERE IFNULL(is_active, 1) = 1"#,
    )
    .fetch_one(pool)
    .await?;

    Ok(count)
}

pub async fn list_products_filtered(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    active: Option<i64>,
    query: Option<&str>,
) -> Result<Vec<Product>> {
    let mut builder = QueryBuilder::new(format!("SELECT {PRODUCT_SELECT}{PRODUCT_FROM}"));
    apply_product_filters(&mut builder, active, query);
    builder
        .push(" ORDER BY p.sort_order ASC, p.id ASC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let products = builder.build_query_as::<Product>().fetch_all(pool).await?;

    Ok(products)
}

pub async fn count_products_filtered(
    pool: &SqlitePool,
    active: Option<i64>,
    query: Option<&str>,
) -> Result<i64> {
    let mut builder = QueryBuilder::new(r#"SELECT COUNT(1) FROM products p"#);
    apply_product_filters(&mut builder, active, query);
    let count = builder.build_query_scalar::<i64>().fetch_one(pool).await?;
    Ok(count)
}

pub async fn list_product_categories(pool: &SqlitePool) -> Result<Vec<ProductCategory>> {
    let categories = sqlx::query_as::<_, ProductCategory>(
        r#"SELECT id, name, emoji, custom_emoji_id, sort_order, is_active, created_at
        FROM product_categories
        WHERE IFNULL(is_active, 1) = 1
        ORDER BY sort_order ASC, id ASC"#,
    )
    .fetch_all(pool)
    .await?;

    Ok(categories)
}

pub async fn get_product_category(pool: &SqlitePool, id: i64) -> Result<Option<ProductCategory>> {
    let category = sqlx::query_as::<_, ProductCategory>(
        r#"SELECT id, name, emoji, custom_emoji_id, sort_order, is_active, created_at
        FROM product_categories
        WHERE id = ? AND IFNULL(is_active, 1) = 1"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;

    Ok(category)
}

pub async fn insert_product_category(
    pool: &SqlitePool,
    name: &str,
    emoji: Option<&str>,
    custom_emoji_id: Option<&str>,
    sort_order: Option<i64>,
    is_active: Option<i64>,
) -> Result<ProductCategory> {
    let result = sqlx::query(
        r#"INSERT INTO product_categories (name, emoji, custom_emoji_id, sort_order, is_active)
        VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(name)
    .bind(emoji)
    .bind(custom_emoji_id)
    .bind(sort_order)
    .bind(is_active.unwrap_or(1))
    .execute(pool)
    .await?;

    let id = result.last_insert_rowid();
    if sort_order.is_none() {
        sqlx::query("UPDATE product_categories SET sort_order = ? WHERE id = ?")
            .bind(id)
            .bind(id)
            .execute(pool)
            .await?;
    }

    get_product_category(pool, id)
        .await?
        .ok_or_else(|| anyhow!("insert category failed"))
}

pub async fn update_product_category(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    emoji: Option<&str>,
    custom_emoji_id: Option<&str>,
    sort_order: Option<i64>,
    is_active: Option<i64>,
) -> Result<Option<ProductCategory>> {
    let result = sqlx::query(
        r#"UPDATE product_categories
        SET name = ?, emoji = ?, custom_emoji_id = ?, sort_order = ?, is_active = ?
        WHERE id = ?"#,
    )
    .bind(name)
    .bind(emoji)
    .bind(custom_emoji_id)
    .bind(sort_order)
    .bind(is_active.unwrap_or(1))
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    sqlx::query(
        r#"UPDATE products
        SET category = ?
        WHERE category_id = ?"#,
    )
    .bind(name)
    .bind(id)
    .execute(pool)
    .await?;

    get_product_category(pool, id).await
}

pub async fn set_product_category_active(
    pool: &SqlitePool,
    id: i64,
    is_active: i64,
) -> Result<bool> {
    let result = sqlx::query("UPDATE product_categories SET is_active = ? WHERE id = ?")
        .bind(is_active)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn count_product_items(pool: &SqlitePool, product_id: i64) -> Result<i64> {
    let count = sqlx::query_scalar::<_, i64>(
        r#"SELECT COUNT(1) FROM product_items WHERE product_id = ? AND is_buy = 0"#,
    )
    .bind(product_id)
    .fetch_one(pool)
    .await?;
    Ok(count)
}

#[allow(dead_code)]
pub async fn cancel_order_for_user(
    pool: &SqlitePool,
    order_id: &str,
    user_id: Option<i64>,
) -> Result<bool> {
    let Some(uid) = user_id else {
        return Ok(false);
    };
    let result = sqlx::query(
        r#"UPDATE orders
        SET status = 'cancel'
        WHERE id = ? AND user_id = ? AND status = 'pending'"#,
    )
    .bind(order_id)
    .bind(uid)
    .execute(pool)
    .await?;

    Ok(result.rows_affected() > 0)
}

pub async fn delete_product_item(pool: &SqlitePool, product_id: i64, item_id: i64) -> Result<u64> {
    let result = sqlx::query(r#"DELETE FROM product_items WHERE id = ? AND product_id = ?"#)
        .bind(item_id)
        .bind(product_id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

// -------- Product plans (pricing options) --------
pub async fn list_product_plans(pool: &SqlitePool, product_id: i64) -> Result<Vec<ProductPlan>> {
    let rows = sqlx::query_as::<sqlx::Sqlite, ProductPlan>(
        r#"SELECT id, product_id, label, months, price, sort_order, created_at
        FROM product_plans
        WHERE product_id = ?
        ORDER BY sort_order ASC, id ASC"#,
    )
    .bind(product_id)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn insert_product_plan(
    pool: &SqlitePool,
    product_id: i64,
    label: &str,
    months: i64,
    price: i64,
    sort_order: Option<i64>,
) -> Result<ProductPlan> {
    let result = sqlx::query(
        r#"INSERT INTO product_plans (product_id, label, months, price, sort_order)
        VALUES (?, ?, ?, ?, ?)"#,
    )
    .bind(product_id)
    .bind(label)
    .bind(months)
    .bind(price)
    .bind(sort_order)
    .execute(pool)
    .await?;
    let id = result.last_insert_rowid();
    get_product_plan(pool, id)
        .await?
        .ok_or_else(|| anyhow!("insert plan failed"))
}

pub async fn update_product_plan(
    pool: &SqlitePool,
    id: i64,
    label: &str,
    months: i64,
    price: i64,
    sort_order: Option<i64>,
) -> Result<Option<ProductPlan>> {
    let result = sqlx::query(
        r#"UPDATE product_plans
        SET label = ?, months = ?, price = ?, sort_order = ?
        WHERE id = ?"#,
    )
    .bind(label)
    .bind(months)
    .bind(price)
    .bind(sort_order)
    .bind(id)
    .execute(pool)
    .await?;
    if result.rows_affected() == 0 {
        return Ok(None);
    }
    get_product_plan(pool, id).await
}

pub async fn delete_product_plan(pool: &SqlitePool, id: i64) -> Result<u64> {
    let result = sqlx::query(r#"DELETE FROM product_plans WHERE id = ?"#)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(result.rows_affected())
}

pub async fn get_product_plan(pool: &SqlitePool, id: i64) -> Result<Option<ProductPlan>> {
    let row = sqlx::query_as::<sqlx::Sqlite, ProductPlan>(
        r#"SELECT id, product_id, label, months, price, sort_order, created_at
        FROM product_plans
        WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(row)
}

// ---- Subscribers ----
pub async fn upsert_subscriber(pool: &SqlitePool, profile: &Subscriber) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO subscribers (
            user_id, chat_id, username, first_name, last_name, full_name, language_code, preferred_language, stock_notifications_enabled, is_bot
        ) VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?)
        ON CONFLICT(user_id) DO UPDATE SET
            chat_id = excluded.chat_id,
            username = excluded.username,
            first_name = excluded.first_name,
            last_name = excluded.last_name,
            full_name = excluded.full_name,
            language_code = excluded.language_code,
            preferred_language = coalesce(subscribers.preferred_language, excluded.preferred_language),
            stock_notifications_enabled = coalesce(subscribers.stock_notifications_enabled, excluded.stock_notifications_enabled),
            is_bot = excluded.is_bot,
            updated_at = datetime('now')"#,
    )
    .bind(profile.user_id)
    .bind(profile.chat_id)
    .bind(&profile.username)
    .bind(&profile.first_name)
    .bind(&profile.last_name)
    .bind(&profile.full_name)
    .bind(&profile.language_code)
    .bind(&profile.preferred_language)
    .bind(profile.stock_notifications_enabled.unwrap_or(1))
    .bind(profile.is_bot)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn list_subscribers(pool: &SqlitePool) -> Result<Vec<Subscriber>> {
    let subs = sqlx::query_as::<sqlx::Sqlite, Subscriber>(
        r#"SELECT user_id, chat_id, username, first_name, last_name, full_name, language_code, preferred_language, stock_notifications_enabled, is_bot, created_at, updated_at
        FROM subscribers
        ORDER BY created_at DESC"#,
    )
    .fetch_all(pool)
    .await?;
    Ok(subs)
}

pub async fn insert_product_items(
    pool: &SqlitePool,
    product_id: i64,
    items: &[String],
) -> Result<usize> {
    if items.is_empty() {
        return Ok(0);
    }

    let mut sql = String::from("INSERT INTO product_items (product_id, content) VALUES ");
    let mut first = true;
    for _ in items {
        if !first {
            sql.push_str(", ");
        }
        sql.push_str("(?, ?)");
        first = false;
    }

    let mut query = sqlx::query(&sql);
    for item in items {
        query = query.bind(product_id).bind(item);
    }

    let result = query.execute(pool).await?;
    Ok(result.rows_affected() as usize)
}

pub async fn list_product_items(
    pool: &SqlitePool,
    product_id: i64,
    limit: i64,
    offset: i64,
) -> Result<Vec<ProductItem>> {
    let items = sqlx::query_as::<sqlx::Sqlite, ProductItem>(
        r#"SELECT id, product_id, content, created_at, is_buy
        FROM product_items
        WHERE product_id = ? AND is_buy = 0
        ORDER BY id
        LIMIT ? OFFSET ?"#,
    )
    .bind(product_id)
    .bind(limit)
    .bind(offset)
    .fetch_all(pool)
    .await?;

    Ok(items)
}

pub async fn take_product_items(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: i64,
    qty: i64,
) -> Result<Vec<ProductItem>> {
    if qty <= 0 {
        return Ok(Vec::new());
    }

    let conn = tx.as_mut();
    let items = sqlx::query_as::<sqlx::Sqlite, ProductItem>(
        r#"SELECT id, product_id, content, created_at, is_buy
        FROM product_items
        WHERE product_id = ? AND is_buy = 0
        ORDER BY id
        LIMIT ?"#,
    )
    .bind(product_id)
    .bind(qty)
    .fetch_all(&mut *conn)
    .await?;

    if items.len() < qty as usize {
        return Err(anyhow!(
            "not enough product items (need {qty}, have {})",
            items.len()
        ));
    }

    // Mark items as bought.
    let mut builder = QueryBuilder::new("UPDATE product_items SET is_buy = 1 WHERE id IN (");
    let mut separated = builder.separated(", ");
    for item in &items {
        separated.push_bind(item.id);
    }
    builder.push(")");
    builder.build().execute(&mut *conn).await?;

    Ok(items)
}

pub async fn return_product_items(
    tx: &mut Transaction<'_, Sqlite>,
    product_id: i64,
    item_ids: &[i64],
) -> Result<usize> {
    if item_ids.is_empty() {
        return Ok(0);
    }
    let mut qb = QueryBuilder::new("UPDATE product_items SET is_buy = 0 WHERE product_id = ");
    qb.push_bind(product_id);
    qb.push(" AND id IN (");
    let mut separated = qb.separated(", ");
    for id in item_ids {
        separated.push_bind(id);
    }
    qb.push(")");
    let result = qb.build().execute(tx.as_mut()).await?;
    Ok(result.rows_affected() as usize)
}

#[allow(clippy::too_many_arguments)]
pub async fn insert_product(
    pool: &SqlitePool,
    name: &str,
    price: i64,
    is_active: Option<i64>,
    requires_input: Option<i64>,
    input_prompt: Option<&str>,
    description: Option<&str>,
    image_url: Option<&str>,
    delivery_type: Option<&str>,
    file_path: Option<&str>,
    file_name: Option<&str>,
    file_mime: Option<&str>,
    category_id: Option<i64>,
    category: Option<&str>,
    button_emoji: Option<&str>,
    button_custom_emoji_id: Option<&str>,
) -> Result<Product> {
    let result = sqlx::query(
        r#"INSERT INTO products (name, price, is_active, requires_input, input_prompt, description, image_url, delivery_type, file_path, file_name, file_mime, category_id, category, button_emoji, button_custom_emoji_id) 
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(name)
    .bind(price)
    .bind(is_active.unwrap_or(1))
    .bind(requires_input.unwrap_or(0))
    .bind(input_prompt)
    .bind(description)
    .bind(image_url)
    .bind(delivery_type.unwrap_or("stock_item"))
    .bind(file_path)
    .bind(file_name)
    .bind(file_mime)
    .bind(category_id)
    .bind(category)
    .bind(button_emoji)
    .bind(button_custom_emoji_id)
    .execute(pool)
    .await?;

    let id = result.last_insert_rowid();
    let first_sort_order =
        sqlx::query_scalar::<_, Option<i64>>("SELECT MIN(sort_order) FROM products WHERE id != ?")
            .bind(id)
            .fetch_one(pool)
            .await?
            .flatten()
            .unwrap_or(1);
    sqlx::query("UPDATE products SET sort_order = ? WHERE id = ?")
        .bind(first_sort_order - 1)
        .bind(id)
        .execute(pool)
        .await?;

    get_product(pool, id)
        .await?
        .ok_or_else(|| anyhow!("insert failed"))
}

#[allow(clippy::too_many_arguments)]
pub async fn update_product(
    pool: &SqlitePool,
    id: i64,
    name: &str,
    price: i64,
    is_active: Option<i64>,
    requires_input: Option<i64>,
    input_prompt: Option<&str>,
    description: Option<&str>,
    image_url: Option<&str>,
    delivery_type: Option<&str>,
    file_path: Option<&str>,
    file_name: Option<&str>,
    file_mime: Option<&str>,
    category_id: Option<i64>,
    category: Option<&str>,
    button_emoji: Option<&str>,
    button_custom_emoji_id: Option<&str>,
) -> Result<Option<Product>> {
    let result = sqlx::query(
        r#"UPDATE products 
        SET name = ?, price = ?, is_active = ?, requires_input = ?, input_prompt = ?, description = ?, image_url = ?, delivery_type = ?, file_path = ?, file_name = ?, file_mime = ?, category_id = ?, category = ?, button_emoji = ?, button_custom_emoji_id = ?
        WHERE id = ?"#,
    )
    .bind(name)
    .bind(price)
    .bind(is_active.unwrap_or(1))
    .bind(requires_input.unwrap_or(0))
    .bind(input_prompt)
    .bind(description)
    .bind(image_url)
    .bind(delivery_type.unwrap_or("stock_item"))
    .bind(file_path)
    .bind(file_name)
    .bind(file_mime)
    .bind(category_id)
    .bind(category)
    .bind(button_emoji)
    .bind(button_custom_emoji_id)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    let product = get_product(pool, id).await?;
    Ok(product)
}

pub async fn update_product_show_sold_count(
    pool: &SqlitePool,
    id: i64,
    show_sold_count: i64,
) -> Result<Option<Product>> {
    let result = sqlx::query(r#"UPDATE products SET show_sold_count = ? WHERE id = ?"#)
        .bind(show_sold_count)
        .bind(id)
        .execute(pool)
        .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    get_product(pool, id).await
}

pub async fn count_product_paid_quantity_sold(pool: &SqlitePool, product_id: i64) -> Result<i64> {
    let sold = sqlx::query_scalar::<_, Option<i64>>(
        r#"SELECT COALESCE(SUM(qty), 0)
        FROM orders
        WHERE product_id = ? AND status = 'paid'"#,
    )
    .bind(product_id)
    .fetch_one(pool)
    .await?;

    Ok(sold.unwrap_or(0))
}

pub async fn update_product_file_metadata(
    pool: &SqlitePool,
    id: i64,
    file_path: Option<&str>,
    file_name: Option<&str>,
    file_mime: Option<&str>,
) -> Result<Option<Product>> {
    let result = sqlx::query(
        r#"UPDATE products
        SET file_path = ?, file_name = ?, file_mime = ?
        WHERE id = ?"#,
    )
    .bind(file_path)
    .bind(file_name)
    .bind(file_mime)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    get_product(pool, id).await
}

pub async fn update_product_image_url(
    pool: &SqlitePool,
    id: i64,
    image_url: Option<&str>,
) -> Result<Option<Product>> {
    let result = sqlx::query(
        r#"UPDATE products
        SET image_url = ?
        WHERE id = ?"#,
    )
    .bind(image_url)
    .bind(id)
    .execute(pool)
    .await?;

    if result.rows_affected() == 0 {
        return Ok(None);
    }

    get_product(pool, id).await
}

pub async fn set_product_active(pool: &SqlitePool, id: i64, is_active: i64) -> Result<bool> {
    let result = sqlx::query(r#"UPDATE products SET is_active = ? WHERE id = ?"#)
        .bind(is_active)
        .bind(id)
        .execute(pool)
        .await?;

    Ok(result.rows_affected() > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use sqlx::sqlite::SqlitePoolOptions;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    #[tokio::test]
    async fn products_can_use_managed_category_with_custom_emoji() {
        let pool = test_pool().await;
        let category = insert_product_category(
            &pool,
            "CAP CUT",
            Some("🎬"),
            Some("5375135722514685501"),
            Some(1),
            Some(1),
        )
        .await
        .unwrap();

        let product = insert_product(
            &pool,
            "Vip",
            5_000,
            Some(1),
            Some(0),
            None,
            None,
            None,
            Some("stock_item"),
            None,
            None,
            None,
            Some(category.id),
            Some(&category.name),
            None,
            None,
        )
        .await
        .unwrap();

        assert_eq!(product.category_id, Some(category.id));
        assert_eq!(product.category.as_deref(), Some("CAP CUT"));
        assert_eq!(product.category_emoji.as_deref(), Some("🎬"));
        assert_eq!(
            product.category_custom_emoji_id.as_deref(),
            Some("5375135722514685501")
        );

        let listed = list_products_by_category(&pool, "CAP CUT").await.unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].id, product.id);

        update_product_category(
            &pool,
            category.id,
            "CAPCUT",
            Some("🎞️"),
            None,
            Some(2),
            Some(1),
        )
        .await
        .unwrap()
        .unwrap();

        let updated = get_product(&pool, product.id).await.unwrap().unwrap();
        assert_eq!(updated.category.as_deref(), Some("CAPCUT"));
        assert_eq!(updated.category_emoji.as_deref(), Some("🎞️"));
    }

    #[tokio::test]
    async fn update_product_image_url_can_clear_existing_image() {
        let pool = test_pool().await;
        let product = insert_product(
            &pool,
            "Product with image",
            10_000,
            Some(1),
            Some(0),
            None,
            None,
            Some("/uploads/product_1.jpg"),
            Some("stock_item"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        let updated = update_product_image_url(&pool, product.id, None)
            .await
            .unwrap()
            .unwrap();

        assert_eq!(updated.id, product.id);
        assert_eq!(updated.image_url, None);
    }

    #[tokio::test]
    async fn list_product_items_returns_available_items_only() {
        let pool = test_pool().await;
        let product = insert_product(
            &pool,
            "Stock product",
            10_000,
            Some(1),
            Some(0),
            None,
            None,
            None,
            Some("stock_item"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();
        insert_product_items(
            &pool,
            product.id,
            &["sold".to_string(), "available".to_string()],
        )
        .await
        .unwrap();

        let mut tx = pool.begin().await.unwrap();
        let sold_items = take_product_items(&mut tx, product.id, 1).await.unwrap();
        tx.commit().await.unwrap();

        let available = list_product_items(&pool, product.id, 20, 0).await.unwrap();

        assert_eq!(sold_items[0].content, "sold");
        assert_eq!(available.len(), 1);
        assert_eq!(available[0].content, "available");
        assert_eq!(available[0].is_buy, Some(0));
    }

    #[tokio::test]
    async fn count_product_paid_quantity_sold_sums_paid_orders_only() {
        let pool = test_pool().await;
        let product = insert_product(
            &pool,
            "Sold count",
            5_000,
            Some(1),
            Some(0),
            None,
            None,
            None,
            Some("stock_item"),
            None,
            None,
            None,
            None,
            None,
            None,
            None,
        )
        .await
        .unwrap();

        for (idx, status, qty) in [
            (1, OrderStatus::Paid, 2),
            (2, OrderStatus::Paid, 3),
            (3, OrderStatus::Pending, 9),
            (4, OrderStatus::Refunded, 4),
            (5, OrderStatus::Cancel, 7),
        ] {
            let mut order = Order::new(
                1,
                1,
                product.id,
                qty,
                qty * product.price,
                format!("MEMO{idx}"),
                None,
                None,
                None,
                None,
                None,
            );
            order.status = status;
            insert_order(&pool, &order).await.unwrap();
        }

        assert_eq!(
            count_product_paid_quantity_sold(&pool, product.id)
                .await
                .unwrap(),
            5
        );
    }
}

#[allow(dead_code)]
pub async fn insert_order(pool: &SqlitePool, order: &Order) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO orders
        (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&order.id)
    .bind(order.user_id)
    .bind(order.chat_id)
    .bind(order.product_id)
    .bind(order.qty)
    .bind(order.amount)
    .bind(order.status.to_string())
    .bind(&order.bank_memo)
    .bind(&order.created_at)
    .bind(&order.paid_at)
    .bind(&order.payment_tx_id)
    .bind(&order.delivered_data)
    .bind(&order.reserved_item_ids)
    .bind(&order.customer_input)
    .bind(order.plan_id)
    .bind(&order.plan_label)
    .bind(order.plan_months)
    .bind(order.plan_price)
    .bind(order.reservation_mode.to_string())
    .execute(pool)
    .await?;

    Ok(())
}

pub async fn insert_order_tx(tx: &mut Transaction<'_, Sqlite>, order: &Order) -> Result<()> {
    sqlx::query(
        r#"INSERT INTO orders
        (id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode)
        VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(&order.id)
    .bind(order.user_id)
    .bind(order.chat_id)
    .bind(order.product_id)
    .bind(order.qty)
    .bind(order.amount)
    .bind(order.status.to_string())
    .bind(&order.bank_memo)
    .bind(&order.created_at)
    .bind(&order.paid_at)
    .bind(&order.payment_tx_id)
    .bind(&order.delivered_data)
    .bind(&order.reserved_item_ids)
    .bind(&order.customer_input)
    .bind(order.plan_id)
    .bind(&order.plan_label)
    .bind(order.plan_months)
    .bind(order.plan_price)
    .bind(order.reservation_mode.to_string())
    .execute(tx.as_mut())
    .await?;

    Ok(())
}

pub async fn get_order(pool: &SqlitePool, order_id: &str) -> Result<Option<Order>> {
    let order = sqlx::query_as::<sqlx::Sqlite, Order>(
        r#"SELECT id, user_id, chat_id, product_id, qty, amount, status, bank_memo, created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids, customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode
        FROM orders
        WHERE id = ?"#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?;

    Ok(order)
}

pub async fn get_order_with_product(
    pool: &SqlitePool,
    order_id: &str,
) -> Result<Option<OrderWithProduct>> {
    let row = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category_id as p_category_id,
            p.category as p_category,
            NULL as p_category_emoji,
            NULL as p_category_custom_emoji_id,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at,
            p.show_sold_count as p_show_sold_count
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.id = ?"#,
    )
    .bind(order_id)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_join_row))
}

pub async fn list_orders_admin(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    status: Option<OrderStatus>,
    query: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<Vec<OrderWithProduct>> {
    let mut builder = QueryBuilder::new(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category_id as p_category_id,
            p.category as p_category,
            NULL as p_category_emoji,
            NULL as p_category_custom_emoji_id,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at,
            p.show_sold_count as p_show_sold_count
        FROM orders o
        JOIN products p ON p.id = o.product_id"#,
    );
    apply_order_filters(
        &mut builder,
        status,
        query.map(|s| s.to_string()),
        from.map(|s| s.to_string()),
        to.map(|s| s.to_string()),
    );
    builder
        .push(" ORDER BY o.created_at DESC LIMIT ")
        .push_bind(limit)
        .push(" OFFSET ")
        .push_bind(offset);

    let rows = builder
        .build_query_as::<OrderJoinRow>()
        .fetch_all(pool)
        .await?;
    Ok(rows.into_iter().map(map_join_row).collect())
}

pub async fn sum_paid_between(pool: &SqlitePool, from: &str, to: &str) -> Result<i64> {
    let total = sqlx::query_scalar::<_, i64>(
        r#"SELECT IFNULL(SUM(amount),0) 
           FROM orders 
           WHERE status = 'paid' AND created_at >= ? AND created_at <= ?"#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

pub async fn count_orders_admin(
    pool: &SqlitePool,
    status: Option<OrderStatus>,
    query: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<i64> {
    let mut builder = QueryBuilder::new(
        r#"SELECT COUNT(1) FROM orders o JOIN products p ON p.id = o.product_id"#,
    );
    apply_order_filters(
        &mut builder,
        status,
        query.map(|s| s.to_string()),
        from.map(|s| s.to_string()),
        to.map(|s| s.to_string()),
    );
    let count = builder.build_query_scalar::<i64>().fetch_one(pool).await?;
    Ok(count)
}

pub async fn list_orders_for_user(
    pool: &SqlitePool,
    user_id: i64,
    limit: i64,
) -> Result<Vec<OrderWithProduct>> {
    let rows = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category_id as p_category_id,
            p.category as p_category,
            NULL as p_category_emoji,
            NULL as p_category_custom_emoji_id,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at,
            p.show_sold_count as p_show_sold_count
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.user_id = ?
        ORDER BY o.created_at DESC
        LIMIT ?"#,
    )
    .bind(user_id)
    .bind(limit)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_join_row).collect())
}

pub async fn find_order_by_memo(pool: &SqlitePool, memo: &str) -> Result<Option<OrderWithProduct>> {
    let row = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category_id as p_category_id,
            p.category as p_category,
            NULL as p_category_emoji,
            NULL as p_category_custom_emoji_id,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at,
            p.show_sold_count as p_show_sold_count
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.bank_memo = ?"#,
    )
    .bind(memo)
    .fetch_optional(pool)
    .await?;

    Ok(row.map(map_join_row))
}

pub async fn list_pending_before(pool: &SqlitePool, before: &str) -> Result<Vec<OrderWithProduct>> {
    let rows = sqlx::query_as::<sqlx::Sqlite, OrderJoinRow>(
        r#"SELECT 
            o.id,
            o.user_id,
            o.chat_id,
            o.product_id,
            o.qty,
            o.amount,
            o.status,
            o.bank_memo,
            o.created_at,
            o.paid_at,
            o.payment_tx_id,
            o.delivered_data,
            o.reserved_item_ids,
            o.customer_input,
            o.plan_id,
            o.plan_label,
            o.plan_months,
            o.plan_price,
            o.reservation_mode,
            p.id as p_id,
            p.name as p_name,
            p.price as p_price,
            p.is_active as p_is_active,
            p.requires_input as p_requires_input,
            p.input_prompt as p_input_prompt,
            p.description as p_description,
            p.image_url as p_image_url,
            p.delivery_type as p_delivery_type,
            p.file_path as p_file_path,
            p.file_name as p_file_name,
            p.file_mime as p_file_mime,
            p.category_id as p_category_id,
            p.category as p_category,
            NULL as p_category_emoji,
            NULL as p_category_custom_emoji_id,
            p.button_emoji as p_button_emoji,
            p.button_custom_emoji_id as p_button_custom_emoji_id,
            p.created_at as p_created_at,
            p.show_sold_count as p_show_sold_count
        FROM orders o
        JOIN products p ON p.id = o.product_id
        WHERE o.status = 'pending' AND o.created_at <= ?
        ORDER BY o.created_at ASC"#,
    )
    .bind(before)
    .fetch_all(pool)
    .await?;

    Ok(rows.into_iter().map(map_join_row).collect())
}

pub async fn mark_order_paid(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &str,
    tx_id: &str,
    paid_at: DateTime<Utc>,
    delivered_data: Option<&str>,
    reserved_item_ids: Option<&str>,
) -> Result<()> {
    let result = tx
        .execute(
            sqlx::query(
                r#"UPDATE orders 
            SET status = 'paid', payment_tx_id = ?, paid_at = ?, delivered_data = ?, reserved_item_ids = ?
            WHERE id = ? AND status = 'pending'"#,
            )
            .bind(tx_id)
            .bind(paid_at.to_rfc3339())
            .bind(delivered_data)
            .bind(reserved_item_ids)
            .bind(order_id),
        )
        .await?;

    if result.rows_affected() == 0 {
        return Err(anyhow!("order not found or not pending"));
    }

    Ok(())
}

#[allow(dead_code)]
pub async fn update_order_status(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &str,
    status: OrderStatus,
) -> Result<()> {
    let result = tx
        .execute(
            sqlx::query(
                r#"UPDATE orders 
                SET status = ?
                WHERE id = ?"#,
            )
            .bind(status.to_string())
            .bind(order_id),
        )
        .await?;

    if result.rows_affected() == 0 {
        return Err(anyhow!("order not found"));
    }

    Ok(())
}

pub async fn update_order_status_with_data(
    tx: &mut Transaction<'_, Sqlite>,
    order_id: &str,
    status: OrderStatus,
    delivered_data: Option<&str>,
    reserved_item_ids: Option<&str>,
) -> Result<()> {
    let result = tx
        .execute(
            sqlx::query(
                r#"UPDATE orders 
                SET status = ?, delivered_data = ?, reserved_item_ids = ?
                WHERE id = ?"#,
            )
            .bind(status.to_string())
            .bind(delivered_data)
            .bind(reserved_item_ids)
            .bind(order_id),
        )
        .await?;

    if result.rows_affected() == 0 {
        return Err(anyhow!("order not found"));
    }

    Ok(())
}

fn map_join_row(row: OrderJoinRow) -> OrderWithProduct {
    OrderWithProduct {
        order: Order {
            id: row.id,
            user_id: row.user_id,
            chat_id: row.chat_id,
            product_id: row.product_id,
            qty: row.qty,
            amount: row.amount,
            status: OrderStatus::from_str(&row.status),
            bank_memo: row.bank_memo,
            created_at: row.created_at,
            paid_at: row.paid_at,
            payment_tx_id: row.payment_tx_id,
            delivered_data: row.delivered_data,
            reserved_item_ids: row.reserved_item_ids,
            customer_input: row.customer_input,
            plan_id: row.plan_id,
            plan_label: row.plan_label,
            plan_months: row.plan_months,
            plan_price: row.plan_price,
            reservation_mode: row.reservation_mode,
        },
        product: Product {
            id: row.p_id,
            name: row.p_name,
            price: row.p_price,
            is_active: row.p_is_active,
            requires_input: row.p_requires_input,
            input_prompt: row.p_input_prompt,
            description: row.p_description,
            image_url: row.p_image_url,
            delivery_type: row.p_delivery_type,
            file_path: row.p_file_path,
            file_name: row.p_file_name,
            file_mime: row.p_file_mime,
            category_id: row.p_category_id,
            category: row.p_category,
            category_emoji: row.p_category_emoji,
            category_custom_emoji_id: row.p_category_custom_emoji_id,
            button_emoji: row.p_button_emoji,
            button_custom_emoji_id: row.p_button_custom_emoji_id,
            created_at: row.p_created_at,
            sort_order: None,
            show_sold_count: row.p_show_sold_count,
        },
    }
}

// --- Webhook audit log ---

#[allow(clippy::too_many_arguments)]
pub async fn insert_webhook_event(
    pool: &SqlitePool,
    provider: &str,
    authorized: bool,
    source_ip: Option<&str>,
    memo_extracted: Option<&str>,
    tx_id: Option<&str>,
    amount: Option<i64>,
    status: Option<&str>,
    matched_order_id: Option<&str>,
    result: Option<&str>,
    error: Option<&str>,
    raw_json: Option<&str>,
) -> Result<i64> {
    let rec = sqlx::query(
        r#"INSERT INTO webhook_events (
            provider, authorized, source_ip, memo_extracted, tx_id, amount, status,
            matched_order_id, result, error, raw_json
        ) VALUES (?,?,?,?,?,?,?,?,?,?,?)"#,
    )
    .bind(provider)
    .bind(if authorized { 1 } else { 0 })
    .bind(source_ip)
    .bind(memo_extracted)
    .bind(tx_id)
    .bind(amount)
    .bind(status)
    .bind(matched_order_id)
    .bind(result)
    .bind(error)
    .bind(raw_json)
    .execute(pool)
    .await?;

    Ok(rec.last_insert_rowid())
}

#[derive(sqlx::FromRow, serde::Serialize)]
pub struct WebhookEventListRow {
    pub id: i64,
    pub received_at: String,
    pub provider: String,
    pub authorized: i64,
    pub source_ip: Option<String>,
    pub memo_extracted: Option<String>,
    pub tx_id: Option<String>,
    pub amount: Option<i64>,
    pub status: Option<String>,
    pub matched_order_id: Option<String>,
    pub result: Option<String>,
    pub error: Option<String>,
}

pub async fn list_webhook_events(
    pool: &SqlitePool,
    limit: i64,
    offset: i64,
    provider: Option<&str>,
    memo: Option<&str>,
    tx_id: Option<&str>,
) -> Result<Vec<WebhookEventListRow>> {
    let mut sql = String::from(
        "SELECT id, received_at, provider, authorized, source_ip, memo_extracted, tx_id, amount, status, matched_order_id, result, error\n         FROM webhook_events WHERE 1=1",
    );

    if provider.is_some() {
        sql.push_str(" AND provider = ?");
    }
    if memo.is_some() {
        sql.push_str(" AND memo_extracted LIKE ?");
    }
    if tx_id.is_some() {
        sql.push_str(" AND tx_id LIKE ?");
    }

    sql.push_str(" ORDER BY received_at DESC LIMIT ? OFFSET ?");

    let mut q = sqlx::query_as::<sqlx::Sqlite, WebhookEventListRow>(&sql);

    if let Some(p) = provider {
        q = q.bind(p);
    }
    if let Some(m) = memo {
        q = q.bind(format!("%{}%", m));
    }
    if let Some(t) = tx_id {
        q = q.bind(format!("%{}%", t));
    }

    q = q.bind(limit).bind(offset);

    Ok(q.fetch_all(pool).await?)
}

pub async fn count_webhook_events(
    pool: &SqlitePool,
    provider: Option<&str>,
    memo: Option<&str>,
    tx_id: Option<&str>,
) -> Result<i64> {
    let mut sql = String::from("SELECT COUNT(1) as cnt FROM webhook_events WHERE 1=1");
    if provider.is_some() {
        sql.push_str(" AND provider = ?");
    }
    if memo.is_some() {
        sql.push_str(" AND memo_extracted LIKE ?");
    }
    if tx_id.is_some() {
        sql.push_str(" AND tx_id LIKE ?");
    }

    let mut q = sqlx::query_scalar::<_, i64>(&sql);

    if let Some(p) = provider {
        q = q.bind(p);
    }
    if let Some(m) = memo {
        q = q.bind(format!("%{}%", m));
    }
    if let Some(t) = tx_id {
        q = q.bind(format!("%{}%", t));
    }

    Ok(q.fetch_one(pool).await?)
}
