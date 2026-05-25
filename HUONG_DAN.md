# HƯỚNG DẪN CÀI ĐẶT & CHẠY BOTBÁNHÀNG (Rust + Teloxide + Axum + SQLite)

Tài liệu này hướng dẫn cài đặt môi trường, chạy local và deploy lên server (systemd + Nginx/HTTPS).

---

## 1) Yêu cầu hệ thống

- Ubuntu/Debian Linux (khuyến nghị Ubuntu 22.04+)
- Quyền `sudo`
- Mở cổng HTTP/HTTPS (80/443) nếu chạy public

Project dùng:
- Rust (toolchain qua `rustup`)
- SQLite (file DB)
- (Tuỳ chọn) Nginx + Certbot để chạy HTTPS

---

## 2) Cài Rust (đúng version)

Project đã chạy ổn với **Rust 1.92.0**.

### Cài rustup

```bash
curl https://sh.rustup.rs -sSf | sh -s -- -y
. "$HOME/.cargo/env"
```

### Cài đúng toolchain 1.92.0 và set mặc định

```bash
rustup toolchain install 1.92.0
rustup default 1.92.0

rustc --version
cargo --version
```

> Gợi ý: nếu bạn muốn dùng stable mới hơn vẫn được, nhưng để tránh lệch dependency thì nên bám 1.92.0.

---

## 3) Cài thư viện hệ thống cần thiết

### SQLite libs

```bash
sudo apt update
sudo apt install -y sqlite3 libsqlite3-dev
```

### OpenSSL (thường đã có sẵn)

```bash
sudo apt install -y pkg-config libssl-dev
```

---

## 4) Clone source và chuẩn bị cấu hình

```bash
git clone https://github.com/ptn1411/botbanhang.git
cd botbanhang
```

### Tạo file `.env`

Repo có file mẫu `.env.example`. Copy sang `.env` và điền giá trị thật:

```bash
cp .env.example .env
nano .env
```

Các biến quan trọng:
- `TELOXIDE_TOKEN`: token bot Telegram
- `DATABASE_URL`: mặc định `sqlite://shop.db`
- `WEBHOOK_SECRET`: secret để SePay gọi webhook (header `Authorization: Apikey <secret>`)
- `ADMIN_JWT_SECRET`: secret ký JWT cho phiên admin, tối thiểu 32 ký tự ngẫu nhiên
- `ADMIN_SETUP_CODE`: mã bí mật để tạo admin đầu tiên, tạo admin mới và đổi mật khẩu admin
- `ADMIN_COOKIE_SECURE`: đặt `true` khi chạy HTTPS production
- `PORT`: cổng server (mặc định 8080)

---

## 5) Khởi tạo database và chạy migrations

### Cài `sqlx-cli` (SQLite)

```bash
. "$HOME/.cargo/env"
cargo install sqlx-cli --no-default-features --features sqlite
```

### Tạo DB file và chạy migrations

```bash
cd botbanhang

# tạo file db (nếu chưa có)
touch shop.db

# chạy migrations theo DATABASE_URL trong .env
sqlx migrate run
```

---

## 6) Chạy local

```bash
. "$HOME/.cargo/env"

cargo build
cargo run
```

Sau khi chạy:
- Server Axum lắng nghe trên `0.0.0.0:PORT` (mặc định 8080)
- Bot Telegram chạy polling

Test nhanh:
- Trang admin: `http://localhost:8080/admin`
- Trang test webhook: `http://localhost:8080/webhook/test`

---

## 7) Webhook SePay: format & cách test

### SePay gọi webhook

- URL: `POST /webhook/payment`
- Header auth: `Authorization: Apikey <WEBHOOK_SECRET>`
- Body JSON: theo format SePay (ví dụ):

```json
{
  "id": 92704,
  "gateway": "Vietcombank",
  "transactionDate": "2023-03-25 14:02:37",
  "accountNumber": "0123499999",
  "code": null,
  "content": "chuyen tien DHABCDEFGH mua iphone",
  "transferType": "in",
  "transferAmount": 2277000,
  "accumulated": 19077000,
  "subAccount": null,
  "referenceCode": "MBVCB.3278907687",
  "description": ""
}
```

