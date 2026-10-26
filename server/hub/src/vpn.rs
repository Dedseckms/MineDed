//! MineDed VPN access (`/me/vpn`, hub.yaml) on top of the s-ui panel token API (`/apiv2`).
//!
//! One panel client per Yggdrasil profile. The client `remark` is the lookup key
//! (`mc-<undashed uuid>`); the client `name` is a random id because s-ui serves the
//! subscription at `<subPath>/<name>`, so the name is the subscription secret and must not
//! be derivable from the (public) profile UUID. s-ui quirks: failures are HTTP 200 with
//! `success:false`; monthly usage reset is the panel's global `globalReset` job.

use std::time::{Duration, SystemTime};

use axum::body::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{header, Method, Request, StatusCode};
use hyper_util::client::legacy::connect::HttpConnector;
use hyper_util::client::legacy::Client;
use hyper_util::rt::TokioExecutor;
use serde::Deserialize;
use serde_json::{json, Value};
use uuid::Uuid;

use crate::error::ApiError;
use crate::rfc3339;

const UPSTREAM_TIMEOUT: Duration = Duration::from_secs(10);
/// Panel inbounds every client gets: 1 = Hysteria2, 2 = VLESS Reality.
const INBOUNDS: [u32; 2] = [1, 2];
/// s-ui `timeLocation` is Europe/Moscow (UTC+3, no DST); `globalReset` = `0 0 1 * *`.
const RESET_UTC_OFFSET_SECS: i64 = 3 * 3600;

/// VPN backend settings (env `HUB_VPN_*`).
#[derive(Clone)]
pub struct VpnConfig {
    /// s-ui token API root, e.g. `http://mineded-vpn:2095/<panel path>/apiv2`.
    pub api_url: String,
    /// s-ui API token (secret; never logged).
    pub api_token: String,
    /// Public subscription base, e.g. `https://relay.example.ru/<sub path>`.
    pub sub_base: String,
    pub quota_bytes: u64,
}

impl std::fmt::Debug for VpnConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VpnConfig")
            .field("quota_bytes", &self.quota_bytes)
            .finish_non_exhaustive()
    }
}

/// Client summary from `GET clients`.
#[derive(Debug, Clone, Deserialize)]
struct PanelClient {
    id: u64,
    #[serde(default)]
    enable: bool,
    name: String,
    #[serde(default)]
    remark: String,
    #[serde(default)]
    volume: u64,
    #[serde(default)]
    up: u64,
    #[serde(default)]
    down: u64,
}

#[derive(Deserialize)]
struct Envelope {
    #[serde(default)]
    success: bool,
    #[serde(default)]
    msg: String,
    #[serde(default)]
    obj: Value,
}

pub struct VpnService {
    cfg: VpnConfig,
    client: Client<HttpConnector, Full<Bytes>>,
    /// Serializes provisioning so concurrent POSTs never create duplicate clients.
    provision: tokio::sync::Mutex<()>,
}

fn unavailable(message: &str) -> ApiError {
    ApiError::new(StatusCode::SERVICE_UNAVAILABLE, "network", message)
}

/// `mc-<undashed lowercase uuid>`, or `None` when the identity is not a profile UUID.
pub fn client_key(user_id: &str) -> Option<String> {
    let id = Uuid::parse_str(user_id).ok()?;
    Some(format!("mc-{}", id.simple()))
}

impl VpnService {
    pub fn new(mut cfg: VpnConfig) -> Self {
        cfg.api_url = cfg.api_url.trim_end_matches('/').to_owned();
        cfg.sub_base = cfg.sub_base.trim_end_matches('/').to_owned();
        Self {
            cfg,
            client: Client::builder(TokioExecutor::new()).build_http(),
            provision: tokio::sync::Mutex::new(()),
        }
    }

