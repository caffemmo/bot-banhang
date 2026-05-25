#!/usr/bin/env bash
set -euo pipefail

echo "================================================================================="
printf "%-25s %-12s %-8s %-10s %-8s %-20s\n" "TÊN BOT" "TRẠNG THÁI" "PID" "RAM" "CỔNG" "PREFIX TELEGRAM"
echo "================================================================================="

if [ "${EUID:-0}" -ne 0 ]; then
    echo "Lỗi: Script này phải được chạy với quyền root (sudo) để kiểm tra các service hệ thống."
    exit 1
fi

SERVICES=$(systemctl list-units --type=service --all | grep -E 'botbanhang.*\.service' | awk '{print $1}' | sed 's/\.service//' || true)

if [ -z "$SERVICES" ]; then
    echo "Không tìm thấy service botbanhang nào đang chạy trên hệ thống."
    exit 0
fi

for BOT in $SERVICES; do
    STATUS=$(systemctl is-active "$BOT" 2>/dev/null || echo "unknown")
    PID=$(systemctl show -p MainPID "$BOT" 2>/dev/null | cut -d= -f2 || echo "0")
    if [ "$PID" = "0" ]; then PID="-"; fi
    
    RAM="-"
    if [ "$PID" != "-" ] && [ "$PID" -gt 0 ]; then
        RAM=$(ps -p "$PID" -o rss= 2>/dev/null | awk '{printf "%.1fM", $1/1024}' || echo "-")
    fi
    
    PORT="-"
    PREFIX="-"
    ENV_FILE="/opt/$BOT/.env"
    if [ -f "$ENV_FILE" ]; then
        PORT=$(grep -E '^PORT=' "$ENV_FILE" | cut -d= -f2 | tr -d '"' | tr -d "'" || echo "-")
        TOK=$(grep -E '^TELOXIDE_TOKEN=' "$ENV_FILE" | cut -d= -f2 | tr -d '"' | tr -d "'" || echo "")
        if [ -n "$TOK" ]; then
            PREFIX=$(echo "$TOK" | cut -d: -f1)
        fi
    fi
    
    printf "%-25s %-12s %-8s %-10s %-8s %-20s\n" "$BOT" "$STATUS" "$PID" "$RAM" "$PORT" "$PREFIX:***"
done
echo "================================================================================="
