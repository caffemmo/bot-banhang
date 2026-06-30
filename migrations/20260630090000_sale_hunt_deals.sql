CREATE TABLE IF NOT EXISTS sale_hunt_deals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    code TEXT NOT NULL UNIQUE,
    discount_percent INTEGER NOT NULL,
    status TEXT NOT NULL DEFAULT 'active',
    expires_at TEXT NOT NULL,
    order_id TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    used_at TEXT,
    CONSTRAINT sale_hunt_status_check CHECK (status IN ('active', 'used', 'expired'))
);

CREATE INDEX IF NOT EXISTS idx_sale_hunt_deals_user_status
    ON sale_hunt_deals (user_id, status, expires_at);

CREATE INDEX IF NOT EXISTS idx_sale_hunt_deals_user_created
    ON sale_hunt_deals (user_id, created_at);
