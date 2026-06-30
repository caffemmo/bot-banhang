CREATE TABLE IF NOT EXISTS sale_hunt_golden_hour_deals (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    deal_date TEXT NOT NULL UNIQUE,
    starts_at TEXT NOT NULL,
    ends_at TEXT NOT NULL,
    notify_at TEXT NOT NULL,
    discount_percent INTEGER NOT NULL,
    notified_at TEXT,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_sale_hunt_golden_hour_notify
    ON sale_hunt_golden_hour_deals (notify_at, notified_at);

CREATE INDEX IF NOT EXISTS idx_sale_hunt_golden_hour_window
    ON sale_hunt_golden_hour_deals (starts_at, ends_at);
