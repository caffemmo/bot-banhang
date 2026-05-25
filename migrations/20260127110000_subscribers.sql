CREATE TABLE IF NOT EXISTS subscribers (
    user_id INTEGER PRIMARY KEY,
    chat_id INTEGER NOT NULL,
    created_at TEXT DEFAULT (datetime('now'))
);
