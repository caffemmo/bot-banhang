use std::sync::Arc;

use argon2::{
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
    password_hash::{SaltString, rand_core::OsRng},
};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State},
    http::{HeaderMap, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use chrono::Utc;
use jsonwebtoken::{DecodingKey, EncodingKey, Header, Validation, decode, encode};
use serde::{Deserialize, Serialize};

use crate::app::AppContext;
use crate::core::responses::{ApiError, ApiResult, ok};
use crate::domains::auth::models::{AdminUser, AdminUserPublic};
use crate::domains::auth::repo;

const ADMIN_SESSION_COOKIE: &str = "admin_session";
const SESSION_TTL_SECONDS: i64 = 8 * 60 * 60;

#[derive(Debug, Serialize, Deserialize, Clone)]
struct AdminClaims {
    sub: i64,
    username: String,
    exp: usize,
    iat: usize,
}

#[derive(Debug, Deserialize)]
pub struct SetupPayload {
    pub username: String,
    pub password: String,
    pub setup_code: String,
}

#[derive(Debug, Deserialize)]
pub struct LoginPayload {
    pub username: String,
    pub password: String,
}

#[derive(Debug, Deserialize)]
pub struct CreateAdminPayload {
    pub username: String,
    pub password: String,
    pub setup_code: String,
}

#[derive(Debug, Deserialize)]
pub struct ChangePasswordPayload {
    pub password: String,
    pub setup_code: String,
}

#[derive(Debug, Serialize)]
pub struct SetupStatusResponse {
    pub needs_setup: bool,
}

#[derive(Debug, Serialize)]
pub struct AuthUserResponse {
    pub user: AdminUserPublic,
}

#[derive(Debug, Serialize)]
pub struct LoginResponse {
    pub user: AdminUserPublic,
    pub expires_at: i64,
}

pub async fn require_admin_session(
    State(ctx): State<Arc<AppContext>>,
    req: Request<Body>,
    next: Next,
) -> Response {
    let Some(claims) = decode_session_cookie(&ctx, req.headers()) else {
        return ApiError::unauthorized().into_response();
    };

    match repo::get_admin_user(&ctx.pool, claims.sub).await {
        Ok(Some(user)) if user.is_active == 1 && user.username == claims.username => {
            next.run(req).await
        }
        _ => ApiError::unauthorized().into_response(),
    }
}

pub async fn setup_status(State(ctx): State<Arc<AppContext>>) -> ApiResult<SetupStatusResponse> {
    let count = repo::count_admin_users(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("count admin users failed: {e}")))?;
    Ok(ok(SetupStatusResponse {
        needs_setup: count == 0,
    }))
}

pub async fn setup_admin(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<SetupPayload>,
) -> Result<JsonWithHeader<LoginResponse>, ApiError> {
    validate_setup_code(&ctx, &payload.setup_code)?;
    validate_username_password(&payload.username, &payload.password)?;

    let count = repo::count_admin_users(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("count admin users failed: {e}")))?;
    if count > 0 {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "SETUP_LOCKED",
            "admin setup is already completed",
        ));
    }

    let password_hash = hash_password(&payload.password)?;
    let user = repo::insert_admin_user(&ctx.pool, payload.username.trim(), &password_hash)
        .await
        .map_err(|e| ApiError::internal(format!("create admin failed: {e}")))?;
    repo::touch_last_login(&ctx.pool, user.id)
        .await
        .map_err(|e| ApiError::internal(format!("update login failed: {e}")))?;
    login_response(&ctx, user)
}

pub async fn login(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<LoginPayload>,
) -> Result<JsonWithHeader<LoginResponse>, ApiError> {
    let username = payload.username.trim();
    let Some(user) = repo::get_admin_by_username(&ctx.pool, username)
        .await
        .map_err(|e| ApiError::internal(format!("load admin failed: {e}")))?
    else {
        return Err(ApiError::unauthorized());
    };

    if user.is_active != 1 || !verify_password(&payload.password, &user.password_hash)? {
        return Err(ApiError::unauthorized());
    }

    repo::touch_last_login(&ctx.pool, user.id)
        .await
        .map_err(|e| ApiError::internal(format!("update login failed: {e}")))?;
    login_response(&ctx, user)
}

