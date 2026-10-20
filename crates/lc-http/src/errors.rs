//! Mapping of transport failures onto `AppError`.

use std::error::Error as StdError;
use std::path::Path;
use std::time::Duration;

use lc_types::{AppError, ErrorCode};
use reqwest::StatusCode;
use url::Url;

/// Failure of one attempt, plus what the retry loop should do about it.
#[derive(Debug)]
pub(crate) struct AttemptError {
    pub err: AppError,
    /// Worth retrying the same URL (network glitch, 5xx, 429, bad hash).
    pub retry: bool,
    /// Server-provided delay (`Retry-After`).
    pub retry_after: Option<Duration>,
}

impl AttemptError {
    pub fn retryable(err: AppError) -> Self {
        Self {
            err,
            retry: true,
            retry_after: None,
        }
    }

    pub fn fatal(err: AppError) -> Self {
        Self {
            err,
            retry: false,
            retry_after: None,
        }
    }

    /// Errors that must abort the whole operation, not just this URL.
    pub fn is_terminal(&self) -> bool {
        matches!(self.err.code, ErrorCode::Cancelled | ErrorCode::Io)
    }
}

pub(crate) fn cancelled() -> AppError {
    AppError::new(ErrorCode::Cancelled, "cancelled by user")
}

pub(crate) fn io_err(e: std::io::Error, path: &Path) -> AppError {
    AppError::new(ErrorCode::Io, e.to_string()).detail("path", path.display().to_string())
}

/// Transport error → `Blocked` (reset / TLS failure) or `Network`.
pub(crate) fn from_reqwest(e: &reqwest::Error, url: &Url) -> AppError {
    let code = if looks_blocked(e) {
        ErrorCode::Blocked
    } else {
        ErrorCode::Network
    };
    AppError::new(code, format!("request failed: {}", chain_message(e))).detail("url", url.as_str())
}

/// HTTP status → attempt error. Only call for non-success statuses.
pub(crate) fn from_status(
    status: StatusCode,
    url: &Url,
    retry_after: Option<Duration>,
) -> AttemptError {
    let base = |code: ErrorCode| {
        AppError::new(code, format!("HTTP {status}"))
            .detail("url", url.as_str())
            .detail("status", status.as_u16().to_string())
    };
    match status {
        StatusCode::NOT_FOUND | StatusCode::GONE => AttemptError::fatal(base(ErrorCode::NotFound)),
        StatusCode::TOO_MANY_REQUESTS | StatusCode::REQUEST_TIMEOUT => AttemptError {
            err: base(ErrorCode::Network),
            retry: true,
            retry_after,
        },
        s if s.is_server_error() => AttemptError {
            err: base(ErrorCode::Network),
            retry: true,
            retry_after,
        },
        _ => AttemptError::fatal(base(ErrorCode::Network)),
    }
}

fn chain_message(e: &(dyn StdError + 'static)) -> String {
    let mut msg = e.to_string();
    let mut cur = e.source();
    while let Some(s) = cur {
        let part = s.to_string();
        if !msg.contains(&part) {
            msg.push_str(": ");
            msg.push_str(&part);
        }
        cur = s.source();
    }
    msg
}

/// Connection reset/abort or a TLS handshake failure: typical DPI symptoms.
/// Timeouts are deliberately *not* treated as blocking here.
fn looks_blocked(e: &reqwest::Error) -> bool {
    if e.is_timeout() {
        return false;
    }
    let mut cur: Option<&(dyn StdError + 'static)> = Some(e);
    while let Some(s) = cur {
        if let Some(io) = s.downcast_ref::<std::io::Error>() {
            if matches!(
                io.kind(),
                std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionAborted
            ) {
                return true;
            }
        }
        let text = s.to_string().to_ascii_lowercase();
        if [
            "tls",
            "handshake",
            "certificate",
            "connection reset",
            "forcibly closed",
        ]
        .iter()
        .any(|needle| text.contains(needle))
        {
            return true;
        }
        cur = s.source();
    }
    false
}

/// Parses `Retry-After` given in seconds; capped at 30 s.
pub(crate) fn retry_after(headers: &reqwest::header::HeaderMap) -> Option<Duration> {
    let secs: u64 = headers
        .get(reqwest::header::RETRY_AFTER)?
        .to_str()
        .ok()?
        .trim()
        .parse()
        .ok()?;
    Some(Duration::from_secs(secs.min(30)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        let url = Url::parse("http://x/y").unwrap();
        let e = from_status(StatusCode::NOT_FOUND, &url, None);
        assert_eq!(e.err.code, ErrorCode::NotFound);
        assert!(!e.retry);
        assert_eq!(
            e.err.details.as_ref().unwrap().get("url").unwrap(),
            "http://x/y"
        );
        assert!(from_status(StatusCode::BAD_GATEWAY, &url, None).retry);
        assert!(from_status(StatusCode::TOO_MANY_REQUESTS, &url, None).retry);
        assert!(!from_status(StatusCode::FORBIDDEN, &url, None).retry);
    }
}