Lưu ý quan trọng:
- Bot tạo memo dạng `DH` + 8 ký tự (VD `DHABCDEFGH`).
- Memo phải xuất hiện trong `content` (hoặc SePay config được `code` thì sẽ ưu tiên `code`).

### Test bằng curl

```bash
curl -X POST http://localhost:8080/webhook/payment \
  -H 'Content-Type: application/json' \
  -H "Authorization: Apikey $WEBHOOK_SECRET" \
  -d '{
    "id": 1,
    "gateway": "Vietcombank",
    "transactionDate": "2026-01-28 10:00:00",
    "accountNumber": null,
    "code": null,
    "content": "DHABCDEFGH",
    "transferType": "in",
    "transferAmount": 1000,
    "accumulated": null,
    "subAccount": null,
    "referenceCode": "TESTREF",
    "description": null
  }'
```

---

## 8) Admin UI/API

### Admin UI
- URL: `/admin`
- Lần đầu truy cập, trang sẽ yêu cầu tạo admin đầu tiên bằng `ADMIN_SETUP_CODE`.
- Sau đó đăng nhập bằng tài khoản/mật khẩu admin. JWT được lưu trong cookie HttpOnly.
- Trong tab `Admin`, admin đã đăng nhập có thể tạo admin mới hoặc đổi mật khẩu admin khác; cả hai thao tác đều yêu cầu `ADMIN_SETUP_CODE`.

### Admin API
Các API admin nằm dưới `/api/admin/*` và yêu cầu cookie phiên `admin_session` được tạo bởi `/api/auth/login`.

Một số endpoint:
- `/api/admin/products`
- `/api/admin/orders`
- `/api/admin/stats/revenue`
- `/api/admin/webhooks/events` (xem log webhook)

---

## 9) Deploy lên server bằng systemd

Repo có sẵn script `deploy.sh`.

### Chạy deploy

```bash
cd botbanhang
. "$HOME/.cargo/env"

bash ./deploy.sh
```

Script sẽ:
- Build release
- Copy binary + `.env` + `public/` sang `/opt/botbanhang`
- Tạo systemd service `/etc/systemd/system/botbanhang.service`
- Enable + start service

### Xem log

```bash
sudo systemctl status botbanhang --no-pager -l
sudo journalctl -u botbanhang -f
```

---

## 11) Nginx + HTTPS (Let’s Encrypt)

Nếu muốn chạy domain HTTPS (ví dụ `demo.com`):

### Cài Nginx + Certbot

```bash
sudo apt update
sudo apt install -y nginx certbot python3-certbot-nginx
```

### Reverse proxy (HTTP)
Tạo file `/etc/nginx/sites-available/demo.com`:

```nginx
server {
  listen 80;
  listen [::]:80;
  server_name demo.com;

  location / {
    proxy_pass http://127.0.0.1:8080;
    proxy_set_header Host $host;
    proxy_set_header X-Real-IP $remote_addr;
    proxy_set_header X-Forwarded-For $proxy_add_x_forwarded_for;
    proxy_set_header X-Forwarded-Proto $scheme;
  }
}
```

Enable + reload:

```bash
sudo ln -s /etc/nginx/sites-available/demo.com /etc/nginx/sites-enabled/
sudo nginx -t
sudo systemctl reload nginx
```

### Bật HTTPS

```bash
sudo certbot --nginx -d demo.com --redirect
```

---

## 12) Các lỗi hay gặp

### 1) `sqlx migrate run` báo `unable to open database file`
- Kiểm tra `DATABASE_URL` trong `.env`
- Với SQLite, hãy đảm bảo file tồn tại: `touch shop.db`

### 2) Vào `/admin` bị 500 `admin page missing`
- Deploy phải copy `public/` sang `/opt/botbanhang/public`.
- Đã fix trong `deploy.sh`.

### 3) Webhook trả `memo not found`
- Nội dung chuyển khoản `content` không chứa memo `DHXXXXXXXX`.

---

## 13) Version dependencies (tham khảo)

- Rust: **1.92.0**
- Axum: 0.7
- Teloxide: 0.12
- SQLx: 0.7
- SQLite: file-based

Chi tiết xem trong `Cargo.toml`.
