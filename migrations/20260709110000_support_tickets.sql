CREATE TABLE IF NOT EXISTS support_tickets (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    public_key TEXT NOT NULL UNIQUE,
    kind TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'open',
    customer_name TEXT,
    contact_method TEXT NOT NULL DEFAULT 'web',
    contact_value TEXT,
    order_ref TEXT,
    facebook_ref TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    closed_at TEXT
);

CREATE TABLE IF NOT EXISTS support_messages (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    ticket_id INTEGER NOT NULL,
    sender TEXT NOT NULL,
    message TEXT NOT NULL,
    created_at TEXT NOT NULL,
    FOREIGN KEY (ticket_id) REFERENCES support_tickets(id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_support_tickets_status_created
    ON support_tickets (status, created_at);

CREATE INDEX IF NOT EXISTS idx_support_messages_ticket_created
    ON support_messages (ticket_id, created_at);
