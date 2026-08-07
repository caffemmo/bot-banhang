# Telegram Support Bot

`supportbot` is a second Telegram bot binary. It runs beside `botbanhang`, uses the same SQLite database, and has its own Telegram token.

## Configuration

Create a new bot with `@BotFather`, then add these values to `/opt/botbanhang/.env` on the VPS:

```ini
SUPPORT_BOT_TOKEN=token_of_the_new_support_bot
SUPPORT_ADMIN_IDS=123456789,987654321
SUPPORT_CASE_PREFIX=SUP
```

`SUPPORT_ADMIN_IDS` contains Telegram **user IDs**, not usernames. Every admin must send `/start` to the support bot once before Telegram permits the bot to notify them.

## Install on the VPS

Build the support binary together with the main binary:

```bash
cargo build --release --bin botbanhang --bin supportbot
```

On the VPS, add the support settings to `/opt/botbanhang/.env` first. Then deploy the binary and service with:

```bash
chmod +x deploy_supportbot.sh
./deploy_supportbot.sh
```

Set `SUPPORT_BIN_PATH` when the binary is in a different location.

The first start runs the support-case migration automatically. Check logs with:

```bash
sudo journalctl -u supportbot -f
```

## Using it

- A customer sends any non-command message. The bot creates one open case and notifies every configured admin.
- An admin replies directly to the case header or any copied customer message. The reply is copied to the customer without exposing the admin account.
- The customer sends `/close` to close their case.
- An admin replies to a case header with `/close` to close it, or sends `/cases` to see the current number of open cases.

When a closed customer sends a new message, a new case is created automatically.
