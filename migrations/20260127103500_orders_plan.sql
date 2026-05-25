-- Store selected plan per order (when product has custom plans)
ALTER TABLE orders ADD COLUMN plan_id INTEGER;
ALTER TABLE orders ADD COLUMN plan_label TEXT;
ALTER TABLE orders ADD COLUMN plan_months INTEGER;
ALTER TABLE orders ADD COLUMN plan_price INTEGER;
