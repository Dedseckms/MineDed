//! Hub `/me/vpn`: personal MineDed VPN access. The bearer token and the
//! subscription URL are secrets: never logged, never in error messages.

use std::time::Duration;

use lc_types::vpn::VpnAccess;
use lc_types::{AppError, AppResult, ErrorCode};
use serde::Deserialize;
use url::Url;

const HUB_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BODY: usize = 64 * 1024;

/// Wire format (`components/schemas/VpnAccess`). Deliberately not `Debug`.
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    subscription_url: String,
    quota_bytes: u64,
    used_bytes: u64,
    #[serde(default)]
    reset_at: Option<String>,
    enabled: bool,
}

/// Access plus the personal subscription URL (kept inside lc-vpn).
pub(crate) struct Access {
    pub info: VpnAccess,
    pub subscription_url: String,
}

/// `{base}/me/vpn` from `Settings::hub_url` (empty → default hub).
pub(crate) fn endpoint(hub_url: &str) -> AppResult<Url> {
    let base = match hub_url.trim() {
        "" => lc_types::settings::DEFAULT_HUB_URL,
        b => b,
    };
    let base = base.trim_end_matches('/');
    Url::parse(&format!("{base}/me/vpn"))
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"))
        .ok_or_else(|| {
            AppError::new(ErrorCode::InvalidInput, "hub url is not configured")
                .detail("setting", "hubUrl")
        })
}

fn hub_err(code: ErrorCode, message: &str) -> AppError {
    AppError::new(code, message).detail("source", "hub")
}

/// GET (or POST when `provision`) `/me/vpn`. `Ok(None)`: no key and no
/// provisioning requested.
pub(crate) async fn fetch(
    client: &reqwest::Client,
    url: &Url,
    bearer: &str,
    provision: bool,
) -> AppResult<Option<Access>> {
    let rb = if provision {
        client.post(url.clone())
    } else {
        client.get(url.clone())
    };
    let net = |e: reqwest::Error| {
        hub_err(ErrorCode::Network, "hub is unreachable")
            .detail("reason", e.without_url().to_string())
    };
    let resp = rb
        .bearer_auth(bearer)
        .header(reqwest::header::ACCEPT, "application/json")
        .timeout(HUB_TIMEOUT)
        .send()
        .await
        .map_err(net)?;
    let status = resp.status().as_u16();
    match status {
        200..=299 => {}
        401 | 403 => {
            return Err(hub_err(
                ErrorCode::AuthExpired,
                "own-server session expired; log in again",
            ))
        }
        404 if !provision => return Ok(None),
        404 => return Err(hub_err(ErrorCode::NotFound, "MineDed VPN is not available")),
        429 => {
            let mut e = hub_err(ErrorCode::RateLimited, "too many VPN requests; try later");
            if let Some(v) = resp
                .headers()
                .get(reqwest::header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok())
            {
                e = e.detail("retryAfter", v);
            }
            return Err(e);
        }
        503 => {
            return Err(
                hub_err(ErrorCode::Network, "MineDed VPN is temporarily unavailable")
                    .detail("reason", "vpnUnavailable"),
            )
        }
        s => {
            return Err(
                hub_err(ErrorCode::Network, "hub request failed").detail("status", s.to_string())
            )
        }
    }
    let bad = || hub_err(ErrorCode::Network, "unexpected hub response");
    let body = crate::response::read_limited(resp, MAX_BODY, bad, net).await?;
    let w: Wire = serde_json::from_slice(&body).map_err(|_| bad())?;
    let ok = Url::parse(&w.subscription_url)
        .map(|u| matches!(u.scheme(), "http" | "https"))
        .unwrap_or(false);
    if !ok {
        return Err(bad());
    }
    Ok(Some(Access {
        info: VpnAccess {
            quota_bytes: w.quota_bytes,
            used_bytes: w.used_bytes,
            reset_at: w.reset_at,
            enabled: w.enabled,
        },
        subscription_url: w.subscription_url,
    }))
}

#[cfg(test)]
pub(crate) mod tests {
    use std::sync::{Arc, Mutex};

    use axum::http::{HeaderMap, Method, StatusCode};
    use axum::response::IntoResponse;
    use axum::routing::any;
    use axum::Router;

    use super::*;

