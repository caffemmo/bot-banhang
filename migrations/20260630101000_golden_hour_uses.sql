CREATE TABLE IF NOT EXISTS sale_hunt_golden_hour_uses (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deal_id INTEGER NOT NULL,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    order_id TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(deal_id, user_id)
);

CREATE INDEX IF NOT EXISTS idx_sale_hunt_golden_hour_uses_user
    ON sale_hunt_golden_hour_uses (user_id, deal_id);
