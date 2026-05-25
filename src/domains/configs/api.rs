use axum::{Json, Router, extract::State};
use std::collections::HashMap;
use std::sync::Arc;

use super::repo;
use crate::app::{
    AppContext, ORDER_MEMO_LENGTH_MAX, ORDER_MEMO_LENGTH_MIN, normalize_order_memo_prefix,
};
use crate::core::responses::{ApiError, ApiResult, ok};

pub async fn list_configs(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
) -> ApiResult<HashMap<String, String>> {
    let configs = repo::get_all_configs(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("failed to load configs: {e}")))?;
    Ok(ok(configs))
}

pub async fn update_configs(
    State(ctx): State<Arc<AppContext>>,
    _headers: axum::http::HeaderMap,
    Json(payload): Json<HashMap<String, String>>,
) -> ApiResult<()> {
    let payload = sanitize_config_payload(payload)?;

    repo::save_configs(&ctx.pool, &payload)
        .await
        .map_err(|e| ApiError::internal(format!("failed to save configs: {e}")))?;

    let new_configs = repo::get_all_configs(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("failed to reload configs: {e}")))?;

    ctx.update_configs(new_configs);

    Ok(ok(()))
}

pub fn router() -> Router<Arc<AppContext>> {
    Router::new().route(
        "/api/admin/configs",
        axum::routing::get(list_configs).post(update_configs),
    )
}

fn sanitize_config_payload(
    mut payload: HashMap<String, String>,
) -> Result<HashMap<String, String>, ApiError> {
    validate_config_payload(&payload)?;

    if let Some(prefix) = payload.get("order_memo_prefix").cloned() {
        if let Some(normalized) = normalize_order_memo_prefix(&prefix) {
            payload.insert("order_memo_prefix".to_string(), normalized);
        }
    }
    if let Some(length) = payload.get("order_memo_length").cloned() {
        payload.insert("order_memo_length".to_string(), length.trim().to_string());
    }

    Ok(payload)
}

fn validate_config_payload(payload: &HashMap<String, String>) -> Result<(), ApiError> {
    if let Some(prefix) = payload.get("order_memo_prefix") {
        let Some(normalized) = normalize_order_memo_prefix(prefix) else {
            return Err(ApiError::validation(
                "order_memo_prefix phải là chữ/số, dài 1-10 ký tự",
            ));
        };
        if "NAP".starts_with(&normalized) || normalized.starts_with("NAP") {
            return Err(ApiError::validation(
                "order_memo_prefix không được trùng prefix nạp ví NAP",
            ));
        }
    }

    if let Some(length) = payload.get("order_memo_length") {
        let value = length
            .trim()
            .parse::<usize>()
            .map_err(|_| ApiError::validation("order_memo_length phải là số từ 10 đến 16"))?;
        if !(ORDER_MEMO_LENGTH_MIN..=ORDER_MEMO_LENGTH_MAX).contains(&value) {
            return Err(ApiError::validation(
                "order_memo_length phải nằm trong khoảng 10 đến 16",
            ));
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_order_memo_config_before_saving() {
        let valid = HashMap::from([
            ("order_memo_prefix".to_string(), "SHOP".to_string()),
            ("order_memo_length".to_string(), "10".to_string()),
        ]);
        assert!(validate_config_payload(&valid).is_ok());

        let too_short = HashMap::from([("order_memo_length".to_string(), "9".to_string())]);
        assert!(validate_config_payload(&too_short).is_err());

        let too_long = HashMap::from([("order_memo_length".to_string(), "17".to_string())]);
        assert!(validate_config_payload(&too_long).is_err());

        let empty_prefix = HashMap::from([("order_memo_prefix".to_string(), "  ".to_string())]);
        assert!(validate_config_payload(&empty_prefix).is_err());

        let invalid_prefix =
            HashMap::from([("order_memo_prefix".to_string(), "SHOP-".to_string())]);
        assert!(validate_config_payload(&invalid_prefix).is_err());
    }
}