    /// Fake hub: `token` → behaviour. Records `(method, auth)` per call.
    pub(crate) async fn spawn_hub(sub_url: String) -> (String, Arc<Mutex<Vec<String>>>) {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let rec = calls.clone();
        let app = Router::new().route(
            "/api/v1/me/vpn",
            any(move |m: Method, h: HeaderMap| {
                let rec = rec.clone();
                let sub_url = sub_url.clone();
                async move {
                    let auth = h
                        .get("authorization")
                        .and_then(|v| v.to_str().ok())
                        .unwrap_or_default()
                        .to_owned();
                    rec.lock().unwrap().push(format!("{m} {auth}"));
                    let body = serde_json::json!({
                        "subscriptionUrl": sub_url,
                        "quotaBytes": 274877906944u64,
                        "usedBytes": 1024,
                        "resetAt": "2026-11-01T00:00:00Z",
                        "enabled": true,
                    });
                    match (auth.as_str(), m == Method::POST) {
                        ("Bearer good", _) | ("Bearer new", true) => {
                            axum::Json(body).into_response()
                        }
                        ("Bearer new", false) => StatusCode::NOT_FOUND.into_response(),
                        ("Bearer busy", _) => {
                            (StatusCode::TOO_MANY_REQUESTS, [("retry-after", "30")]).into_response()
                        }
                        ("Bearer down", _) => StatusCode::SERVICE_UNAVAILABLE.into_response(),
                        ("Bearer junk", _) => axum::Json(serde_json::json!({
                            "subscriptionUrl": "file:///etc/passwd",
                            "quotaBytes": 1, "usedBytes": 0, "enabled": true
                        }))
                        .into_response(),
                        _ => StatusCode::UNAUTHORIZED.into_response(),
                    }
                }
            }),
        );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        (format!("http://127.0.0.1:{port}/api/v1"), calls)
    }

    #[test]
    fn endpoint_defaults_and_validates() {
        assert_eq!(
            endpoint("").unwrap().as_str(),
            format!("{}/me/vpn", lc_types::settings::DEFAULT_HUB_URL)
        );
        assert_eq!(
            endpoint(" https://h.example/api/v1/ ").unwrap().as_str(),
            "https://h.example/api/v1/me/vpn"
        );
        assert_eq!(
            endpoint("ftp://x").unwrap_err().code,
            ErrorCode::InvalidInput
        );
    }

    #[tokio::test]
    async fn rejects_chunked_access_before_body_finishes() {
        let (url, server) = crate::response::tests::oversized_chunked(MAX_BODY).await;
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let result = tokio::time::timeout(
            Duration::from_secs(3),
            fetch(&client, &url, "test-bearer", false),
        )
        .await;
        server.abort();
        let err = result
            .expect("must reject without waiting for EOF")
            .err()
            .unwrap();
        assert_eq!(err.code, ErrorCode::Network);
        assert!(!format!("{err:?}").contains("secret-subscription"));
        assert!(!format!("{err:?}").contains("test-bearer"));
    }

    #[tokio::test]
    async fn maps_hub_answers() {
        let (base, calls) = spawn_hub("https://sub.example/s/tok".into()).await;
        let url = endpoint(&base).unwrap();
        let c = reqwest::Client::builder().no_proxy().build().unwrap();

        let a = fetch(&c, &url, "good", false).await.unwrap().unwrap();
        assert_eq!(a.subscription_url, "https://sub.example/s/tok");
        assert_eq!(a.info.quota_bytes, 274877906944);
        assert_eq!(a.info.reset_at.as_deref(), Some("2026-11-01T00:00:00Z"));
        assert!(a.info.enabled);

        assert!(fetch(&c, &url, "new", false).await.unwrap().is_none());
        assert!(fetch(&c, &url, "new", true).await.unwrap().is_some());
        let code = |t: &'static str| {
            let (c, url) = (c.clone(), url.clone());
            async move { fetch(&c, &url, t, true).await.err().unwrap() }
        };
        assert_eq!(code("expired").await.code, ErrorCode::AuthExpired);
        let busy = code("busy").await;
        assert_eq!(busy.code, ErrorCode::RateLimited);
        assert_eq!(busy.details.unwrap()["retryAfter"], "30");
        let down = code("down").await;
        assert_eq!(down.code, ErrorCode::Network);
        assert_eq!(down.details.unwrap()["reason"], "vpnUnavailable");
        let junk = code("junk").await;
        assert!(!format!("{junk:?}").contains("passwd"));

        let calls = calls.lock().unwrap();
        assert_eq!(calls[0], "GET Bearer good");
        assert_eq!(calls[2], "POST Bearer new");
    }
}
