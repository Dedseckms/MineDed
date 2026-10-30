//! Bearer validation through Drasl's authenticated profile endpoint.
//! Only a valid profile grants an identity; results are cached briefly.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{header, Method, Request, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use tokio::sync::Semaphore;
use uuid::Uuid;

use crate::error::ApiError;

/// Authenticated caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Stable user key (profile UUID).
    pub user_id: String,
    pub name: String,
}

/// Validates Yggdrasil access tokens.
#[async_trait]
pub trait TokenValidator: Send + Sync {
    /// `Ok(Some)` = valid, `Ok(None)` = invalid/expired, `Err` = Drasl unreachable.
    async fn validate(&self, token: &str) -> Result<Option<Identity>, ApiError>;
}

const POSITIVE_TTL: Duration = Duration::from_secs(60);
const NEGATIVE_TTL: Duration = Duration::from_secs(10);
const MAX_CACHE: usize = 10_000;
const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROFILE_BYTES: usize = 16 * 1024;
pub(crate) const MAX_BEARER_BYTES: usize = 4096;
type TokenCache = HashMap<[u8; 32], (Instant, Option<Identity>)>;

/// Drasl client. `api_root` is the authlib-injector API root, e.g.
/// `http://mineup-skin-drasl:25585/authlib-injector` (plain HTTP, internal network).
pub struct DraslValidator {
    api_root: String,
    client: Client<HttpConnector, Full<Bytes>>,
    cache: Mutex<TokenCache>,
    requests: Semaphore,
}

#[derive(Deserialize)]
struct Profile {
    id: String,
    name: String,
}

impl DraslValidator {
    pub fn new(api_root: &str) -> Self {
        Self {
            api_root: api_root.trim_end_matches('/').to_owned(),
            client: Client::builder(TokioExecutor::new()).build_http(),
            cache: Mutex::new(HashMap::new()),
            requests: Semaphore::new(16),
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        token: &str,
        body: Option<String>,
    ) -> Result<(StatusCode, Bytes), ApiError> {
        let mut req = Request::builder()
            .method(method)
            .uri(format!("{}{path}", self.api_root));
        if body.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/json");
        } else {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let req = req
            .body(Full::new(Bytes::from(body.unwrap_or_default())))
            .map_err(|_| ApiError::upstream("bad drasl request"))?;
        let fut = async {
            let res = self.client.request(req).await?;
            let status = res.status();
            let bytes = Limited::new(res.into_body(), MAX_PROFILE_BYTES)
                .collect()
                .await?
                .to_bytes();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((status, bytes))
        };
        match tokio::time::timeout(UPSTREAM_TIMEOUT, fut).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(_)) => Err(ApiError::upstream("drasl response unavailable")),
            Err(_) => Err(ApiError::upstream("drasl timeout")),
        }
    }

    async fn lookup(&self, token: &str) -> Result<Option<Identity>, ApiError> {
        // Drasl's `/authserver/validate` needs the launcher's clientToken, which
        // the hub never sees; the bearer-authenticated profile endpoint validates
        // the token and yields the identity in one call.
        let (status, bytes) = self
            .call(
                Method::GET,
                "/minecraftservices/minecraft/profile",
                token,
                None,
            )
            .await?;
        if status.is_server_error() {
            return Err(ApiError::upstream("drasl error").detail("status", status.as_str()));
        }
        if status != StatusCode::OK {
            return Ok(None);
        }
        let p: Profile = serde_json::from_slice(&bytes)
            .map_err(|_| ApiError::upstream("drasl returned an invalid profile"))?;
        let id = Uuid::parse_str(&p.id)
            .ok()
            .filter(|id| !id.is_nil())
            .ok_or_else(|| ApiError::upstream("drasl returned an invalid profile"))?;
        if p.name.is_empty() || p.name.chars().count() > 16 || p.name.chars().any(char::is_control)
        {
            return Err(ApiError::upstream("drasl returned an invalid profile"));
        }
        Ok(Some(Identity {
            user_id: id.simple().to_string(),
            name: p.name,
        }))
    }
}

