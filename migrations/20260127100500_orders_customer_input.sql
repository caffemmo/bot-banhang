-- Store buyer-provided data per order (e.g., email to activate)
ALTER TABLE orders ADD COLUMN customer_input TEXT;
