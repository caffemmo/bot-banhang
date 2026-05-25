-- Allow products to require extra buyer input (e.g., email)
ALTER TABLE products ADD COLUMN requires_input INTEGER NOT NULL DEFAULT 0;
ALTER TABLE products ADD COLUMN input_prompt TEXT;
