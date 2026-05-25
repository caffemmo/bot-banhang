CREATE TABLE IF NOT EXISTS client_api_keys (
    chat_id INTEGER PRIMARY KEY,
    token_hash TEXT NOT NULL,
    created_at TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at TEXT NOT NULL DEFAULT (datetime('now'))
);
