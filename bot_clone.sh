#!/usr/bin/env bash
set -euo pipefail

echo "=================================================="
echo "         CÔNG CỤ TẠO VÀ CẤU HÌNH BOT MỚI          "
echo "=================================================="

if [ "${EUID:-0}" -ne 0 ]; then
    echo "Lỗi: Script này phải được chạy với quyền root (sudo)."
    exit 1
fi

MASTER_DIR="/opt/botbanhang"
if [ ! -d "$MASTER_DIR" ] || [ ! -f "$MASTER_DIR/botbanhang" ]; then
    echo "Lỗi: Không tìm thấy bot gốc tại $MASTER_DIR."
    exit 1
fi

read -r -p "Nhập phần tên phụ cho bot mới (ví dụ nhập 'shop2' sẽ tạo 'botbanhang_shop2'): " SUFFIX
if [[ ! "$SUFFIX" =~ ^[a-zA-Z0-9_-]+$ ]]; then
    echo "Lỗi: Tên không hợp lệ. Chỉ sử dụng chữ cái, số, dấu gạch dưới (_) và dấu gạch ngang (-)."
    exit 1
fi
BOT_NAME="botbanhang_${SUFFIX#botbanhang_}"

TARGET_DIR="/opt/$BOT_NAME"
if [ -d "$TARGET_DIR" ]; then
    echo "Lỗi: Thư mục đích $TARGET_DIR đã tồn tại."
    exit 1
fi

read -r -p "Nhập TELOXIDE_TOKEN (Token Telegram): " TELOXIDE_TOKEN
if [ -z "$TELOXIDE_TOKEN" ]; then
    echo "Lỗi: TELOXIDE_TOKEN không được để trống."
    exit 1
fi

read -r -p "Nhập PORT chạy Web Admin (ví dụ: 8081): " PORT
if ! [[ "$PORT" =~ ^[0-9]+$ ]] || [ "$PORT" -le 1024 ] || [ "$PORT" -gt 65535 ]; then
    echo "Lỗi: Số cổng (Port) không hợp lệ."
    exit 1
fi

if ss -tuln | grep -q ":$PORT "; then
    echo "Lỗi: Cổng $PORT hiện đang bị ứng dụng khác sử dụng."
    exit 1
fi

read -r -p "Nhập DATABASE_URL [Mặc định: sqlite://shop.db]: " DATABASE_URL
DATABASE_URL=${DATABASE_URL:-sqlite://shop.db}

read -r -p "Nhập ADMIN_SETUP_CODE (Mã cài đặt Admin) [Mặc định: ngẫu nhiên 9 số]: " ADMIN_SETUP_CODE
if [ -z "$ADMIN_SETUP_CODE" ]; then
    ADMIN_SETUP_CODE=$(tr -dc '0-9' < /dev/urandom | fold -w 9 | head -n 1)
fi

read -r -p "Nhập ADMIN_JWT_SECRET (Mã bảo mật JWT) [Mặc định: ngẫu nhiên 32 ký tự]: " ADMIN_JWT_SECRET
if [ -z "$ADMIN_JWT_SECRET" ]; then
    ADMIN_JWT_SECRET=$(tr -dc 'a-zA-Z0-9' < /dev/urandom | fold -w 32 | head -n 1)
fi

echo "==> Đang tạo cấu trúc thư mục tại $TARGET_DIR..."
mkdir -p "$TARGET_DIR/storage/uploads"
mkdir -p "$TARGET_DIR/storage/product_files"
chmod 700 "$TARGET_DIR/storage/product_files"

echo "==> Đang copy file chạy binary và các file giao diện public..."
cp -a "$MASTER_DIR/botbanhang" "$TARGET_DIR/botbanhang"
cp -a "$MASTER_DIR/public" "$TARGET_DIR/public"

echo "==> Đang tạo file cấu hình .env..."
cat <<EOF > "$TARGET_DIR/.env"
TELOXIDE_TOKEN=$TELOXIDE_TOKEN
DATABASE_URL=$DATABASE_URL
WEBHOOK_SECRET=$(tr -dc 'a-f0-9' < /dev/urandom | fold -w 64 | head -n 1)
PORT=$PORT
LOG_LEVEL=info
ADMIN_JWT_SECRET=$ADMIN_JWT_SECRET
ADMIN_SETUP_CODE=$ADMIN_SETUP_CODE
ADMIN_COOKIE_SECURE=false
RESERVE_TTL_MINUTES=5
EOF

chmod 600 "$TARGET_DIR/.env"

echo "==> Đang tạo systemd service $BOT_NAME.service..."
cat <<EOF > "/etc/systemd/system/$BOT_NAME.service"
[Unit]
Description=Bot Ban Hang Service ($BOT_NAME)
After=network.target

[Service]
Type=simple
User=root
WorkingDirectory=$TARGET_DIR
ExecStart=$TARGET_DIR/botbanhang
Restart=on-failure
EnvironmentFile=$TARGET_DIR/.env

[Install]
WantedBy=multi-user.target
EOF

echo "==> Đang tải lại cấu hình systemd và khởi động service..."
systemctl daemon-reload
systemctl enable --now "$BOT_NAME.service"

echo "==> Tạo bot thành công! Kiểm tra trạng thái bằng lệnh: systemctl status $BOT_NAME"
