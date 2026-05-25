#!/usr/bin/env bash
set -euo pipefail

echo "=================================================="
echo "         CÔNG CỤ CẬP NHẬT CODE CHO CÁC BOT        "
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

echo "Chọn chế độ cập nhật:"
echo "1) Cập nhật TẤT CẢ các bot nhân bản (/opt/botbanhang_*)"
echo "2) Cập nhật MỘT bot cụ thể"
read -r -p "Lựa chọn của bạn [1-2]: " MODE

TARGETS=()
if [ "$MODE" = "1" ]; then
    for d in /opt/botbanhang_*; do
        if [ -d "$d" ] && [ -f "$d/botbanhang" ]; then
            TARGETS+=("$d")
        fi
    done
    if [ ${#TARGETS[@]} -eq 0 ]; then
        echo "Không tìm thấy bot nhân bản nào khớp với /opt/botbanhang_*."
        exit 0
    fi
elif [ "$MODE" = "2" ]; then
    read -r -p "Nhập chính xác tên thư mục bot (ví dụ: botbanhang_shop2): " SPECIFIC
    if [ ! -d "/opt/$SPECIFIC" ] || [ ! -f "/opt/$SPECIFIC/botbanhang" ]; then
        echo "Lỗi: Không tìm thấy bot hợp lệ tại /opt/$SPECIFIC."
        exit 1
    fi
    TARGETS+=("/opt/$SPECIFIC")
else
    echo "Lỗi: Lựa chọn không hợp lệ."
    exit 1
fi

for TARGET in "${TARGETS[@]}"; do
    BOT_NAME=$(basename "$TARGET")
    echo "--------------------------------------------------"
    echo "==> Đang cập nhật bot: $BOT_NAME"
    
    if systemctl is-active --quiet "$BOT_NAME"; then
        echo "   -> Đang dừng service $BOT_NAME..."
        systemctl stop "$BOT_NAME"
    fi
    
    BACKUP_DIR="$TARGET/backups"
    mkdir -p "$BACKUP_DIR"
    TS=$(date -u +%Y%m%d%H%M%S)
    
    echo "   -> Đang sao lưu file chạy cũ vào backups/botbanhang.$TS..."
    cp -a "$TARGET/botbanhang" "$BACKUP_DIR/botbanhang.$TS"
    
    echo "   -> Đang copy file chạy binary và public mới từ bot gốc..."
    cp -a "$MASTER_DIR/botbanhang" "$TARGET/botbanhang"
    rm -rf "$TARGET/public"
    cp -a "$MASTER_DIR/public" "$TARGET/public"
    
    echo "   -> Đang khởi động lại service $BOT_NAME..."
    systemctl start "$BOT_NAME"
    
    if systemctl is-active --quiet "$BOT_NAME"; then
        echo "   -> $BOT_NAME đã được cập nhật và đang hoạt động thành công!"
    else
        echo "   -> CẢNH BÁO: $BOT_NAME khởi động thất bại sau khi cập nhật. Xem log bằng lệnh: journalctl -u $BOT_NAME"
    fi
done

echo "=================================================="
echo "==> Hoàn tất toàn bộ quá trình cập nhật."
