#!/usr/bin/env bash
set -euo pipefail

APP_NAME="botbanhang"
INSTALL_DIR="/opt/${APP_NAME}"
SERVICE_FILE="/etc/systemd/system/${APP_NAME}.service"
BIN_PATH="${BIN_PATH:-target/release/${APP_NAME}}"
BACKUP_DIR="${INSTALL_DIR}/backups"
TIMESTAMP="$(date -u +%Y%m%d%H%M%S)"

resolve_sqlite_db_path() {
  local database_url=""

  if [[ -f "${INSTALL_DIR}/.env" ]]; then
    database_url="$(
      sudo awk -F= '/^DATABASE_URL=/{print substr($0, index($0, "=") + 1)}' "${INSTALL_DIR}/.env" \
        | tail -n 1 \
        | tr -d '\r' \
        | tr -d '"' \
        | tr -d "'"
    )"
  fi

  if [[ -z "${database_url}" ]]; then
    database_url="sqlite://shop.db"
  fi

  case "${database_url}" in
    sqlite://*) database_url="${database_url#sqlite://}" ;;
    sqlite:*) database_url="${database_url#sqlite:}" ;;
  esac

  if [[ "${database_url}" = /* ]]; then
    printf '%s\n' "${database_url}"
  else
    printf '%s\n' "${INSTALL_DIR}/${database_url}"
  fi
}

backup_current_install() {
  echo "==> Creating backup directory: ${BACKUP_DIR}"
  sudo mkdir -p "${BACKUP_DIR}"

  if [[ -x "${INSTALL_DIR}/${APP_NAME}" ]]; then
    echo "==> Backing up current binary"
    sudo cp -a "${INSTALL_DIR}/${APP_NAME}" "${BACKUP_DIR}/${APP_NAME}.${TIMESTAMP}"
  else
    echo "==> No existing binary to back up"
  fi

  local db_path
  db_path="$(resolve_sqlite_db_path)"
  if [[ -f "${db_path}" ]]; then
    local db_backup_dir="${BACKUP_DIR}/db-${TIMESTAMP}"
    echo "==> Backing up SQLite DB: ${db_path}"
    sudo mkdir -p "${db_backup_dir}"
    sudo cp -a "${db_path}" "${db_backup_dir}/"

    if [[ -f "${db_path}-wal" ]]; then
      sudo cp -a "${db_path}-wal" "${db_backup_dir}/"
    fi
    if [[ -f "${db_path}-shm" ]]; then
      sudo cp -a "${db_path}-shm" "${db_backup_dir}/"
    fi
  else
    echo "==> No SQLite DB found at ${db_path}; skipping DB backup"
  fi
}

if [[ ! -x "$BIN_PATH" ]]; then
  if [[ -x "./${APP_NAME}" ]]; then
    BIN_PATH="./${APP_NAME}"
  else
    echo "Binary not found. Expected at \$BIN_PATH (currently '${BIN_PATH}') or ./botbanhang."
    echo "Giải nén artifact để có sẵn binary rồi chạy lại."
    exit 1
  fi
fi

echo "==> Using prebuilt binary at ${BIN_PATH}"

echo "==> Preparing install dir: ${INSTALL_DIR}"
sudo mkdir -p "${INSTALL_DIR}"
sudo mkdir -p "${INSTALL_DIR}/storage/uploads" "${INSTALL_DIR}/storage/product_files"

echo "==> Stopping service before backup/deploy"
sudo systemctl stop "${APP_NAME}.service" >/dev/null 2>&1 || true

backup_current_install

# copy to temp then move atomically
sudo cp "${BIN_PATH}" "${INSTALL_DIR}/${APP_NAME}.new"
sudo mv -f "${INSTALL_DIR}/${APP_NAME}.new" "${INSTALL_DIR}/${APP_NAME}"

echo "==> Copying configs/assets (.env, public/)"
if [[ -f .env ]]; then
  sudo cp .env "${INSTALL_DIR}/"
fi
if [[ -d public ]]; then
  if [[ -d "${INSTALL_DIR}/public/uploads" ]]; then
    sudo cp -an "${INSTALL_DIR}/public/uploads/." "${INSTALL_DIR}/storage/uploads/" || true
  fi
  sudo rm -rf "${INSTALL_DIR}/public"
  sudo cp -r public "${INSTALL_DIR}/"
fi

echo "==> Installing management scripts..."
sudo cp -f bot_clone.sh bot_update.sh bot_list.sh "${INSTALL_DIR}/" 2>/dev/null || true
sudo chmod +x "${INSTALL_DIR}/bot_clone.sh" "${INSTALL_DIR}/bot_update.sh" "${INSTALL_DIR}/bot_list.sh" 2>/dev/null || true

echo "==> Writing systemd service: ${SERVICE_FILE}"
sudo tee "${SERVICE_FILE}" >/dev/null <<'EOF'
[Unit]
Description=Bot Ban Hang service
After=network.target

[Service]
Type=simple
WorkingDirectory=/opt/botbanhang
ExecStart=/opt/botbanhang/botbanhang
Restart=on-failure
EnvironmentFile=/opt/botbanhang/.env

[Install]
WantedBy=multi-user.target
EOF

echo "==> Reloading and enabling service"
sudo systemctl daemon-reload
sudo systemctl reset-failed "${APP_NAME}.service" >/dev/null 2>&1 || true
if ! sudo systemctl enable --now "${APP_NAME}.service"; then
  echo "==> Service failed to start. Recent status:"
  sudo systemctl status "${APP_NAME}.service" --no-pager -l || true
  echo "==> Recent logs:"
  sudo journalctl -u "${APP_NAME}.service" -n 80 --no-pager || true
  exit 1
fi

echo "==> Done. Check status with: sudo systemctl status ${APP_NAME}"
