CREATE TABLE IF NOT EXISTS order_support_requests (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id TEXT NOT NULL REFERENCES orders(id),
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    username TEXT,
    product_name TEXT NOT NULL,
    bank_memo TEXT NOT NULL,
    amount INTEGER NOT NULL,
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_order_support_requests_created
ON order_support_requests (created_at DESC);

CREATE INDEX IF NOT EXISTS idx_order_support_requests_user
ON order_support_requests (user_id, created_at DESC);