pub async fn logout(State(ctx): State<Arc<AppContext>>) -> impl IntoResponse {
    (
        [(
            header::SET_COOKIE,
            clear_session_cookie(ctx.config.admin_cookie_secure),
        )],
        Json(serde_json::json!({"ok": true, "data": {"success": true}})),
    )
}

pub async fn me(
    State(ctx): State<Arc<AppContext>>,
    headers: HeaderMap,
) -> ApiResult<AuthUserResponse> {
    let Some(claims) = decode_session_cookie(&ctx, &headers) else {
        return Err(ApiError::unauthorized());
    };
    let Some(user) = repo::get_admin_user(&ctx.pool, claims.sub)
        .await
        .map_err(|e| ApiError::internal(format!("load admin failed: {e}")))?
    else {
        return Err(ApiError::unauthorized());
    };
    if user.is_active != 1 {
        return Err(ApiError::unauthorized());
    }
    Ok(ok(AuthUserResponse { user: user.into() }))
}

pub async fn list_admins(State(ctx): State<Arc<AppContext>>) -> ApiResult<Vec<AdminUserPublic>> {
    let users = repo::list_admin_users(&ctx.pool)
        .await
        .map_err(|e| ApiError::internal(format!("list admins failed: {e}")))?;
    Ok(ok(users.into_iter().map(AdminUserPublic::from).collect()))
}

pub async fn create_admin(
    State(ctx): State<Arc<AppContext>>,
    Json(payload): Json<CreateAdminPayload>,
) -> ApiResult<AdminUserPublic> {
    validate_setup_code(&ctx, &payload.setup_code)?;
    validate_username_password(&payload.username, &payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let user = repo::insert_admin_user(&ctx.pool, payload.username.trim(), &password_hash)
        .await
        .map_err(|e| ApiError::internal(format!("create admin failed: {e}")))?;
    Ok(ok(user.into()))
}

pub async fn change_admin_password(
    State(ctx): State<Arc<AppContext>>,
    Path(id): Path<i64>,
    Json(payload): Json<ChangePasswordPayload>,
) -> ApiResult<AdminUserPublic> {
    validate_setup_code(&ctx, &payload.setup_code)?;
    validate_password(&payload.password)?;

    let password_hash = hash_password(&payload.password)?;
    let Some(user) = repo::update_password_hash(&ctx.pool, id, &password_hash)
        .await
        .map_err(|e| ApiError::internal(format!("change password failed: {e}")))?
    else {
        return Err(ApiError::not_found("admin user not found"));
    };
    Ok(ok(user.into()))
}

fn login_response(
    ctx: &AppContext,
    user: AdminUser,
) -> Result<JsonWithHeader<LoginResponse>, ApiError> {
    let expires_at = Utc::now().timestamp() + SESSION_TTL_SECONDS;
    let token = encode_session_token(ctx, &user, expires_at)?;
    let cookie = session_cookie(&token, ctx.config.admin_cookie_secure);
    let body = LoginResponse {
        user: user.into(),
        expires_at,
    };
    Ok(Json(crate::core::responses::ApiSuccess {
        ok: true,
        data: body,
    })
    .with_header(header::SET_COOKIE, cookie))
}

trait WithHeader<T> {
    fn with_header(self, name: header::HeaderName, value: String) -> JsonWithHeader<T>;
}

impl<T> WithHeader<T> for Json<crate::core::responses::ApiSuccess<T>> {
    fn with_header(self, name: header::HeaderName, value: String) -> JsonWithHeader<T> {
        JsonWithHeader {
            response: self,
            name,
            value,
        }
    }
}

pub struct JsonWithHeader<T> {
    response: Json<crate::core::responses::ApiSuccess<T>>,
    name: header::HeaderName,
    value: String,
}

impl<T: Serialize> IntoResponse for JsonWithHeader<T> {
    fn into_response(self) -> Response {
        let mut response = self.response.into_response();
        response.headers_mut().insert(
            self.name,
            self.value.parse().unwrap_or_else(|_| {
                "admin_session=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0"
                    .parse()
                    .unwrap()
            }),
        );
        response
    }
}

fn hash_password(password: &str) -> Result<String, ApiError> {
    let salt = SaltString::generate(&mut OsRng);
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|e| ApiError::internal(format!("hash password failed: {e}")))
}

