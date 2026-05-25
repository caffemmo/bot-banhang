-- Add sort_order column to products table
ALTER TABLE products ADD COLUMN sort_order INTEGER DEFAULT 0;

-- Initialize sort_order to match product id so current ordering is preserved
UPDATE products SET sort_order = id;
