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

generate_hex() {
    local bytes="$1"
    if command -v openssl >/dev/null 2>&1; then
        openssl rand -hex "$bytes"
    else
        od -An -N"$bytes" -tx1 /dev/urandom | tr -d ' \n'
        echo
    fi
}

generate_digits() {
    local len="$1"
    local out=""
    local chunk
    while [ "${#out}" -lt "$len" ]; do
        chunk=$(od -An -N4 -tu4 /dev/urandom | awk '{printf "%010u", $1}')
        out="${out}${chunk}"
    done
    printf "%s\n" "${out:0:$len}"
}

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
    ADMIN_SETUP_CODE=$(generate_digits 9)
fi
if [ "${#ADMIN_SETUP_CODE}" -lt 8 ]; then
    echo "Lỗi: ADMIN_SETUP_CODE phải có ít nhất 8 ký tự."
    exit 1
fi

read -r -p "Nhập ADMIN_JWT_SECRET (Mã bảo mật JWT) [Mặc định: ngẫu nhiên 32 ký tự]: " ADMIN_JWT_SECRET
if [ -z "$ADMIN_JWT_SECRET" ]; then
    ADMIN_JWT_SECRET=$(generate_hex 16)
fi
if [ "${#ADMIN_JWT_SECRET}" -lt 32 ]; then
    echo "Lỗi: ADMIN_JWT_SECRET phải có ít nhất 32 ký tự."
    exit 1
fi

echo "==> Đang tạo cấu trúc thư mục tại $TARGET_DIR..."
mkdir -p "$TARGET_DIR/storage/uploads"
mkdir -p "$TARGET_DIR/storage/product_files"
chmod 700 "$TARGET_DIR/storage/product_files"

echo "==> Đang copy file chạy binary, giao diện public và i18n..."
cp -a "$MASTER_DIR/botbanhang" "$TARGET_DIR/botbanhang"
cp -a "$MASTER_DIR/public" "$TARGET_DIR/public"
if [ -d "$MASTER_DIR/i18n" ]; then
    cp -a "$MASTER_DIR/i18n" "$TARGET_DIR/i18n"
fi

echo "==> Đang tạo file cấu hình .env..."
WEBHOOK_SECRET=$(generate_hex 32)
cat <<EOF > "$TARGET_DIR/.env"
TELOXIDE_TOKEN=$TELOXIDE_TOKEN
DATABASE_URL=$DATABASE_URL
WEBHOOK_SECRET=$WEBHOOK_SECRET
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
