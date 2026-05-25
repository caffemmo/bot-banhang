CREATE TABLE IF NOT EXISTS product_categories (
    id INTEGER PRIMARY KEY AUTOINCREMENT,
    name TEXT NOT NULL UNIQUE,
    emoji TEXT,
    custom_emoji_id TEXT,
    sort_order INTEGER,
    is_active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL DEFAULT (datetime('now'))
);

ALTER TABLE products ADD COLUMN category_id INTEGER;

INSERT OR IGNORE INTO product_categories (name, sort_order)
SELECT TRIM(category), MIN(sort_order)
FROM products
WHERE TRIM(IFNULL(category, '')) <> ''
GROUP BY TRIM(category);

UPDATE products
SET category_id = (
    SELECT pc.id
    FROM product_categories pc
    WHERE pc.name = TRIM(products.category)
)
WHERE TRIM(IFNULL(category, '')) <> '';
