CREATE TABLE IF NOT EXISTS giot_viet_physical_orders (
    id TEXT PRIMARY KEY,
    customer_name TEXT NOT NULL,
    phone TEXT NOT NULL,
    province TEXT NOT NULL,
    ward TEXT NOT NULL,
    address TEXT NOT NULL,
    delivery_note TEXT,
    items_json TEXT NOT NULL,
    status TEXT NOT NULL DEFAULT 'new' CHECK (status IN ('new', 'contacted', 'confirmed', 'shipping', 'delivered', 'cancelled')),
    created_at TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_giot_viet_physical_orders_created_at
    ON giot_viet_physical_orders(created_at DESC);

CREATE INDEX IF NOT EXISTS idx_giot_viet_physical_orders_status
    ON giot_viet_physical_orders(status, created_at DESC);
