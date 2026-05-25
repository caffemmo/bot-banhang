CREATE TABLE IF NOT EXISTS broadcast_templates (
    id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    text TEXT NOT NULL,
    mode TEXT NOT NULL DEFAULT 'message_only'
        CHECK (mode IN ('message_only', 'view_shop', 'product_list', 'new_product')),
    buttons_json TEXT NOT NULL DEFAULT '[]',
    product_id INTEGER,
    sort_order INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT DEFAULT (datetime('now'))
);

INSERT OR IGNORE INTO broadcast_templates (id, name, text, mode, buttons_json, sort_order)
VALUES
(1, 'Hàng mới lên kho',
'{5375135722514685501} HÀNG MỚI VỪA LÊN KHO
━━━━━━━━━━━━

Sản phẩm hot vừa được nhập thêm.
Nhanh tay mua trước khi hết hàng.',
'view_shop',
'[[{"text":"{5375135722514685501} Xem sản phẩm","callback_data":"start:shop"}]]',
1),
(2, 'Flash sale',
'{5375135722514685501} FLASH SALE HÔM NAY
━━━━━━━━━━━━

Một số sản phẩm đang có giá tốt.
Vào shop để xem và đặt đơn ngay.',
'view_shop',
'[[{"text":"{5375135722514685501} Xem sản phẩm","callback_data":"start:shop"}],[{"text":"{5420147074266044260} Nạp ví","callback_data":"wallet:topup"}]]',
2),
(3, 'Nạp ví bonus',
'{5420147074266044260} NẠP VÍ NHẬN BONUS
━━━━━━━━━━━━

Nạp ví trước để thanh toán nhanh hơn khi hàng mới lên kho.',
'message_only',
'[[{"text":"{5420147074266044260} Nạp ví ngay","callback_data":"wallet:topup"},{"text":"Xem ví","callback_data":"start:wallet"}]]',
3),
(4, 'Sản phẩm hot còn ít',
'{5375135722514685501} SẢN PHẨM HOT CÒN ÍT
━━━━━━━━━━━━

Kho đang còn số lượng giới hạn.
Ai thanh toán trước sẽ được giao trước.',
'view_shop',
'[[{"text":"{5375135722514685501} Mua ngay","callback_data":"start:shop"}]]',
4),
(5, 'Thông báo hỗ trợ',
'THÔNG BÁO TỪ SHOP
━━━━━━━━━━━━

Shop đang hỗ trợ xử lý đơn và nạp ví.
Bạn có thể xem đơn đã mua hoặc quay lại shop.',
'message_only',
'[[{"text":"Xem đơn đã mua","callback_data":"start:orders"},{"text":"Xem sản phẩm","callback_data":"start:shop"}]]',
5);
