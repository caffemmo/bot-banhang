CREATE TABLE IF NOT EXISTS crypto_payment_requests (
    id                      INTEGER PRIMARY KEY AUTOINCREMENT,
    purpose                 TEXT    NOT NULL DEFAULT 'order'
                            CHECK(purpose IN ('order','wallet_topup')),
    order_id                TEXT REFERENCES orders(id),
    wallet_topup_id         INTEGER,
    user_id                 INTEGER NOT NULL,
    chat_id                 INTEGER NOT NULL,
    method                  TEXT    NOT NULL CHECK(method IN ('binance_pay','bep20')),
    amount_vnd              INTEGER NOT NULL,
    amount_usdt             REAL,
    rate_vnd_per_usdt       TEXT    NOT NULL,
    amount_usdt_base        TEXT    NOT NULL,
    amount_usdt_expected    TEXT    NOT NULL,
    amount_token_units      TEXT    NOT NULL,
    memo                    TEXT    NOT NULL UNIQUE,
    address                 TEXT,
    binance_prepay_id       TEXT,
    binance_checkout_url    TEXT,
    binance_qrcode_link     TEXT,
    binance_qr_content      TEXT,
    binance_deeplink        TEXT,
    binance_universal_url   TEXT,
    binance_transaction_id  TEXT,
    binance_open_user_id    TEXT,
    tx_hash                 TEXT,
    tx_from                 TEXT,
    tx_block_number         INTEGER,
    confirmations           INTEGER NOT NULL DEFAULT 0,
    status                  TEXT    NOT NULL DEFAULT 'pending'
                            CHECK(status IN (
                                'pending',
                                'confirming',
                                'completed',
                                'expired',
                                'failed',
                                'manual_review'
                            )),
    failure_reason          TEXT,
    created_at              TEXT    NOT NULL DEFAULT (datetime('now')),
    expires_at              TEXT    NOT NULL,
    completed_at            TEXT,
    updated_at              TEXT    NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_crypto_pay_order
ON crypto_payment_requests(order_id);

CREATE INDEX IF NOT EXISTS idx_crypto_pay_memo
ON crypto_payment_requests(memo);

CREATE INDEX IF NOT EXISTS idx_crypto_pay_status
ON crypto_payment_requests(user_id, status);

CREATE INDEX IF NOT EXISTS idx_crypto_pay_method_status
ON crypto_payment_requests(method, status, created_at);

CREATE UNIQUE INDEX IF NOT EXISTS idx_crypto_pay_pending_bep20_units
ON crypto_payment_requests(amount_token_units)
WHERE method = 'bep20' AND status IN ('pending','confirming');

CREATE UNIQUE INDEX IF NOT EXISTS idx_crypto_pay_tx_hash
ON crypto_payment_requests(tx_hash)
WHERE tx_hash IS NOT NULL;

CREATE TABLE IF NOT EXISTS crypto_worker_state (
    key        TEXT PRIMARY KEY,
    value      TEXT NOT NULL,
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
