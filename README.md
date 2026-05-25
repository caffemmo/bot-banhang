# Bot Telegram Bán Hàng (Rust + Teloxide + Axum + SQLite)

## Tính năng chính

- /shop: duyệt sản phẩm, hỗ trợ sản phẩm cần thông tin kích hoạt (requires_input) + gói (plans) với giá khác nhau.
- /orders: xem 10 đơn gần nhất của user.
- Thanh toán tự động:
    - **VietQR (Bank transfer)**: Gửi QR, tự đối soát qua webhook (SePay).
- Quản lý đơn hàng: tự hết hạn sau 5-15 phút nếu chưa thanh toán (worker nền).
- Admin `/admin`: CRUD sản phẩm, item stock, plans, ẩn nút add-item với requires_input; xem đơn, mark paid, resend data, export CSV, thống kê doanh thu 7/30 ngày, broadcast tới user (text + ảnh).
- Lưu subscriber khi /start để broadcast; gửi thông báo hết hạn đơn.

## Yêu cầu

- Rust toolchain (stable), SQLite (libsqlite3), OpenSSL.

## Cấu hình (.env)

```ini
TELOXIDE_TOKEN=your_bot_token
DATABASE_URL=sqlite://shop.db
ADMIN_JWT_SECRET=change-this-to-at-least-32-random-chars
ADMIN_SETUP_CODE=change-this-setup-code
ADMIN_COOKIE_SECURE=false
WEBHOOK_SECRET=change-me (dành cho SePay / VietQR)
```

## Khởi tạo & chạy local   

```bash
cargo build
sqlx migrate run          # chạy migrations trong ./migrations
cargo run
```

Bot chạy long-polling; server Axum mở cổng PORT (mặc định 8080).

## Webhook thanh toán (mẫu)

URL webhook SePay: `https://<domain-cua-ban>/webhook/payment`.
Trong UI SePay chỉ nhập giá trị `WEBHOOK_SECRET`, không nhập thêm chữ `Apikey`.
Backend nhận cả `Authorization: <WEBHOOK_SECRET>`, `Authorization: Apikey <WEBHOOK_SECRET>` và header cũ `X-Webhook-Secret`.

```bash
curl -X POST http://localhost:8080/webhook/payment \
  -H "Content-Type: application/json" \
  -H "Authorization: $WEBHOOK_SECRET" \
  -d '{"memo":"MEMO123","amount":50000,"status":"paid","tx_id":"TX123"}'
```

## Admin

- Truy cập `/admin`, tạo admin đầu tiên bằng `ADMIN_SETUP_CODE`, sau đó đăng nhập bằng tài khoản/mật khẩu.
- Xuất CSV: `/api/admin/orders/export`.
- Doanh thu: `/api/admin/stats/revenue?days=7`.

## Build nhanh

- Windows & Linux (cross) bằng PowerShell: `./build.ps1`
- Linux native: `cargo build --release`
- Quy trình build/deploy VPS chi tiết: [`docs/BUILD_DEPLOY.md`](docs/BUILD_DEPLOY.md)

## Triển khai Linux (systemd)

Chạy script có sẵn:

```bash
./deploy.sh
```

Script sẽ build, copy binary + config vào `/opt/botbanhang`, tạo service `/etc/systemd/system/botbanhang.service`, reload và start. Kiểm tra:

```bash
sudo systemctl status botbanhang
sudo journalctl -u botbanhang -f
```

Đổi PORT bằng cách sửa `.env` trong `/opt/botbanhang` rồi `sudo systemctl restart botbanhang`.

## Backup / dữ liệu

- DB: `shop.db` (SQLite). Sao lưu định kỳ file này.
- File cấu hình: `.env`.
- Khi deploy bằng `deploy.sh`, script sẽ tự backup binary và DB vào `/opt/botbanhang/backups` trước khi thay binary mới.

## Lưu ý bảo mật

- Giữ `WEBHOOK_SECRET`, `ADMIN_JWT_SECRET`, `ADMIN_SETUP_CODE`, token bot bí mật.
- Nếu chạy systemd dưới user riêng: tạo user `bot`, chown `/opt/botbanhang`, thêm `User=bot` vào service.

## Thư mục nguồn

- `src/bot/` bot flow, handlers, keyboards, texts.
- `src/server/` API/admin/webhook/broadcast/stats/worker.
- `src/db/` models, repo, migrations.
- `build.ps1`, `deploy.sh` hỗ trợ build/deploy.
