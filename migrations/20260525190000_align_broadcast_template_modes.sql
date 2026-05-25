UPDATE broadcast_templates
SET mode = 'view_shop', updated_at = datetime('now')
WHERE id = 1
  AND name = 'Hàng mới lên kho'
  AND mode = 'message_only';

UPDATE broadcast_templates
SET mode = 'view_shop', updated_at = datetime('now')
WHERE id = 2
  AND name = 'Flash sale'
  AND mode = 'message_only';

UPDATE broadcast_templates
SET mode = 'view_shop', updated_at = datetime('now')
WHERE id = 4
  AND name = 'Sản phẩm hot còn ít'
  AND mode = 'message_only';
