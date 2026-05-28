PRAGMA foreign_keys = OFF;

CREATE TABLE IF NOT EXISTS orders_refund_status_migration (
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
    delivered_data TEXT,
    reserved_item_ids TEXT,
    customer_input TEXT,
    plan_id INTEGER,
    plan_label TEXT,
    plan_months INTEGER,
    plan_price INTEGER,
    reservation_mode TEXT NOT NULL DEFAULT 'reserved'
        CHECK (reservation_mode IN ('reserved', 'no_reserve')),
    CONSTRAINT status_check CHECK (status IN ('pending', 'paid', 'refunded', 'cancel', 'expired'))
);

INSERT INTO orders_refund_status_migration (
    id, user_id, chat_id, product_id, qty, amount, status, bank_memo,
    created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids,
    customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode
)
SELECT
    id, user_id, chat_id, product_id, qty, amount, status, bank_memo,
    created_at, paid_at, payment_tx_id, delivered_data, reserved_item_ids,
    customer_input, plan_id, plan_label, plan_months, plan_price, reservation_mode
FROM orders;

DROP TABLE orders;
ALTER TABLE orders_refund_status_migration RENAME TO orders;

CREATE INDEX IF NOT EXISTS idx_orders_user_created ON orders (user_id, created_at);
CREATE INDEX IF NOT EXISTS idx_orders_bank_memo ON orders (bank_memo);

PRAGMA foreign_keys = ON;
