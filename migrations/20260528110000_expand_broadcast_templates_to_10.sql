INSERT OR IGNORE INTO broadcast_templates (id, name, text, mode, buttons_json, sort_order)
VALUES
(6, 'Cập nhật kho mới',
'{5375135722514685501} KHO VỪA CẬP NHẬT
━━━━━━━━━━━━

Shop vừa thêm hoặc bổ sung một số sản phẩm.
Bạn có thể xem danh sách hiện có và nạp ví trước khi đặt đơn.',
'product_list',
'[[{"text":"Xem danh sách","callback_data":"start:shop"},{"text":"Nạp ví","callback_data":"wallet:topup"}]]',
6),
(7, 'Nhắc nạp ví',
'{5420147074266044260} NHẮC NẠP VÍ
━━━━━━━━━━━━

Bạn nên nạp ví sẵn để thanh toán nhanh khi sản phẩm cần mua còn hàng.',
'message_only',
'[[{"text":"Nạp ví ngay","callback_data":"wallet:topup"},{"text":"Lịch sử nạp","callback_data":"wallet:topup_history"}]]',
7),
(8, 'Hướng dẫn mua hàng',
'HƯỚNG DẪN MUA HÀNG
━━━━━━━━━━━━

Nếu bạn cần xem cách đặt đơn hoặc quay lại danh sách sản phẩm, dùng các nút bên dưới.',
'message_only',
'[[{"text":"Hướng dẫn","callback_data":"start:help"},{"text":"Xem shop","callback_data":"start:shop"}]]',
8),
(9, 'Kiểm tra đơn hàng',
'KIỂM TRA ĐƠN HÀNG
━━━━━━━━━━━━

Bạn có thể xem lại các đơn đã mua và số dư ví hiện tại.',
'message_only',
'[[{"text":"Đơn đã mua","callback_data":"start:orders"},{"text":"Xem ví","callback_data":"start:wallet"}]]',
9),
(10, 'Kết nối API',
'KẾT NỐI API SHOP
━━━━━━━━━━━━

Dùng API để tích hợp mua hàng tự động hoặc tạo key API mới khi cần.',
'message_only',
'[[{"text":"API của tôi","callback_data":"shop_api"},{"text":"Tạo API mới","callback_data":"shop_api_new"}]]',
10);
