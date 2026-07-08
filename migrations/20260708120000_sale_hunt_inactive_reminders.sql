CREATE TABLE IF NOT EXISTS sale_hunt_inactive_reminders (
    user_id INTEGER PRIMARY KEY,
    chat_id INTEGER NOT NULL,
    deal_id INTEGER,
    sent_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_sale_hunt_inactive_reminders_sent
    ON sale_hunt_inactive_reminders (sent_at);
