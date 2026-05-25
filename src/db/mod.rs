use anyhow::Result;
use sqlx::sqlite::SqlitePoolOptions;
use sqlx::{SqlitePool, migrate::MigrateDatabase};

pub type DbPool = SqlitePool;

pub async fn init_pool(database_url: &str) -> Result<DbPool> {
    if !sqlx::Sqlite::database_exists(database_url)
        .await
        .unwrap_or(false)
    {
        sqlx::Sqlite::create_database(database_url).await?;
    }

    let pool = SqlitePoolOptions::new()
        .max_connections(5)
        .connect(database_url)
        .await?;

    sqlx::migrate!("./migrations").run(&pool).await?;

    Ok(pool)
}
