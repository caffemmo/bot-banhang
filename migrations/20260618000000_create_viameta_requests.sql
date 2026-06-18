CREATE TABLE IF NOT EXISTS viameta_requests (
    order_id TEXT PRIMARY KEY NOT NULL,
    service TEXT NOT NULL,
    cookie TEXT NOT NULL,
    uid TEXT,
    image_path TEXT,
    status TEXT NOT NULL DEFAULT 'pending',
    response TEXT,
    error TEXT,
    created_at TEXT DEFAULT CURRENT_TIMESTAMP,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(order_id) REFERENCES orders(id) ON DELETE CASCADE
);

