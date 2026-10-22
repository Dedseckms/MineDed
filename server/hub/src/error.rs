//! HTTP errors in the launcher `AppError` shape: `{code, message, details}`.
//! `code` strings match `lc_types::ErrorCode` (camelCase).

use std::collections::BTreeMap;

use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

#[derive(Debug, Clone)]
pub struct ApiError {
    pub status: StatusCode,
    pub code: &'static str,
    pub message: String,
    pub details: Option<BTreeMap<String, String>>,
    retry_after: Option<u64>,
}

impl ApiError {
    pub fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
            details: None,
            retry_after: None,
        }
    }

    pub fn detail(mut self, key: &str, value: impl Into<String>) -> Self {
        self.details
            .get_or_insert_with(BTreeMap::new)
            .insert(key.to_owned(), value.into());
        self
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalidInput", message)
    }

    pub fn auth_required() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "authRequired",
            "bearer token required",
        )
    }

    pub fn auth_expired() -> Self {
        Self::new(
            StatusCode::UNAUTHORIZED,
            "authExpired",
            "token is invalid or expired",
        )
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "notFound", message)
    }

    /// HTTP 429, code `rateLimited`, with `details.retryAfter` and a `Retry-After` header.
    pub fn rate_limited(retry_after_secs: u64) -> Self {
        let mut e = Self::new(StatusCode::TOO_MANY_REQUESTS, "rateLimited", "rate limited")
            .detail("retryAfter", retry_after_secs.to_string());
        e.retry_after = Some(retry_after_secs);
        e
    }

    pub fn upstream(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_GATEWAY, "network", message)
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "code": self.code, "message": self.message, "details": self.details });
        let mut res = (self.status, Json(body)).into_response();
        if let Some(secs) = self.retry_after {
            if let Ok(v) = HeaderValue::from_str(&secs.to_string()) {
                res.headers_mut().insert(header::RETRY_AFTER, v);
            }
        }
        res
    }
}
