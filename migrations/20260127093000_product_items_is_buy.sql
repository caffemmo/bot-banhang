-- Add is_buy flag to product_items to mark reserved/used items
ALTER TABLE product_items ADD COLUMN is_buy INTEGER NOT NULL DEFAULT 0;
CREATE INDEX IF NOT EXISTS idx_product_items_available ON product_items (product_id, is_buy);

