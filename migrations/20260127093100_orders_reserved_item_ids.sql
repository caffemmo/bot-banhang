-- Track reserved item ids for each order so we can release stock
ALTER TABLE orders ADD COLUMN reserved_item_ids TEXT;