    async fn call(
        &self,
        method: Method,
        path: &str,
        form: Option<String>,
    ) -> Result<Value, ApiError> {
        let mut req = Request::builder()
            .method(method)
            .uri(format!("{}/{path}", self.cfg.api_url))
            .header("Token", &self.cfg.api_token);
        if form.is_some() {
            req = req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded");
        }
        let req = req
            .body(Full::new(Bytes::from(form.unwrap_or_default())))
            .map_err(|_| unavailable("bad vpn backend request"))?;
        let fut = async {
            let res = self.client.request(req).await?;
            let status = res.status();
            let bytes = res.into_body().collect().await?.to_bytes();
            Ok::<_, Box<dyn std::error::Error + Send + Sync>>((status, bytes))
        };
        let (status, bytes) = match tokio::time::timeout(UPSTREAM_TIMEOUT, fut).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => return Err(unavailable("vpn backend unreachable")),
            Err(_) => return Err(unavailable("vpn backend timeout")),
        };
        if !status.is_success() {
            return Err(unavailable("vpn backend error").detail("status", status.as_str()));
        }
        let env: Envelope = serde_json::from_slice(&bytes)
            .map_err(|_| unavailable("vpn backend returned invalid JSON"))?;
        if !env.success {
            return Err(unavailable("vpn backend rejected the request").detail("msg", env.msg));
        }
        Ok(env.obj)
    }

    async fn find(&self, key: &str) -> Result<Option<PanelClient>, ApiError> {
        let obj = self.call(Method::GET, "clients", None).await?;
        let list = obj.get("clients").cloned().unwrap_or(Value::Null);
        let list: Vec<PanelClient> = if list.is_null() {
            Vec::new()
        } else {
            serde_json::from_value(list)
                .map_err(|_| unavailable("vpn backend returned unexpected clients"))?
        };
        Ok(list
            .into_iter()
            .filter(|c| c.remark == key)
            .min_by_key(|c| c.id))
    }

    async fn create(&self, key: &str) -> Result<(), ApiError> {
        let name = format!("u{}", Uuid::new_v4().simple());
        let data = json!({
            "enable": true,
            "name": name,
            "remark": key,
            "desc": "",
            "group": "",
            "inbounds": INBOUNDS,
            "volume": self.cfg.quota_bytes,
            "expiry": 0,
            "up": 0,
            "down": 0,
            "links": [],
            "config": {
                "vless": { "name": name, "uuid": Uuid::new_v4(), "flow": "xtls-rprx-vision" },
                "hysteria2": { "name": name, "password": Uuid::new_v4().simple().to_string() },
            },
        });
        let form = form_urlencoded::Serializer::new(String::new())
            .append_pair("object", "clients")
            .append_pair("action", "new")
            .append_pair("data", &data.to_string())
            .finish();
        self.call(Method::POST, "save", Some(form)).await.map(drop)
    }

    fn access(&self, c: &PanelClient, now: SystemTime) -> Value {
        let used = c.up.saturating_add(c.down);
        let enabled = c.enable && (c.volume == 0 || used < c.volume);
        json!({
            "subscriptionUrl": format!("{}/{}?format=clash", self.cfg.sub_base, c.name),
            "quotaBytes": c.volume,
            "usedBytes": used,
            "resetAt": rfc3339::format(rfc3339::next_month_start(now, RESET_UTC_OFFSET_SECS)),
            "enabled": enabled,
        })
    }

    /// The `VpnAccess` of `key`, `None` if no client exists.
    pub async fn get(&self, key: &str) -> Result<Option<Value>, ApiError> {
        Ok(self
            .find(key)
            .await?
            .map(|c| self.access(&c, SystemTime::now())))
    }

    /// Idempotent: returns the existing client's access or creates one.
    pub async fn provision(&self, key: &str) -> Result<Value, ApiError> {
        let _guard = self.provision.lock().await;
        if let Some(c) = self.find(key).await? {
            return Ok(self.access(&c, SystemTime::now()));
        }
        self.create(key).await?;
        let c = self
            .find(key)
            .await?
            .ok_or_else(|| unavailable("vpn client was not created"))?;
        Ok(self.access(&c, SystemTime::now()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn key_from_profile_uuid() {
        assert_eq!(
            client_key("0000000000000000000000000000000A").as_deref(),
            Some("mc-0000000000000000000000000000000a")
        );
        assert_eq!(
            client_key("00000000-0000-0000-0000-00000000000b").as_deref(),
            Some("mc-0000000000000000000000000000000b")
        );
        assert_eq!(client_key("token:abcdef"), None);
    }
}
