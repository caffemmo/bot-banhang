ALTER TABLE products ADD COLUMN delivery_type TEXT NOT NULL DEFAULT 'stock_item';
ALTER TABLE products ADD COLUMN file_path TEXT;
ALTER TABLE products ADD COLUMN file_name TEXT;
ALTER TABLE products ADD COLUMN file_mime TEXT;

UPDATE products
SET delivery_type = CASE
    WHEN IFNULL(requires_input, 0) = 1 THEN 'manual_input'
    ELSE 'stock_item'
END;
