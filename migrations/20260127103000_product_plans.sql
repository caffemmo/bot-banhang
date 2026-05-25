-- Plans/options per product (e.g., number of months with custom price)
CREATE TABLE IF NOT EXISTS product_plans (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    product_id INTEGER NOT NULL REFERENCES products(id),
    label TEXT NOT NULL,
    months INTEGER NOT NULL,
    price INTEGER NOT NULL,
    sort_order INTEGER DEFAULT 0,
    created_at TEXT DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_product_plans_product ON product_plans(product_id);
