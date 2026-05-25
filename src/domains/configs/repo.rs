use anyhow::Result;
use sqlx::{Row, SqlitePool};
use std::collections::HashMap;

pub async fn get_all_configs(pool: &SqlitePool) -> Result<HashMap<String, String>> {
    let records = sqlx::query("SELECT key, value FROM app_configs")
        .fetch_all(pool)
        .await?;

    let mut map = HashMap::new();
    for r in records {
        let key: String = r.get("key");
        let value: String = r.get("value");
        map.insert(key, value);
    }

    Ok(map)
}

pub async fn save_configs(pool: &SqlitePool, configs: &HashMap<String, String>) -> Result<()> {
    let mut tx = pool.begin().await?;

    for (k, v) in configs {
        sqlx::query(
            r#"
            INSERT INTO app_configs (key, value)
            VALUES (?, ?)
            ON CONFLICT(key) DO UPDATE SET value = excluded.value
            "#,
        )
        .bind(k)
        .bind(v)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}
