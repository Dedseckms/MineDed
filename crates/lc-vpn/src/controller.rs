//! Minimal client for mihomo's external-controller REST API.

use std::collections::HashMap;
use std::time::Duration;

use lc_types::{AppError, AppResult, ErrorCode};
use serde_json::Value;
use url::Url;

/// URL used for latency tests (tiny 204 response, served worldwide).
pub(crate) const DELAY_URL: &str = "https://www.gstatic.com/generate_204";
const DELAY_TIMEOUT_MS: u32 = 5000;

/// Controller of one running mihomo. The secret is never logged.
#[derive(Clone)]
pub(crate) struct Controller {
    client: reqwest::Client,
    base: Url,
    secret: String,
}

impl Controller {
    /// `client` must not use a proxy (loopback target).
    pub fn new(client: reqwest::Client, port: u16, secret: String) -> AppResult<Self> {
        let base = Url::parse(&format!("http://127.0.0.1:{port}/"))
            .map_err(|e| AppError::internal(format!("bad controller URL: {e}")))?;
        Ok(Self {
            client,
            base,
            secret,
        })
    }

    fn url(&self, segments: &[&str]) -> AppResult<Url> {
        let mut u = self.base.clone();
        u.path_segments_mut()
            .map_err(|()| AppError::internal("controller URL cannot be a base"))?
            .pop_if_empty()
            .extend(segments);
        Ok(u)
    }

    async fn send(&self, rb: reqwest::RequestBuilder) -> AppResult<reqwest::Response> {
        let resp = rb
            .bearer_auth(&self.secret)
            .timeout(Duration::from_secs(15))
            .send()
            .await
            .map_err(|e| {
                AppError::new(ErrorCode::Network, "mihomo controller unreachable")
                    .detail("reason", e.to_string())
            })?;
        let status = resp.status();
        if status.is_success() {
            Ok(resp)
        } else {
            let code = if status == reqwest::StatusCode::NOT_FOUND {
                ErrorCode::NotFound
            } else {
                ErrorCode::Internal
            };
            Err(AppError::new(code, "mihomo controller request failed")
                .detail("status", status.as_u16().to_string()))
        }
    }

    /// Succeeds once the controller answers (used as readiness probe).
    pub async fn version(&self) -> AppResult<String> {
        let resp = self
            .send(
                self.client
                    .get(self.url(&["version"])?)
                    .timeout(Duration::from_secs(2)),
            )
            .await?;
        let v: Value = resp.json().await.map_err(|e| {
            AppError::new(ErrorCode::Internal, "bad controller response")
                .detail("reason", e.to_string())
        })?;
        Ok(v.get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned())
    }

    /// Switches selector `group` to `node`.
    pub async fn select(&self, group: &str, node: &str) -> AppResult<()> {
        let body = serde_json::json!({ "name": node });
        self.send(self.client.put(self.url(&["proxies", group])?).json(&body))
            .await?;
        Ok(())
    }

    /// Latency of every node in `group` (ms); failed nodes are absent.
    pub async fn group_delay(&self, group: &str) -> AppResult<HashMap<String, u32>> {
        let mut u = self.url(&["group", group, "delay"])?;
        u.query_pairs_mut()
            .append_pair("url", DELAY_URL)
            .append_pair("timeout", &DELAY_TIMEOUT_MS.to_string());
        let resp = match self.send(self.client.get(u)).await {
            Ok(r) => r,
            // mihomo answers 504/503 when no node responded in time.
            Err(e)
                if e.details
                    .as_ref()
                    .and_then(|d| d.get("status"))
                    .is_some_and(|s| s == "504" || s == "503" || s == "408") =>
            {
                return Ok(HashMap::new())
            }
            Err(e) => return Err(e),
        };
        let v: HashMap<String, Value> = resp.json().await.map_err(|e| {
            AppError::new(ErrorCode::Internal, "bad controller response")
                .detail("reason", e.to_string())
        })?;
        Ok(v.into_iter()
            .filter_map(|(k, d)| {
                let ms = u32::try_from(d.as_u64()?).ok()?;
                (ms > 0).then_some((k, ms))
            })
            .collect())
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use axum::extract::{Path, Query};
    use axum::http::{HeaderMap, StatusCode};
    use axum::routing::{get, put};
    use axum::{Json, Router};

    use super::*;

    fn authed(h: &HeaderMap) -> bool {
        h.get("authorization").and_then(|v| v.to_str().ok()) == Some("Bearer sec")
    }

    #[tokio::test]
    async fn talks_to_controller_api() {
        let selected = Arc::new(Mutex::new(String::new()));
        let sel = selected.clone();
        let app = Router::new()
            .route(
                "/version",
                get(|h: HeaderMap| async move {
                    if authed(&h) {
                        Ok(Json(serde_json::json!({"version": "v1.19.33"})))
                    } else {
                        Err(StatusCode::UNAUTHORIZED)
                    }
                }),
            )
            .route(
                "/proxies/{group}",
                put(move |Path(group): Path<String>, Json(b): Json<Value>| {
                    let sel = sel.clone();
                    async move {
                        assert_eq!(group, "LC-PROXY");
                        *sel.lock().unwrap() = b["name"].as_str().unwrap().to_owned();
                        StatusCode::NO_CONTENT
                    }
                }),
            )
            .route(
                "/group/{group}/delay",
                get(|Query(q): Query<HashMap<String, String>>| async move {
                    assert_eq!(q["url"], DELAY_URL);
                    Json(serde_json::json!({"Нидерланды 🇳🇱": 120, "B": 0, "C": 45}))
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });

        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let c = Controller::new(client.clone(), port, "sec".into()).unwrap();
        assert_eq!(c.version().await.unwrap(), "v1.19.33");
        c.select("LC-PROXY", "Нидерланды 🇳🇱 / x").await.unwrap();
        assert_eq!(*selected.lock().unwrap(), "Нидерланды 🇳🇱 / x");
        let d = c.group_delay("LC-PROXY").await.unwrap();
        assert_eq!(d.len(), 2);
        assert_eq!(d["Нидерланды 🇳🇱"], 120);

        let wrong = Controller::new(client, port, "nope".into()).unwrap();
        assert!(wrong.version().await.is_err());
    }
}
