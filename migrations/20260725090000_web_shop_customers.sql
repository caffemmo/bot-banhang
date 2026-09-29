CREATE TABLE IF NOT EXISTS web_customers (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    username TEXT NOT NULL COLLATE NOCASE UNIQUE,
    password_hash TEXT NOT NULL,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    last_login_at TEXT
);

CREATE INDEX IF NOT EXISTS idx_web_customers_active ON web_customers(is_active);

ALTER TABLE orders ADD COLUMN web_customer_id INTEGER REFERENCES web_customers(id);

CREATE INDEX IF NOT EXISTS idx_orders_web_customer_created
    ON orders(web_customer_id, created_at);
