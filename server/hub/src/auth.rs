//! Bearer validation against Drasl (Yggdrasil `/authserver/validate`), plus a
//! best-effort profile lookup (`/minecraftservices/minecraft/profile`) for the
//! caller's id and name. Results are cached briefly to spare Drasl.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use axum::body::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{header, Method, Request, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;

use crate::error::ApiError;

/// Authenticated caller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    /// Stable user key (profile UUID; falls back to a token-derived key).
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

/// Drasl client. `api_root` is the authlib-injector API root, e.g.
/// `http://mineup-skin-drasl:25585/authlib-injector` (plain HTTP, internal network).
pub struct DraslValidator {
    api_root: String,
    client: Client<HttpConnector, Full<Bytes>>,
    cache: Mutex<HashMap<String, (Instant, Option<Identity>)>>,
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
            .map_err(|e| ApiError::upstream("bad drasl request").detail("error", e.to_string()))?;
        let fut = async {
            let res = self.client.request(req).await?;
            let status = res.status();
            let bytes = res.into_body().collect().await?.to_bytes();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((status, bytes))
        };
        match tokio::time::timeout(UPSTREAM_TIMEOUT, fut).await {
            Ok(Ok(v)) => Ok(v),
            Ok(Err(e)) => {
                Err(ApiError::upstream("drasl unreachable").detail("error", e.to_string()))
            }
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
        Ok(Some(match serde_json::from_slice::<Profile>(&bytes) {
            Ok(p) => Identity {
                user_id: p.id,
                name: p.name,
            },
            Err(_) => Identity {
                user_id: format!("token:{}", short_key(token)),
                name: "Player".into(),
            },
        }))
    }
}

fn short_key(token: &str) -> String {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    token.hash(&mut h);
    format!("{:016x}", h.finish())
}

#[async_trait]
impl TokenValidator for DraslValidator {
    async fn validate(&self, token: &str) -> Result<Option<Identity>, ApiError> {
        let now = Instant::now();
        {
            let cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
            if let Some((exp, id)) = cache.get(token) {
                if *exp > now {
                    return Ok(id.clone());
                }
            }
        }
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
        cache.insert(token.to_owned(), (now + ttl, id.clone()));
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
                        Some("Bearer good") => (StatusCode::OK, r#"{"id":"abc","name":"Steve"}"#),
                        Some("Bearer noprofile") => (StatusCode::OK, "not json"),
                        _ => (StatusCode::UNAUTHORIZED, "{}"),
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
                user_id: "abc".into(),
                name: "Steve".into()
            }
        );
        assert_eq!(v.validate("bad").await.unwrap(), None);
        let id = v.validate("noprofile").await.unwrap().unwrap();
        assert!(id.user_id.starts_with("token:"));
        // Cached.
        assert_eq!(v.validate("good").await.unwrap().unwrap().name, "Steve");
    }

    #[tokio::test]
    async fn unreachable_drasl_is_upstream_error() {
        let v = DraslValidator::new("http://127.0.0.1:1/authlib-injector");
        let err = v.validate("x").await.unwrap_err();
        assert_eq!(err.status, StatusCode::BAD_GATEWAY);
    }
}
