ALTER TABLE orders ADD COLUMN reservation_mode TEXT NOT NULL DEFAULT 'reserved'
    CHECK (reservation_mode IN ('reserved', 'no_reserve'));

CREATE TABLE IF NOT EXISTS order_risk_events (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    event_type TEXT NOT NULL,
    reason TEXT NOT NULL,
    window_started_at TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_order_risk_events_user_created
    ON order_risk_events(user_id, created_at);
