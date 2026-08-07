#!/usr/bin/env bash
set -euo pipefail

APP_NAME="supportbot"
INSTALL_DIR="/opt/botbanhang"
SERVICE_FILE="/etc/systemd/system/${APP_NAME}.service"
BIN_PATH="${SUPPORT_BIN_PATH:-target/release/supportbot}"

if [[ ! -x "${BIN_PATH}" ]]; then
  echo "Support bot binary not found or not executable: ${BIN_PATH}"
  echo "Build it first with: cargo build --release --bin supportbot"
  exit 1
fi

if [[ ! -f "${INSTALL_DIR}/.env" ]]; then
  echo "Missing ${INSTALL_DIR}/.env. Configure SUPPORT_BOT_TOKEN and SUPPORT_ADMIN_IDS first."
  exit 1
fi

echo "Installing ${APP_NAME} to ${INSTALL_DIR}"
sudo install -D -m 755 "${BIN_PATH}" "${INSTALL_DIR}/${APP_NAME}"
sudo install -D -m 644 supportbot.service.example "${SERVICE_FILE}"

echo "Starting ${APP_NAME}.service"
sudo systemctl daemon-reload
sudo systemctl enable --now "${APP_NAME}.service"
sudo systemctl status "${APP_NAME}.service" --no-pager -l
