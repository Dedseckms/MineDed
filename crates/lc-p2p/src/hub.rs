//! Minimal client for the rooms part of the hub API (`server/openapi/hub.yaml`).

use lc_accounts::Secret;
use lc_types::room::{InstanceSig, RoomAuthMode};
use lc_types::{AppError, AppResult, ErrorCode, RoomId};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateRoomReq {
    pub endpoint_id: String,
    pub sig: InstanceSig,
    pub auth_mode: RoomAuthMode,
    pub max_peers: u32,
    pub password_protected: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CreateRoomResp {
    pub room_id: RoomId,
    pub code: String,
    pub host_token: String,
    pub expires_at: String,
    #[serde(default)]
    pub relay_urls: Vec<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct JoinRoomReq<'a> {
    endpoint_id: &'a str,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct JoinRoomResp {
    pub room_id: RoomId,
    pub peer_token: String,
    pub host_endpoint_id: String,
    pub sig: InstanceSig,
    pub auth_mode: RoomAuthMode,
    #[serde(default)]
    pub relay_urls: Vec<String>,
    #[serde(default)]
    pub password_protected: bool,
}

/// Error body shared by hub and launcher (`{code, message, details}`).
#[derive(Debug, Deserialize)]
struct HubError {
    code: Option<String>,
    message: Option<String>,
    #[serde(default)]
    details: Option<std::collections::BTreeMap<String, String>>,
}

#[derive(Clone)]
pub(crate) struct HubClient {
    client: reqwest::Client,
    base: String,
}

impl HubClient {
    /// `base` is `Settings::hub_url`, e.g. `https://mc.example.ru/api/v1`.
    pub fn new(client: reqwest::Client, base: &str) -> AppResult<Self> {
        let base = base.trim().trim_end_matches('/').to_owned();
        let ok = url::Url::parse(&base)
            .map(|u| matches!(u.scheme(), "http" | "https"))
            .unwrap_or(false);
        if !ok {
            return Err(
                AppError::new(ErrorCode::InvalidInput, "hub url is not configured")
                    .detail("setting", "hubUrl"),
            );
        }
        Ok(Self { client, base })
    }

    pub async fn create_room(
        &self,
        req: &CreateRoomReq,
        bearer: Option<&Secret>,
    ) -> AppResult<CreateRoomResp> {
        let url = format!("{}/rooms", self.base);
        let mut rb = self.client.post(&url).json(req);
        if let Some(token) = bearer {
            rb = rb.bearer_auth(token.expose());
        }
        self.send_json(rb, &url).await
    }

    pub async fn join_room(
        &self,
        code: &str,
        endpoint_id: &str,
        bearer: Option<&Secret>,
    ) -> AppResult<JoinRoomResp> {
        // `code` is canonical (`[A-Z0-9]{4}-[A-Z0-9]{4}`), safe in a path.
        let url = format!("{}/rooms/{code}/join", self.base);
        let mut rb = self.client.post(&url).json(&JoinRoomReq { endpoint_id });
        if let Some(token) = bearer {
            rb = rb.bearer_auth(token.expose());
        }
        self.send_json(rb, &url).await
    }

    /// Best effort: the room also expires on its own.
    pub async fn close_room(&self, room_id: RoomId, host_token: &Secret) -> AppResult<()> {
        let url = format!("{}/rooms/{room_id}", self.base);
        let resp = self
            .client
            .delete(&url)
            .bearer_auth(host_token.expose())
            .send()
            .await
            .map_err(|e| network(&url, &e))?;
        if resp.status().is_success() {
            Ok(())
        } else {
            Err(status_error(resp, &url).await)
        }
    }

    async fn send_json<T: for<'de> Deserialize<'de>>(
        &self,
        rb: reqwest::RequestBuilder,
        url: &str,
    ) -> AppResult<T> {
        let resp = rb.send().await.map_err(|e| network(url, &e))?;
        if !resp.status().is_success() {
            return Err(status_error(resp, url).await);
        }
        let bytes = resp.bytes().await.map_err(|e| network(url, &e))?;
        serde_json::from_slice(&bytes).map_err(|e| {
            AppError::new(ErrorCode::Network, format!("unexpected hub response: {e}"))
                .detail("url", url)
        })
    }
}

fn network(url: &str, e: &reqwest::Error) -> AppError {
    // reqwest errors carry the URL only, never headers (no token leak).
    AppError::new(ErrorCode::Network, format!("hub request failed: {e}")).detail("url", url)
}

async fn status_error(resp: reqwest::Response, url: &str) -> AppError {
    let status = resp.status();
    let body: Option<HubError> = resp
        .bytes()
        .await
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok());
    let code = match status.as_u16() {
        401 => ErrorCode::AuthRequired,
        403 => ErrorCode::PermissionDenied,
        404 => ErrorCode::NotFound,
        409 => ErrorCode::Conflict,
        400 | 422 => ErrorCode::InvalidInput,
        // Hub reports rate limits as permissionDenied + reason=rateLimited.
        429 => ErrorCode::PermissionDenied,
        _ => ErrorCode::Network,
    };
    let message = body
        .as_ref()
        .and_then(|b| b.message.clone())
        .unwrap_or_else(|| format!("hub returned {status}"));
    let mut err = AppError::new(code, message)
        .detail("url", url)
        .detail("status", status.as_u16().to_string());
    let (hub_code, details) = body.map(|b| (b.code, b.details)).unwrap_or_default();
    if let Some(hub_code) = hub_code {
        err = err.detail("hubCode", hub_code);
    }
    let reason = details
        .and_then(|mut d| d.remove("reason"))
        .or_else(|| (status.as_u16() == 429).then(|| "rateLimited".to_owned()));
    if let Some(reason) = reason {
        err = err.detail("reason", reason);
    }
    err
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_empty_hub_url() {
        use lc_http::{Downloader, Route, StubDownloader};
        let c = StubDownloader.client(Route::Direct);
        assert_eq!(
            HubClient::new(c.clone(), "").err().map(|e| e.code),
            Some(ErrorCode::InvalidInput)
        );
        assert!(HubClient::new(c, "https://hub.test/api/v1/").is_ok());
    }

    #[tokio::test]
    async fn rate_limit_maps_to_permission_denied() {
        use axum::http::StatusCode;
        use axum::routing::post;
        use lc_http::{Downloader, Route, StubDownloader};

        let app = axum::Router::new().route(
            "/api/v1/rooms/{code}/join",
            post(|| async {
                (
                    StatusCode::TOO_MANY_REQUESTS,
                    axum::Json(serde_json::json!({
                        "code": "permissionDenied",
                        "message": "slow down",
                        "details": {"reason": "rateLimited"}
                    })),
                )
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        let hub = HubClient::new(
            StubDownloader.client(Route::Direct),
            &format!("http://{addr}/api/v1"),
        )
        .unwrap();
        let err = hub.join_room("ABCD-1234", "e", None).await.unwrap_err();
        assert_eq!(err.code, ErrorCode::PermissionDenied);
        let d = err.details.unwrap();
        assert_eq!(d["reason"], "rateLimited");
        assert_eq!(d["status"], "429");
    }

    #[test]
    fn create_request_is_camel_case() {
        let req = CreateRoomReq {
            endpoint_id: "e".into(),
            sig: InstanceSig {
                mc: "1.21.1".into(),
                loader: lc_types::version::LoaderKind::NeoForge,
                loader_version: None,
                mods_hash: "h".into(),
            },
            auth_mode: RoomAuthMode::Open,
            max_peers: 8,
            password_protected: true,
        };
        let v = serde_json::to_value(&req).unwrap();
        assert_eq!(v["endpointId"], "e");
        assert_eq!(v["authMode"], "open");
        assert_eq!(v["passwordProtected"], true);
        assert_eq!(v["sig"]["loader"], "neoForge");
        assert_eq!(v["sig"]["modsHash"], "h");
    }
}
