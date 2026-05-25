ALTER TABLE subscribers ADD COLUMN username TEXT;
ALTER TABLE subscribers ADD COLUMN first_name TEXT;
ALTER TABLE subscribers ADD COLUMN last_name TEXT;
ALTER TABLE subscribers ADD COLUMN full_name TEXT;
ALTER TABLE subscribers ADD COLUMN language_code TEXT;
ALTER TABLE subscribers ADD COLUMN is_bot INTEGER DEFAULT 0;
ALTER TABLE subscribers ADD COLUMN updated_at TEXT;

UPDATE subscribers
SET full_name = trim(
    coalesce(first_name, '') ||
    CASE
        WHEN first_name IS NOT NULL AND first_name <> '' AND last_name IS NOT NULL AND last_name <> '' THEN ' '
        ELSE ''
    END ||
    coalesce(last_name, '')
)
WHERE (full_name IS NULL OR full_name = '')
  AND (
      (first_name IS NOT NULL AND first_name <> '')
      OR (last_name IS NOT NULL AND last_name <> '')
  );
