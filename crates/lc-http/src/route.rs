//! `Route::Auto` building blocks: per-host "proxy first" memory and
//! detection of spoofed responses (HTML block pages instead of data).

use std::collections::HashMap;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use lc_types::{AppError, ErrorCode};
use url::Url;

use crate::contract::ResourceKind;

/// Hosts that recently needed the proxy; they try the proxy first until the
/// entry expires.
#[derive(Debug)]
pub(crate) struct HostMemory {
    ttl: Duration,
    until: Mutex<HashMap<String, Instant>>,
}

impl HostMemory {
    pub fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            until: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_proxy_first(&self, url: &Url) -> bool {
        self.contains(url)
    }

    /// Whether url's host has a live (unexpired) entry.
    pub fn contains(&self, url: &Url) -> bool {
        let Some(key) = host_key(url) else {
            return false;
        };
        let mut map = self.until.lock().unwrap_or_else(PoisonError::into_inner);
        match map.get(&key) {
            Some(&t) if t > Instant::now() => true,
            Some(_) => {
                map.remove(&key);
                false
            }
            None => false,
        }
    }

    pub fn remember(&self, url: &Url) {
        if let Some(key) = self.insert(url) {
            tracing::info!(host = %key, ttl_secs = self.ttl.as_secs(), "host remembered as proxy-first");
        }
    }

    /// Adds (or refreshes) url's host; returns its key.
    pub fn insert(&self, url: &Url) -> Option<String> {
        let key = host_key(url)?;
        self.until
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(key.clone(), Instant::now() + self.ttl);
        Some(key)
    }

    pub fn forget(&self, url: &Url) {
        if let Some(key) = host_key(url) {
            self.until
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&key);
        }
    }
}

/// `host:port` (lowercase host); `None` for URLs without a host.
fn host_key(url: &Url) -> Option<String> {
    let host = url.host_str()?.to_ascii_lowercase();
    Some(format!(
        "{host}:{}",
        url.port_or_known_default().unwrap_or_default()
    ))
}

/// Kinds whose payload is JSON or binary, never an HTML page.
pub(crate) fn expects_data(kind: ResourceKind) -> bool {
    !matches!(kind, ResourceKind::Other)
}

/// HTML instead of JSON/binary: a captive portal or an ISP block page.
pub(crate) fn looks_like_block_page(content_type: Option<&str>, body_start: &[u8]) -> bool {
    if content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("text/html")) {
        return true;
    }
    let start = body_start
        .iter()
        .position(|b| !b.is_ascii_whitespace())
        .map_or(&[][..], |i| &body_start[i..]);
    let start = start.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(start);
    let head = &start[..start.len().min(16)];
    let head = head.to_ascii_lowercase();
    head.starts_with(b"<!doctype") || head.starts_with(b"<html")
}

pub(crate) fn spoofed(url: &Url) -> AppError {
    AppError::new(
        ErrorCode::Blocked,
        "response looks like a block page (HTML instead of data)",
    )
    .detail("url", url.as_str())
}

/// Failures after which `Route::Auto` retries the same URL via the proxy.
pub(crate) fn worth_proxy(err: &AppError) -> bool {
    matches!(
        err.code,
        ErrorCode::Network | ErrorCode::Blocked | ErrorCode::HashMismatch
    )
}

/// Error for `Route::Proxied` when no proxy is configured.
pub(crate) fn no_proxy_configured() -> AppError {
    AppError::new(
        ErrorCode::Network,
        "proxy route requested but no proxy is configured (set a proxy in settings)",
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn block_page_detection() {
        assert!(looks_like_block_page(
            Some("text/html; charset=utf-8"),
            b"{}"
        ));
        assert!(looks_like_block_page(None, b"  \n<!DOCTYPE html><p>"));
        assert!(looks_like_block_page(None, b"<HTML><body>"));
        assert!(looks_like_block_page(None, b"\xEF\xBB\xBF<html>"));
        assert!(!looks_like_block_page(
            Some("application/json"),
            b"{\"a\":1}"
        ));
        assert!(!looks_like_block_page(None, b"PK\x03\x04"));
        assert!(!looks_like_block_page(None, b""));
        assert!(expects_data(ResourceKind::VersionMeta));
        assert!(expects_data(ResourceKind::Content));
        assert!(!expects_data(ResourceKind::Other));
    }

    #[test]
    fn memory_expires_and_is_per_host() {
        let m = HostMemory::new(Duration::from_millis(30));
        let a = u("https://API.modrinth.com/v2/x");
        m.remember(&a);
        assert!(m.is_proxy_first(&u("https://api.modrinth.com/other")));
        assert!(!m.is_proxy_first(&u("http://api.modrinth.com/other")));
        assert!(!m.is_proxy_first(&u("https://cdn.modrinth.com/x")));
        std::thread::sleep(Duration::from_millis(40));
        assert!(!m.is_proxy_first(&a));
        m.remember(&a);
        m.forget(&a);
        assert!(!m.is_proxy_first(&a));
    }

    #[test]
    fn fallback_codes() {
        assert!(worth_proxy(&spoofed(&u("http://x/"))));
        assert!(worth_proxy(&AppError::new(ErrorCode::HashMismatch, "x")));
        assert!(!worth_proxy(&AppError::new(ErrorCode::NotFound, "x")));
        assert!(!worth_proxy(&AppError::new(ErrorCode::Cancelled, "x")));
    }
}
