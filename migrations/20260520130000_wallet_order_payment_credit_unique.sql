CREATE UNIQUE INDEX IF NOT EXISTS idx_wallet_transactions_order_payment_credit
ON wallet_transactions(order_id, type)
WHERE order_id IS NOT NULL AND type = 'refund';
