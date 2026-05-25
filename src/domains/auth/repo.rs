use crate::db::DbPool;
use crate::domains::auth::models::AdminUser;

pub async fn count_admin_users(pool: &DbPool) -> sqlx::Result<i64> {
    sqlx::query_scalar("SELECT COUNT(*) FROM admin_users")
        .fetch_one(pool)
        .await
}

pub async fn list_admin_users(pool: &DbPool) -> sqlx::Result<Vec<AdminUser>> {
    sqlx::query_as::<_, AdminUser>(
        r#"
        SELECT id, username, password_hash, is_active, created_at, updated_at, last_login_at
        FROM admin_users
        ORDER BY id ASC
        "#,
    )
    .fetch_all(pool)
    .await
}

pub async fn get_admin_user(pool: &DbPool, id: i64) -> sqlx::Result<Option<AdminUser>> {
    sqlx::query_as::<_, AdminUser>(
        r#"
        SELECT id, username, password_hash, is_active, created_at, updated_at, last_login_at
        FROM admin_users
        WHERE id = ?
        "#,
    )
    .bind(id)
    .fetch_optional(pool)
    .await
}

pub async fn get_admin_by_username(
    pool: &DbPool,
    username: &str,
) -> sqlx::Result<Option<AdminUser>> {
    sqlx::query_as::<_, AdminUser>(
        r#"
        SELECT id, username, password_hash, is_active, created_at, updated_at, last_login_at
        FROM admin_users
        WHERE username = ?
        "#,
    )
    .bind(username)
    .fetch_optional(pool)
    .await
}

pub async fn insert_admin_user(
    pool: &DbPool,
    username: &str,
    password_hash: &str,
) -> sqlx::Result<AdminUser> {
    sqlx::query(
        r#"
        INSERT INTO admin_users (username, password_hash, is_active, created_at, updated_at)
        VALUES (?, ?, 1, datetime('now'), datetime('now'))
        "#,
    )
    .bind(username)
    .bind(password_hash)
    .execute(pool)
    .await?;

    let id: i64 = sqlx::query_scalar("SELECT last_insert_rowid()")
        .fetch_one(pool)
        .await?;
    get_admin_user(pool, id)
        .await?
        .ok_or(sqlx::Error::RowNotFound)
}

pub async fn update_password_hash(
    pool: &DbPool,
    id: i64,
    password_hash: &str,
) -> sqlx::Result<Option<AdminUser>> {
    let changed = sqlx::query(
        r#"
        UPDATE admin_users
        SET password_hash = ?, updated_at = datetime('now')
        WHERE id = ?
        "#,
    )
    .bind(password_hash)
    .bind(id)
    .execute(pool)
    .await?
    .rows_affected();

    if changed == 0 {
        return Ok(None);
    }

    get_admin_user(pool, id).await
}

pub async fn touch_last_login(pool: &DbPool, id: i64) -> sqlx::Result<()> {
    sqlx::query(
        r#"
        UPDATE admin_users
        SET last_login_at = datetime('now'), updated_at = datetime('now')
        WHERE id = ?
        "#,
    )
    .bind(id)
    .execute(pool)
    .await?;
    Ok(())
}
