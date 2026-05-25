-- SQLite schema for products and orders
CREATE TABLE IF NOT EXISTS products (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL,
    price INTEGER NOT NULL,
    is_active INTEGER DEFAULT 1,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS orders (
    id TEXT PRIMARY KEY,
    user_id INTEGER NOT NULL,
    chat_id INTEGER NOT NULL,
    product_id INTEGER NOT NULL REFERENCES products(id),
    qty INTEGER NOT NULL,
    amount INTEGER NOT NULL,
    status TEXT NOT NULL,
    bank_memo TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL,
    paid_at TEXT,
    payment_tx_id TEXT,
    CONSTRAINT status_check CHECK (status IN ('pending', 'paid', 'cancel', 'expired'))
);

CREATE INDEX IF NOT EXISTS idx_orders_user_created ON orders (user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_orders_bank_memo ON orders (bank_memo);