#[async_trait]
impl TokenValidator for DraslValidator {
    async fn validate(&self, token: &str) -> Result<Option<Identity>, ApiError> {
        if token.is_empty() || token.len() > MAX_BEARER_BYTES {
            return Ok(None);
        }
        // The cache does not need to retain bearer credentials.
        let key: [u8; 32] = Sha256::digest(token.as_bytes()).into();
        let now = Instant::now();
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((exp, id)) = cache.get(&key) {
                if *exp > now {
                    return Ok(id.clone());
                }
            }
        }
        let _permit = self
            .requests
            .try_acquire()
            .map_err(|_| ApiError::upstream("drasl validation busy"))?;
        let id = self.lookup(token).await?;
        let ttl = if id.is_some() {
            POSITIVE_TTL
        } else {
            NEGATIVE_TTL
        };
        let mut cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        if cache.len() >= MAX_CACHE {
            cache.retain(|_, (exp, _)| *exp > now);
            if cache.len() >= MAX_CACHE {
                cache.clear();
            }
        }
        cache.insert(key, (Instant::now() + ttl, id.clone()));
        Ok(id)
    }
}

#[cfg(test)]
mod tests {
    use axum::http::HeaderMap;
    use axum::routing::{get, post};
    use axum::{Json, Router};

    use super::*;

    async fn fake_drasl() -> String {
        let app = Router::new()
            .route(
                "/authlib-injector/authserver/validate",
                post(|Json(v): Json<serde_json::Value>| async move {
                    if v["accessToken"] == "good" || v["accessToken"] == "noprofile" {
                        StatusCode::NO_CONTENT
                    } else {
                        StatusCode::FORBIDDEN
                    }
                }),
            )
            .route(
                "/authlib-injector/minecraftservices/minecraft/profile",
                get(|h: HeaderMap| async move {
                    match h.get("authorization").and_then(|v| v.to_str().ok()) {
                        Some("Bearer good") => (
                            StatusCode::OK,
                            r#"{"id":"069a79f4-44e9-4726-a5be-fca90e38aaf5","name":"Steve"}"#
                                .to_owned(),
                        ),
                        Some("Bearer noprofile") => (StatusCode::OK, "not json".to_owned()),
                        Some("Bearer badid") => (
                            StatusCode::OK,
                            r#"{"id":"not-a-profile","name":"Steve"}"#.to_owned(),
                        ),
                        Some("Bearer badname") => (
                            StatusCode::OK,
                            r#"{"id":"069a79f444e94726a5befca90e38aaf5","name":""}"#.to_owned(),
                        ),
                        Some("Bearer oversized") => {
                            (StatusCode::OK, " ".repeat(MAX_PROFILE_BYTES + 1))
                        }
                        _ => (StatusCode::UNAUTHORIZED, "{}".to_owned()),
                    }
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}/authlib-injector")
    }

    #[tokio::test]
    async fn validates_against_drasl() {
        let v = DraslValidator::new(&fake_drasl().await);
        let id = v.validate("good").await.unwrap().unwrap();
        assert_eq!(
            id,
            Identity {
                user_id: "069a79f444e94726a5befca90e38aaf5".into(),
                name: "Steve".into()
            }
        );
        assert_eq!(v.validate("bad").await.unwrap(), None);
        // Cached.
        assert_eq!(v.validate("good").await.unwrap().unwrap().name, "Steve");
    }

    #[tokio::test]
    async fn invalid_or_oversized_profiles_never_authenticate() {
        let v = DraslValidator::new(&fake_drasl().await);
        for token in ["noprofile", "badid", "badname", "oversized"] {
            let err = v.validate(token).await.unwrap_err();
            assert_eq!(err.status, StatusCode::BAD_GATEWAY);
            assert!(err.details.is_none());
        }
        assert!(v.cache.lock().unwrap().is_empty());
        assert_eq!(
            v.validate(&"x".repeat(MAX_BEARER_BYTES + 1)).await.unwrap(),
            None
        );
    }

    #[tokio::test]
    async fn concurrent_validation_has_a_bound() {
        let v = DraslValidator::new("http://127.0.0.1:1/authlib-injector");
        let _permits = v.requests.acquire_many(16).await.unwrap();
        let err = v.validate("some-token").await.unwrap_err();
        assert_eq!(err.message, "drasl validation busy");
    }

    #[tokio::test]
    async fn unreachable_drasl_is_upstream_error() {
        let v = DraslValidator::new("http://127.0.0.1:1/authlib-injector");
        let err = v.validate("x").await.unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_GATEWAY);
    }
}
