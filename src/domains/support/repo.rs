use anyhow::Result;
use chrono::Utc;
use sqlx::SqlitePool;

use crate::domains::support::models::{SupportMessage, SupportTicket};

pub struct NewSupportTicket<'a> {
    pub public_key: &'a str,
    pub kind: &'a str,
    pub customer_name: Option<&'a str>,
    pub contact_method: &'a str,
    pub contact_value: Option<&'a str>,
    pub order_ref: Option<&'a str>,
    pub facebook_ref: Option<&'a str>,
    pub message: &'a str,
}

pub async fn create_ticket(pool: &SqlitePool, ticket: NewSupportTicket<'_>) -> Result<SupportTicket> {
    let now = Utc::now().to_rfc3339();
    let mut tx = pool.begin().await?;
    let ticket_id = sqlx::query(
        r#"INSERT INTO support_tickets
            (public_key, kind, status, customer_name, contact_method, contact_value, order_ref, facebook_ref, created_at, updated_at)
           VALUES (?, ?, 'open', ?, ?, ?, ?, ?, ?, ?)"#,
    )
    .bind(ticket.public_key)
    .bind(ticket.kind)
    .bind(ticket.customer_name)
    .bind(ticket.contact_method)
    .bind(ticket.contact_value)
    .bind(ticket.order_ref)
    .bind(ticket.facebook_ref)
    .bind(&now)
    .bind(&now)
    .execute(&mut *tx)
    .await?
    .last_insert_rowid();

    sqlx::query(
        r#"INSERT INTO support_messages (ticket_id, sender, message, created_at)
           VALUES (?, 'customer', ?, ?)"#,
    )
    .bind(ticket_id)
    .bind(ticket.message)
    .bind(&now)
    .execute(&mut *tx)
    .await?;

    tx.commit().await?;
    get_ticket(pool, ticket_id).await?.ok_or_else(|| anyhow::anyhow!("ticket not found after create"))
}

pub async fn get_ticket(pool: &SqlitePool, id: i64) -> Result<Option<SupportTicket>> {
    let ticket = sqlx::query_as::<_, SupportTicket>(
        r#"SELECT id, public_key, kind, status, customer_name, contact_method, contact_value,
                  order_ref, facebook_ref, created_at, updated_at, closed_at
           FROM support_tickets
           WHERE id = ?"#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await?;
    Ok(ticket)
}

pub async fn get_ticket_by_public_key(
    pool: &SqlitePool,
    id: i64,
    public_key: &str,
) -> Result<Option<SupportTicket>> {
    let ticket = sqlx::query_as::<_, SupportTicket>(
        r#"SELECT id, public_key, kind, status, customer_name, contact_method, contact_value,
                  order_ref, facebook_ref, created_at, updated_at, closed_at
           FROM support_tickets
           WHERE id = ? AND public_key = ?"#,
    )
    .bind(id)
    .bind(public_key)
    .fetch_optional(pool)
    .await?;
    Ok(ticket)
}

pub async fn list_messages(pool: &SqlitePool, ticket_id: i64) -> Result<Vec<SupportMessage>> {
    let messages = sqlx::query_as::<_, SupportMessage>(
        r#"SELECT id, ticket_id, sender, message, created_at
           FROM support_messages
           WHERE ticket_id = ?
           ORDER BY id ASC"#,
    )
    .bind(ticket_id)
    .fetch_all(pool)
    .await?;
    Ok(messages)
}

pub async fn add_message(
    pool: &SqlitePool,
    ticket_id: i64,
    sender: &str,
    message: &str,
) -> Result<SupportMessage> {
    let now = Utc::now().to_rfc3339();
    let message_id = sqlx::query(
        r#"INSERT INTO support_messages (ticket_id, sender, message, created_at)
           VALUES (?, ?, ?, ?)"#,
    )
    .bind(ticket_id)
    .bind(sender)
    .bind(message)
    .bind(&now)
    .execute(pool)
    .await?
    .last_insert_rowid();

    sqlx::query(
        r#"UPDATE support_tickets
           SET status = CASE WHEN ? = 'admin' THEN 'answered' ELSE 'open' END,
               updated_at = ?
           WHERE id = ?"#,
    )
    .bind(sender)
    .bind(&now)
    .bind(ticket_id)
    .execute(pool)
    .await?;

    let message = sqlx::query_as::<_, SupportMessage>(
        r#"SELECT id, ticket_id, sender, message, created_at
           FROM support_messages
           WHERE id = ?"#,
    )
    .bind(message_id)
    .fetch_one(pool)
    .await?;
    Ok(message)
}

pub async fn close_ticket(pool: &SqlitePool, ticket_id: i64) -> Result<()> {
    let now = Utc::now().to_rfc3339();
    sqlx::query(
        r#"UPDATE support_tickets
           SET status = 'closed', updated_at = ?, closed_at = ?
           WHERE id = ?"#,
    )
    .bind(&now)
    .bind(&now)
    .bind(ticket_id)
    .execute(pool)
    .await?;
    Ok(())
}
