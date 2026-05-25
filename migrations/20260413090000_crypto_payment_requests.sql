CREATE TABLE IF NOT EXISTS crypto_payment_requests (
    id                INTEGER PRIMARY KEY AUTOINCREMENT,
    order_id          TEXT    NOT NULL REFERENCES orders(id),
    user_id           INTEGER NOT NULL,
    chat_id           INTEGER NOT NULL,
    method            TEXT    NOT NULL CHECK(method IN ('binance_pay','bep20')),
    amount_vnd        INTEGER NOT NULL,
    amount_usdt       REAL    NOT NULL,
    rate_vnd_per_usdt REAL    NOT NULL,
    memo              TEXT    NOT NULL UNIQUE,
    address           TEXT,
    binance_prepay_id TEXT,
    tx_hash           TEXT,
    confirmations     INTEGER NOT NULL DEFAULT 0,
    status            TEXT    NOT NULL DEFAULT 'pending'
                      CHECK(status IN ('pending','confirming','completed','expired','failed')),
    created_at        TEXT    NOT NULL DEFAULT (datetime('now')),
    completed_at      TEXT
);

CREATE INDEX idx_crypto_pay_order  ON crypto_payment_requests(order_id);
CREATE INDEX idx_crypto_pay_memo   ON crypto_payment_requests(memo);
CREATE INDEX idx_crypto_pay_status ON crypto_payment_requests(user_id, status);
