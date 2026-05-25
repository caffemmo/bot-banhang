use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct ApiSuccess<T> {
    pub ok: bool,
    pub data: T,
}

#[derive(Debug, Serialize)]
pub struct ErrorBody {
    code: String,
    message: String,
}

#[derive(Debug, Serialize)]
pub struct ApiErrorResponse {
    ok: bool,
    error: ErrorBody,
}

#[derive(Debug)]
pub struct ApiError {
    pub(crate) status: StatusCode,
    pub(crate) code: &'static str,
    pub(crate) message: String,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }

    #[allow(dead_code)]
    pub fn validation(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "VALIDATION_ERROR", message)
    }

    #[allow(dead_code)]
    #[allow(dead_code)]
    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "NOT_FOUND", message)
    }

    #[allow(dead_code)]
    pub fn unauthorized() -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "UNAUTHORIZED", "unauthorized")
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(StatusCode::INTERNAL_SERVER_ERROR, "INTERNAL_ERROR", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = ApiErrorResponse {
            ok: false,
            error: ErrorBody {
                code: self.code.to_string(),
                message: self.message,
            },
        };
        (self.status, Json(body)).into_response()
    }
}

pub type ApiResult<T> = std::result::Result<Json<ApiSuccess<T>>, ApiError>;

pub fn ok<T>(data: T) -> Json<ApiSuccess<T>> {
    Json(ApiSuccess { ok: true, data })
}

#[derive(Debug, Serialize)]
pub struct PaginatedResponse<T> {
    pub items: Vec<T>,
    pub limit: i64,
    pub offset: i64,
    pub total: i64,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct Ack {
    pub success: bool,
}

#[allow(dead_code)]
#[derive(Debug, Serialize)]
pub struct MessageResponse {
    pub ok: bool,
    pub message: String,
}
