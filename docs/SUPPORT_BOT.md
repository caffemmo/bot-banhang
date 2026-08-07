# Telegram Support Bot

`supportbot` is a second Telegram bot binary. It runs beside `botbanhang`, uses the same SQLite database, and has its own Telegram token.

## Configuration

Create a new bot with `@BotFather`, then add these values to `/opt/botbanhang/.env` on the VPS:

```ini
SUPPORT_BOT_TOKEN=token_of_the_new_support_bot
SUPPORT_MANAGER_IDS=123456789
SUPPORT_AGENT_IDS=234567890,345678901
SUPPORT_CASE_PREFIX=SUP
SUPPORT_OVERDUE_MINUTES=30
```

All IDs are Telegram **user IDs**, not usernames. Every manager and agent must send `/start` to the support bot once before Telegram permits the bot to notify them. `SUPPORT_ADMIN_IDS` remains accepted for the existing setup and is treated as `SUPPORT_MANAGER_IDS` when the newer variable is not set.

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

- A customer sends any non-command message. The bot creates one new case and notifies managers and available agents.
- An agent replies `/claim` to the case header to accept it. The bot removes that case from other agents' chats, then routes future customer messages and images only to the assigned agent and managers.
- A manager can reply to every case. An assigned agent can reply only to their own case.
- An assigned agent or a manager replies `/transfer TELEGRAM_ID` to a case to transfer it. The new agent receives the most recent conversation history.
- The customer sends `/close` to close their case.
- A manager uses `/cases` for a summary and `/new`, `/active`, `/overdue`, or `/closed` for a list. A case becomes overdue after `SUPPORT_OVERDUE_MINUTES` without activity.

When a closed customer sends a new message, a new case is created automatically.
