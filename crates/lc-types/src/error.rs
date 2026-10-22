use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use specta::Type;

/// Machine-readable error class. The UI maps it to an i18n key `errors.<code>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub enum ErrorCode {
    /// Generic network failure (timeout, DNS, reset).
    Network,
    /// Host looks blocked (RST/TLS failure/DPI); UI may suggest enabling VPN.
    Blocked,
    HashMismatch,
    NotFound,
    AuthExpired,
    AuthRequired,
    JavaMissing,
    LoaderInstall,
    /// CurseForge file with `allowModDistribution=false`; `details` holds the URL.
    ManualDownloadRequired,
    Cancelled,
    Io,
    InvalidInput,
    Conflict,
    PermissionDenied,
    /// Too many requests / attempts; `details.retryAfter` (seconds) when known.
    RateLimited,
    NotImplemented,
    Internal,
}

/// The only error type that crosses crate and IPC boundaries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type, thiserror::Error)]
#[serde(rename_all = "camelCase")]
#[error("{code:?}: {message}")]
pub struct AppError {
    pub code: ErrorCode,
    /// Developer-facing message (English). Not shown verbatim to users.
    pub message: String,
    /// Optional context: `url`, `path`, `host`, `expected`, `actual`...
    pub details: Option<BTreeMap<String, String>>,
}

pub type AppResult<T> = Result<T, AppError>;

impl AppError {
    pub fn new(code: ErrorCode, message: impl Into<String>) -> Self {
        Self {
            code,
            message: message.into(),
            details: None,
        }
    }

    pub fn detail(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.details
            .get_or_insert_with(BTreeMap::new)
            .insert(key.into(), value.into());
        self
    }

    /// Returned by every stub until the owning role implements it.
    pub fn not_implemented(what: &str) -> Self {
        Self::new(
            ErrorCode::NotImplemented,
            format!("{what} is not implemented yet"),
        )
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self::new(ErrorCode::Internal, message)
    }
}

impl From<std::io::Error> for AppError {
    fn from(e: std::io::Error) -> Self {
        Self::new(ErrorCode::Io, e.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn serializes_camel_case() {
        let e = AppError::not_implemented("x");
        let v = serde_json::to_value(&e).unwrap();
        assert_eq!(v["code"], "notImplemented");
        assert!(v["details"].is_null());

        let e = e.detail("url", "https://x");
        assert_eq!(
            serde_json::to_value(&e).unwrap()["details"]["url"],
            "https://x"
        );
    }
}
