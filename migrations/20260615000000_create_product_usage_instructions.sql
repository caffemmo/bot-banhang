CREATE TABLE IF NOT EXISTS product_usage_instructions (
    product_id INTEGER PRIMARY KEY NOT NULL,
    content TEXT NOT NULL,
    updated_at TEXT DEFAULT CURRENT_TIMESTAMP,
    FOREIGN KEY(product_id) REFERENCES products(id) ON DELETE CASCADE
);
