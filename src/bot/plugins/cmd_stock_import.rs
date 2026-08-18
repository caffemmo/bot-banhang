use std::sync::Arc;
use std::time::Duration;

use anyhow::{Context, Result, anyhow};
use reqwest::Url;
use serde::Deserialize;
use teloxide::prelude::Requester;
use teloxide::types::{BotCommand, Message};

use crate::app::AppContext;
use crate::bot::BotDialogue;
use crate::bot::plugins::AppPlugin;

const STOCK_IMPORT_COMMAND: &str = "/nhapkho";
const STOCK_IMPORT_URL_ENV: &str = "CAFFEMMO_STOCK_IMPORT_URL";
const STOCK_IMPORT_KEY_ENV: &str = "CAFFEMMO_STOCK_API_KEY";
const MAX_ACCOUNT_LINES: usize = 250;
const MAX_ACCOUNT_LINE_CHARS: usize = 5_000;
const MAX_REQUEST_BYTES: usize = 900_000;

pub struct StockImportCommandPlugin;

#[derive(Debug, PartialEq, Eq)]
struct StockImportRequest {
    code: String,
    accounts: String,
    account_count: usize,
}

#[derive(Debug, Deserialize)]
struct StockImportResponse {
    status: String,
    msg: String,
}

#[derive(Debug)]
struct StockImportConfig {
    endpoint: Url,
    api_key: String,
}

impl StockImportConfig {
    fn from_env() -> Result<Self> {
        let endpoint = std::env::var(STOCK_IMPORT_URL_ENV)
            .unwrap_or_default()
            .trim()
            .to_string();
        let api_key = std::env::var(STOCK_IMPORT_KEY_ENV)
            .unwrap_or_default()
            .trim()
            .to_string();

        if endpoint.is_empty() || api_key.is_empty() {
            return Err(anyhow!("stock import is not configured"));
        }

        let endpoint = Url::parse(&endpoint).context("invalid stock import endpoint")?;
        if endpoint.scheme() != "https" {
            return Err(anyhow!("stock import endpoint must use HTTPS"));
        }

        Ok(Self { endpoint, api_key })
    }
}

#[async_trait::async_trait]
impl AppPlugin for StockImportCommandPlugin {
    fn name(&self) -> &'static str {
        "CmdStockImport"
    }

    fn commands(&self) -> Vec<BotCommand> {
        vec![BotCommand {
            command: "nhapkho".to_string(),
            description: "Admin: nhập hàng vào kho website".to_string(),
        }]
    }

    async fn handle_message(
        &self,
        ctx: Arc<AppContext>,
        msg: Message,
        _dialogue: BotDialogue,
    ) -> Result<bool, anyhow::Error> {
        let text = msg.text().unwrap_or("").trim();
        let Some(parsed) = parse_stock_import_command(text) else {
            return Ok(false);
        };

        let Some(user) = msg.from() else {
            return Ok(true);
        };
        if !ctx.is_telegram_admin(user.id.0 as i64) {
            ctx.bot
                .send_message(msg.chat.id, "Bạn không có quyền nhập hàng vào kho.")
                .await?;
            return Ok(true);
        }
        if !msg.chat.is_private() {
            ctx.bot
                .send_message(
                    msg.chat.id,
                    "Vì dữ liệu hàng là thông tin nhạy cảm, hãy dùng /nhapkho trong chat riêng với bot.",
                )
                .await?;
            return Ok(true);
        }

        let request = match parsed {
            Ok(request) => request,
            Err(message) => {
                ctx.bot.send_message(msg.chat.id, message).await?;
                return Ok(true);
            }
        };
        let config = match StockImportConfig::from_env() {
            Ok(config) => config,
            Err(_) => {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        "Chức năng nhập kho chưa được cấu hình trên máy chủ bot.",
                    )
                    .await?;
                return Ok(true);
            }
        };

        // Remove the source message so account data is not retained in Telegram chat history.
        let _ = ctx.bot.delete_message(msg.chat.id, msg.id).await;
        let progress = ctx
            .bot
            .send_message(
                msg.chat.id,
                format!(
                    "Đang nhập {} tài khoản vào kho {}...",
                    request.account_count, request.code
                ),
            )
            .await?;

        let result = submit_stock_import(&config, &request).await;
        let _ = ctx.bot.delete_message(msg.chat.id, progress.id).await;

        match result {
            Ok(message) => {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        format!(
                            "Đã cập nhật kho {} từ bot.\n{}",
                            request.code,
                            limit_message(&message, 1_000)
                        ),
                    )
                    .await?;
            }
            Err(error) => {
                ctx.bot
                    .send_message(
                        msg.chat.id,
                        format!("Không thể cập nhật kho: {}", limit_message(&error.to_string(), 1_000)),
                    )
                    .await?;
            }
        }

        Ok(true)
    }
}

