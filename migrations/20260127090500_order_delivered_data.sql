-- Store delivered data snapshot per order (items sent to user)
ALTER TABLE orders ADD COLUMN delivered_data TEXT;