fn verify_password(password: &str, password_hash: &str) -> Result<bool, ApiError> {
    let parsed = PasswordHash::new(password_hash)
        .map_err(|e| ApiError::internal(format!("parse password hash failed: {e}")))?;
    Ok(Argon2::default()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

fn encode_session_token(
    ctx: &AppContext,
    user: &AdminUser,
    expires_at: i64,
) -> Result<String, ApiError> {
    let claims = AdminClaims {
        sub: user.id,
        username: user.username.clone(),
        exp: expires_at as usize,
        iat: Utc::now().timestamp() as usize,
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(ctx.config.admin_jwt_secret.as_bytes()),
    )
    .map_err(|e| ApiError::internal(format!("encode session failed: {e}")))
}

fn decode_session_cookie(ctx: &AppContext, headers: &HeaderMap) -> Option<AdminClaims> {
    let token = session_cookie_value(headers)?;
    decode::<AdminClaims>(
        &token,
        &DecodingKey::from_secret(ctx.config.admin_jwt_secret.as_bytes()),
        &Validation::default(),
    )
    .ok()
    .map(|data| data.claims)
}

fn session_cookie_value(headers: &HeaderMap) -> Option<String> {
    headers
        .get(header::COOKIE)
        .and_then(|v| v.to_str().ok())
        .and_then(|raw| {
            raw.split(';').find_map(|part| {
                let (name, value) = part.trim().split_once('=')?;
                if name == ADMIN_SESSION_COOKIE {
                    Some(value.to_string())
                } else {
                    None
                }
            })
        })
}

fn session_cookie(token: &str, secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!(
        "{ADMIN_SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={SESSION_TTL_SECONDS}{secure}"
    )
}

fn clear_session_cookie(secure: bool) -> String {
    let secure = if secure { "; Secure" } else { "" };
    format!("{ADMIN_SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0{secure}")
}

fn validate_setup_code(ctx: &AppContext, setup_code: &str) -> Result<(), ApiError> {
    if setup_code.trim() != ctx.config.admin_setup_code {
        return Err(ApiError::new(
            StatusCode::FORBIDDEN,
            "INVALID_SETUP_CODE",
            "invalid setup code",
        ));
    }
    Ok(())
}

fn validate_username_password(username: &str, password: &str) -> Result<(), ApiError> {
    let username = username.trim();
    if username.len() < 3 || username.len() > 64 {
        return Err(ApiError::validation("username must be 3..64 chars"));
    }
    if !username
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.'))
    {
        return Err(ApiError::validation(
            "username may contain letters, numbers, dot, dash, underscore",
        ));
    }
    validate_password(password)
}

fn validate_password(password: &str) -> Result<(), ApiError> {
    if password.len() < 8 || password.len() > 256 {
        return Err(ApiError::validation("password must be 8..256 chars"));
    }
    Ok(())
}

pub fn router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route("/api/auth/setup-status", get(setup_status))
        .route("/api/auth/setup", post(setup_admin))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/me", get(me))
}

pub fn admin_router() -> Router<Arc<crate::app::AppContext>> {
    Router::new()
        .route("/api/admin/users", get(list_admins).post(create_admin))
        .route("/api/admin/users/:id/password", put(change_admin_password))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::Body,
        http::{Request, StatusCode, header},
    };
    use serde_json::json;
    use sqlx::{SqlitePool, sqlite::SqlitePoolOptions};
    use std::sync::{Arc, RwLock};
    use teloxide::Bot;
    use tower::ServiceExt;

    async fn test_pool() -> SqlitePool {
        let pool = SqlitePoolOptions::new()
            .max_connections(1)
            .connect("sqlite::memory:")
            .await
            .unwrap();
        sqlx::migrate!("./migrations").run(&pool).await.unwrap();
        pool
    }

    fn test_config() -> crate::config::Config {
        crate::config::Config {
            telegram_token: "TEST".into(),
            database_url: "sqlite::memory:".into(),
            bank_name: "VCB".into(),
            bank_account: Some("0000".into()),
            bank_account_name: None,
            webhook_secret: "webhook".into(),
            admin_jwt_secret: "test-secret-that-is-long-enough-for-hmac".into(),
            admin_setup_code: "SETUP-123".into(),
            admin_cookie_secure: false,
            base_url: None,
            i18n_dir: "i18n".to_string(),
            port: 0,
            crypto: crate::config::CryptoConfig::default(),
        }
    }

    async fn test_context(pool: SqlitePool) -> Arc<crate::app::AppContext> {
        Arc::new(crate::app::AppContext {
            bot: Bot::new("TEST"),
            pool,
            config: test_config(),
            configs: Arc::new(RwLock::new(std::collections::HashMap::new())),
            texts: Arc::new(RwLock::new(crate::bot::texts::BotTexts::default())),
            plugins: Arc::new(vec![]),
            usdt_rate_cache: Arc::new(tokio::sync::RwLock::new(None)),
        })
    }

    async fn app(ctx: Arc<crate::app::AppContext>) -> Router {
        let protected = Router::new()
            .route(
                "/api/admin/protected",
                get(|| async { axum::Json(json!({"ok": true})) }),
            )
            .route_layer(axum::middleware::from_fn_with_state(
                ctx.clone(),
                require_admin_session,
            ));

        Router::new()
            .merge(router())
            .merge(protected)
            .with_state(ctx)
    }

    #[tokio::test]
    async fn setup_creates_first_admin_and_then_locks() {
        let ctx = test_context(test_pool().await).await;
        let app = app(ctx).await;

        let first = app
            .clone()
            .oneshot(
                Request::post("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "username": "root",
                            "password": "very-secret-pass",
                            "setup_code": "SETUP-123"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(first.status(), StatusCode::OK);

        let second = app
            .oneshot(
                Request::post("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "username": "other",
                            "password": "very-secret-pass",
                            "setup_code": "SETUP-123"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(second.status(), StatusCode::FORBIDDEN);
    }

    #[tokio::test]
    async fn login_sets_http_only_session_cookie_and_cookie_authorizes_admin_route() {
        let ctx = test_context(test_pool().await).await;
        let app = app(ctx).await;

        let setup = app
            .clone()
            .oneshot(
                Request::post("/api/auth/setup")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "username": "root",
                            "password": "very-secret-pass",
                            "setup_code": "SETUP-123"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(setup.status(), StatusCode::OK);

        let login = app
            .clone()
            .oneshot(
                Request::post("/api/auth/login")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        json!({
                            "username": "root",
                            "password": "very-secret-pass"
                        })
                        .to_string(),
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(login.status(), StatusCode::OK);
        let cookie = login
            .headers()
            .get(header::SET_COOKIE)
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        assert!(cookie.contains("admin_session="));
        assert!(cookie.contains("HttpOnly"));
        assert!(cookie.contains("SameSite=Strict"));

        let admin_ok = app
            .clone()
            .oneshot(
                Request::get("/api/admin/protected")
                    .header(header::COOKIE, cookie)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(admin_ok.status(), StatusCode::OK);

        let admin_missing_cookie = app
            .oneshot(
                Request::get("/api/admin/protected")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(admin_missing_cookie.status(), StatusCode::UNAUTHORIZED);
    }
}