async fn submit_stock_import(
    config: &StockImportConfig,
    request: &StockImportRequest,
) -> Result<String> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .context("cannot create stock import client")?;
    let response = client
        .post(config.endpoint.clone())
        .bearer_auth(&config.api_key)
        .header("X-Api-Key", &config.api_key)
        .form(&[
            ("code", request.code.as_str()),
            ("account", request.accounts.as_str()),
            ("filter", "1"),
        ])
        .send()
        .await
        .context("website did not respond")?;
    let http_status = response.status();
    let body = response
        .text()
        .await
        .context("cannot read website response")?;
    let payload: StockImportResponse = serde_json::from_str(&body)
        .map_err(|_| anyhow!("website returned an invalid response (HTTP {http_status})"))?;

    if !http_status.is_success() || payload.status != "success" {
        return Err(anyhow!(if payload.msg.trim().is_empty() {
            format!("website rejected the import (HTTP {http_status})")
        } else {
            payload.msg
        }));
    }

    Ok(payload.msg)
}

fn parse_stock_import_command(text: &str) -> Option<Result<StockImportRequest, &'static str>> {
    let text = text.trim();
    let command_end = text.find(char::is_whitespace).unwrap_or(text.len());
    let command = &text[..command_end];
    if !command_matches(command) {
        return None;
    }

    let payload = text[command_end..].trim();
    let mut lines = payload.lines().map(str::trim).filter(|line| !line.is_empty());
    let Some(code) = lines.next() else {
        return Some(Err(stock_import_usage()));
    };
    if code.chars().count() > 255 || code.chars().any(char::is_whitespace) {
        return Some(Err("Mã kho không hợp lệ. Mã kho phải nằm trên một dòng riêng."));
    }

    let account_lines = lines.collect::<Vec<_>>();
    if account_lines.is_empty() {
        return Some(Err(stock_import_usage()));
    }
    if account_lines.len() > MAX_ACCOUNT_LINES {
        return Some(Err("Mỗi lần chỉ nhập tối đa 250 tài khoản."));
    }
    if account_lines
        .iter()
        .any(|line| line.chars().count() > MAX_ACCOUNT_LINE_CHARS)
    {
        return Some(Err("Một dòng tài khoản vượt quá giới hạn cho phép."));
    }

    let accounts = account_lines.join("\n");
    if accounts.len() > MAX_REQUEST_BYTES {
        return Some(Err("Danh sách tài khoản quá lớn, hãy chia thành nhiều lần nhập."));
    }

    Some(Ok(StockImportRequest {
        code: code.to_string(),
        accounts,
        account_count: account_lines.len(),
    }))
}

fn command_matches(command: &str) -> bool {
    command == STOCK_IMPORT_COMMAND
        || command
            .strip_prefix(STOCK_IMPORT_COMMAND)
            .is_some_and(|suffix| suffix.starts_with('@') && suffix.len() > 1)
}

fn stock_import_usage() -> &'static str {
    "Cách dùng:\n/nhapkho <ma_kho>\nuid|pass|2fa\nuid|pass|2fa\n\nMỗi tài khoản một dòng."
}

fn limit_message(value: &str, max_chars: usize) -> String {
    let trimmed = value.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let shortened = trimmed.chars().take(max_chars).collect::<String>();
    format!("{shortened}...")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_stock_import_command() {
        let parsed = parse_stock_import_command("/nhapkho STOCK01\n100|pass|2fa\n200|pass|2fa")
            .unwrap()
            .unwrap();

        assert_eq!(parsed.code, "STOCK01");
        assert_eq!(parsed.accounts, "100|pass|2fa\n200|pass|2fa");
        assert_eq!(parsed.account_count, 2);
    }

    #[test]
    fn rejects_missing_stock_data() {
        assert_eq!(parse_stock_import_command("/nhapkho").unwrap(), Err(stock_import_usage()));
        assert_eq!(
            parse_stock_import_command("/nhapkho STOCK01").unwrap(),
            Err(stock_import_usage())
        );
    }

    #[test]
    fn only_matches_the_stock_import_command() {
        assert!(parse_stock_import_command("/nhapkho@caffemmo_bot STOCK01\n1|a").is_some());
        assert!(parse_stock_import_command("/nhapkhoo STOCK01\n1|a").is_none());
        assert!(parse_stock_import_command("/shop STOCK01\n1|a").is_none());
    }

    #[test]
    fn truncates_messages_without_breaking_unicode() {
        assert_eq!(limit_message("điện", 2), "đi...");
        assert_eq!(limit_message("ok", 10), "ok");
    }
}
