//! Restricted Minecraft catalogue. Upstream contract: https://docs.curseforge.com/rest-api/.

use std::collections::{BTreeMap, VecDeque};
use std::net::IpAddr;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use axum::extract::{OriginalUri, State};
use axum::http::{HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use lc_http::{Downloader, DownloaderConfig, HttpDownloader, Route, SystemProxyMode};
use lc_types::Secret;
use serde_json::{json, Value};
use tokio::sync::Semaphore;

use crate::error::ApiError;
use crate::ratelimit::RateLimiter;
use crate::{AppState, ClientIp};

const PREFIX: &str = "/api/v1/catalog/curseforge";
const UPSTREAM: &str = "https://api.curseforge.com/v1";
const BODY_LIMIT: usize = 4 * 1024 * 1024;
const CACHE_BYTES: usize = 16 * 1024 * 1024;
const CACHE_ENTRIES: usize = 64;
const TTL: Duration = Duration::from_secs(60);
const TIMEOUT: Duration = Duration::from_secs(15);

/// Server-side broker; credentials and transport never implement Debug.
pub struct Broker {
    key: Secret,
    client: reqwest::Client,
    upstream: String,
    limit: RateLimiter<IpAddr>,
    concurrent: Semaphore,
    cache: Mutex<Cache>,
}

#[derive(Default)]
struct Cache {
    entries: VecDeque<(String, Instant, Vec<u8>)>,
    bytes: usize,
}

impl Cache {
    fn sweep(&mut self) {
        self.entries.retain(|(_, time, _)| time.elapsed() < TTL);
        self.bytes = self.entries.iter().map(|(_, _, body)| body.len()).sum();
    }

    fn get(&mut self, key: &str) -> Option<Vec<u8>> {
        self.sweep();
        self.entries
            .iter()
            .find(|(k, _, _)| k == key)
            .map(|(_, _, b)| b.clone())
    }

    fn put(&mut self, key: String, body: Vec<u8>) {
        self.sweep();
        self.entries.retain(|(k, _, _)| k != &key);
        self.bytes = self.entries.iter().map(|(_, _, b)| b.len()).sum();
        if body.len() > BODY_LIMIT {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + body.len() > CACHE_BYTES {
            if let Some((_, _, old)) = self.entries.pop_front() {
                self.bytes -= old.len();
            } else {
                break;
            }
        }
        self.bytes += body.len();
        self.entries.push_back((key, Instant::now(), body));
    }
}

#[derive(Debug, PartialEq, Eq)]
enum Endpoint {
    BatchMods(Vec<u32>),
    BatchFiles(Vec<u32>),
    Search,
    Project(u32),
    Files(u32),
    File(u32, u32),
    Resolve(u32),
    Download(u32, u32),
}

fn invalid() -> ApiError {
    ApiError::invalid_input("unsupported catalogue request")
}
fn failed() -> ApiError {
    ApiError::upstream("catalogue upstream unavailable")
}
fn missing() -> ApiError {
    ApiError::not_found("Minecraft project or file unavailable")
}

fn id(value: &str) -> Result<u32, ApiError> {
    if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err(invalid());
    }
    value
        .parse::<u32>()
        .ok()
        .filter(|v| (1..=i32::MAX as u32).contains(v))
        .ok_or_else(invalid)
}

fn parse(path: &str, raw: Option<&str>) -> Result<(Endpoint, BTreeMap<String, String>), ApiError> {
    if path.len() > 256 {
        return Err(invalid());
    }
    let endpoint = match path.split('/').collect::<Vec<_>>().as_slice() {
        ["", "mods", "search"] => Endpoint::Search,
        ["", "mods", m] => Endpoint::Project(id(m)?),
        ["", "mods", m, "files"] => Endpoint::Files(id(m)?),
        ["", "mods", m, "files", f] => Endpoint::File(id(m)?, id(f)?),
        ["", "mods", m, "files", f, "download-url"] => Endpoint::Download(id(m)?, id(f)?),
        ["", "files", f] => Endpoint::Resolve(id(f)?),
        _ => return Err(invalid()),
    };
    let mut query = BTreeMap::new();
    if raw.is_some_and(|q| q.len() > 4096) {
        return Err(invalid());
    }
    for (key, value) in form_urlencoded::parse(raw.unwrap_or_default().as_bytes()) {
        if query.contains_key(key.as_ref()) {
            return Err(invalid());
        }
        let allowed = match endpoint {
            Endpoint::Search => matches!(
                key.as_ref(),
                "gameId"
                    | "searchFilter"
                    | "slug"
                    | "classId"
                    | "categoryId"
                    | "sortField"
                    | "sortOrder"
                    | "gameVersion"
                    | "modLoaderType"
                    | "index"
                    | "pageSize"
            ),
            Endpoint::Files(_) => matches!(
                key.as_ref(),
                "gameVersion" | "modLoaderType" | "index" | "pageSize"
            ),
            _ => false,
        };
        if !allowed || value.chars().any(char::is_control) {
            return Err(invalid());
        }
        let number = || {
            value
                .parse::<u32>()
                .ok()
                .filter(|_| value.bytes().all(|b| b.is_ascii_digit()))
        };
        let valid = match key.as_ref() {
            "gameId" => number() == Some(432),
            "modLoaderType" => number().is_some_and(|n| [0, 1, 4, 5, 6].contains(&n)),
            "index" => number().is_some_and(|n| n <= 9950),
            "pageSize" => number().is_some_and(|n| (1..=50).contains(&n)),
            "classId" | "categoryId" => {
                number().is_some_and(|n| (1..=i32::MAX as u32).contains(&n))
            }
            "sortField" => number().is_some_and(|n| (1..=14).contains(&n)),
            "sortOrder" => matches!(value.as_ref(), "asc" | "desc"),
            "gameVersion" => value.chars().count() <= 64,
            "slug" => value.chars().count() <= 128,
            "searchFilter" => value.chars().count() <= 256,
            _ => false,
        };
        if !valid {
            return Err(invalid());
        }
        query.insert(key.into_owned(), value.into_owned());
    }
    if endpoint == Endpoint::Search {
        query.insert("gameId".into(), "432".into());
    }
    Ok((endpoint, query))
}

fn batch(path: &str, body: &[u8]) -> Result<Endpoint, ApiError> {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Mods {
        #[serde(rename = "modIds")]
        ids: Vec<u32>,
    }
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Files {
        #[serde(rename = "fileIds")]
        ids: Vec<u32>,
    }
    let values = match path {
        "/mods" => {
            serde_json::from_slice::<Mods>(body)
                .map_err(|_| invalid())?
                .ids
        }
        "/files" => {
            serde_json::from_slice::<Files>(body)
                .map_err(|_| invalid())?
                .ids
        }
        _ => return Err(invalid()),
    };
    if values.is_empty() || values.len() > 50 {
        return Err(invalid());
    }
    let mut ids = Vec::new();
    for n in values {
        if !(1..=i32::MAX as u32).contains(&n) || ids.contains(&n) {
            return Err(invalid());
        }
        ids.push(n);
    }
    Ok(if path == "/mods" {
        Endpoint::BatchMods(ids)
    } else {
        Endpoint::BatchFiles(ids)
    })
}

impl Broker {
    /// Creates a direct, redirect-free transport; no default key or configurable upstream.
    pub fn new(key: Secret) -> Result<Self, ApiError> {
        if key.expose().is_empty() || HeaderValue::from_str(key.expose()).is_err() {
            return Err(invalid());
        }
        let downloader = HttpDownloader::new_no_redirects(DownloaderConfig {
            connect_timeout: TIMEOUT,
            read_timeout: TIMEOUT,
            request_timeout: TIMEOUT,
            system_proxy: SystemProxyMode::Off,
            use_bmclapi: false,
            ..DownloaderConfig::default()
        })
        .map_err(|_| failed())?;
        Ok(Self {
            key,
            client: downloader.client(Route::Direct),
            upstream: UPSTREAM.into(),
            limit: RateLimiter::per_minute(60, 60),
            concurrent: Semaphore::new(8),
            cache: Mutex::new(Cache::default()),
        })
    }

    pub(crate) fn sweep(&self) {
        self.limit.sweep();
        self.cache.lock().unwrap_or_else(|e| e.into_inner()).sweep();
    }

    async fn upstream(
        &self,
        path: &str,
        query: &BTreeMap<String, String>,
        payload: Option<&Value>,
    ) -> Result<Value, ApiError> {
        let mut key = HeaderValue::from_str(self.key.expose()).map_err(|_| failed())?;
        key.set_sensitive(true);
        let url = format!("{}{path}", self.upstream);
        let request = if let Some(payload) = payload {
            self.client.post(url).json(payload)
        } else {
            let mut encoded = form_urlencoded::Serializer::new(String::new());
            encoded.extend_pairs(query);
            let encoded = encoded.finish();
            self.client.get(if encoded.is_empty() {
                url
            } else {
                format!("{url}?{encoded}")
            })
        };
        let mut response = request
            .header("x-api-key", key)
            .header("accept", "application/json")
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|_| failed())?;
        if response.status() == StatusCode::NOT_FOUND {
            return Err(missing());
        }
        if response.status() != StatusCode::OK
            || response
                .content_length()
                .is_some_and(|n| n > BODY_LIMIT as u64)
        {
            return Err(failed());
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await.map_err(|_| failed())? {
            if chunk.len() > BODY_LIMIT.saturating_sub(bytes.len()) {
                return Err(failed());
            }
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes).map_err(|_| failed())?;
        if contains_secret(&value, self.key.expose()) {
            return Err(failed());
        }
        let data = value.get("data").ok_or_else(failed)?;
        let mut envelope = json!({"data": data});
        if let Some(pagination) = value.get("pagination") {
            if !pagination.is_object() {
                return Err(failed());
            }
            envelope["pagination"] = pagination.clone();
        }
        Ok(envelope)
    }

    async fn project(&self, project: u32) -> Result<Value, ApiError> {
        let mut value = self
            .upstream(&format!("/mods/{project}"), &BTreeMap::new(), None)
            .await?;
        validate_project(&mut value["data"], Some(project))?;
        Ok(value)
    }

    async fn projects(&self, ids: &[u32]) -> Result<Value, ApiError> {
        let mut value = self
            .upstream("/mods", &BTreeMap::new(), Some(&json!({"modIds":ids})))
            .await?;
        let mut seen = Vec::new();
        for project in value["data"].as_array_mut().ok_or_else(failed)? {
            validate_project(project, None)?;
            let id = numeric(project, "id")?;
            if !ids.contains(&id) || seen.contains(&id) {
                return Err(failed());
            }
            seen.push(id);
        }
        Ok(json!({"data":value["data"]}))
    }

    async fn fetch(
        &self,
        endpoint: Endpoint,
        query: &BTreeMap<String, String>,
    ) -> Result<Value, ApiError> {
        match endpoint {
            Endpoint::BatchMods(ids) => self.projects(&ids).await,
            Endpoint::BatchFiles(ids) => {
                let mut value = self
                    .upstream("/mods/files", query, Some(&json!({"fileIds":ids})))
                    .await?;
                let files = value["data"].as_array_mut().ok_or_else(failed)?;
                let mut projects = Vec::new();
                let mut seen = Vec::new();
                for file in files.iter() {
                    let id = numeric(file, "id")?;
                    let m = numeric(file, "modId")?;
                    if numeric(file, "gameId")? != 432 {
                        return Err(missing());
                    }
                    if !ids.contains(&id) || seen.contains(&id) {
                        return Err(failed());
                    }
                    seen.push(id);
                    if !projects.contains(&m) {
                        projects.push(m);
                    }
                }
                if !projects.is_empty() {
                    let owners = self.projects(&projects).await?;
                    for file in files {
                        let m = numeric(file, "modId")?;
                        let owner = owners["data"]
                            .as_array()
                            .ok_or_else(failed)?
                            .iter()
                            .find(|p| p["id"] == m)
                            .ok_or_else(missing)?;
                        validate_file(file, m, None, distribution(owner))?;
                    }
                }
                Ok(json!({"data":value["data"]}))
            }
            Endpoint::Search => {
                let mut value = self.upstream("/mods/search", query, None).await?;
                for project in value["data"].as_array_mut().ok_or_else(failed)? {
                    validate_project(project, None)?;
                }
                Ok(value)
            }
            Endpoint::Project(m) => self.project(m).await,
            Endpoint::Resolve(f) => {
                let value = self
                    .upstream("/mods/files", query, Some(&json!({"fileIds":[f]})))
                    .await?;
                let files = value["data"].as_array().ok_or_else(failed)?;
                if files.len() != 1 {
                    return Err(missing());
                }
                let mut file = files[0].clone();
                let m = numeric(&file, "modId")?;
                let project = self.project(m).await?;
                validate_file(&mut file, m, Some(f), distribution(&project["data"]))?;
                Ok(json!({"data": file}))
            }
            Endpoint::Files(m) | Endpoint::File(m, _) | Endpoint::Download(m, _) => {
                let project = self.project(m).await?;
                let allowed = distribution(&project["data"]);
                let path = match endpoint {
                    Endpoint::Files(_) => format!("/mods/{m}/files"),
                    Endpoint::File(_, f) | Endpoint::Download(_, f) => {
                        format!("/mods/{m}/files/{f}")
                    }
                    _ => return Err(invalid()),
                };
                let mut value = self.upstream(&path, query, None).await?;
                match endpoint {
                    Endpoint::Files(_) => {
                        for file in value["data"].as_array_mut().ok_or_else(failed)? {
                            validate_file(file, m, None, allowed)?;
                        }
                    }
                    Endpoint::File(_, f) | Endpoint::Download(_, f) => {
                        validate_file(&mut value["data"], m, Some(f), allowed)?;
                        if matches!(endpoint, Endpoint::Download(..)) {
                            if !allowed || !value["data"]["downloadUrl"].is_string() {
                                return Err(missing());
                            }
                            // Use only the author-permitted URL, never reconstruct CDN paths.
                            let download = self
                                .upstream(&format!("{path}/download-url"), query, None)
                                .await?;
                            if !download["data"].is_string() {
                                return Err(missing());
                            }
                            return Ok(download);
                        }
                    }
                    _ => return Err(invalid()),
                }
                Ok(value)
            }
        }
    }
}

fn contains_secret(value: &Value, key: &str) -> bool {
    match value {
        Value::String(s) => s.contains(key),
        Value::Array(a) => a.iter().any(|v| contains_secret(v, key)),
        Value::Object(o) => o
            .iter()
            .any(|(k, v)| k.contains(key) || contains_secret(v, key)),
        _ => false,
    }
}

fn numeric(value: &Value, name: &str) -> Result<u32, ApiError> {
    value[name]
        .as_u64()
        .filter(|n| (1..=i32::MAX as u64).contains(n))
        .map(|n| n as u32)
        .ok_or_else(failed)
}
fn distribution(project: &Value) -> bool {
    project["allowModDistribution"] != false
}
fn validate_project(value: &mut Value, expected: Option<u32>) -> Result<(), ApiError> {
    let m = numeric(value, "id")?;
    if numeric(value, "gameId")? != 432 || expected.is_some_and(|e| e != m) {
        return Err(missing());
    }
    let allowed = distribution(value);
    if let Some(files) = value.get_mut("latestFiles") {
        for file in files.as_array_mut().ok_or_else(failed)? {
            validate_file(file, m, None, allowed)?;
        }
    }
    Ok(())
}
fn validate_file(
    value: &mut Value,
    project: u32,
    expected: Option<u32>,
    allowed: bool,
) -> Result<(), ApiError> {
    let f = numeric(value, "id")?;
    if numeric(value, "modId")? != project
        || numeric(value, "gameId")? != 432
        || expected.is_some_and(|e| e != f)
    {
        return Err(missing());
    }
    if !allowed {
        value["downloadUrl"] = Value::Null;
    }
    Ok(())
}

pub(crate) async fn handle(
    State(state): State<AppState>,
    ClientIp(ip): ClientIp,
    OriginalUri(uri): OriginalUri,
    request: axum::extract::Request,
) -> Result<Response, ApiError> {
    if let Some(broker) = &state.curseforge {
        broker
            .limit
            .check(&ip)
            .map_err(|s| ApiError::rate_limited(s.min(60)))?;
    }
    if request.headers().contains_key("x-api-key") || request.headers().contains_key("api-key") {
        return Err(invalid());
    }
    let path = uri.path().strip_prefix(PREFIX).ok_or_else(invalid)?;
    let is_batch = request.method() == Method::POST;
    let (endpoint, query) = if request.method() == Method::GET {
        parse(path, uri.query())?
    } else if request.method() == Method::POST
        && uri.query().is_none()
        && matches!(path, "/mods" | "/files")
    {
        if request
            .headers()
            .get(axum::http::header::CONTENT_TYPE)
            .and_then(|v| v.to_str().ok())
            .is_none_or(|v| v.split(';').next().unwrap_or_default().trim() != "application/json")
        {
            return Err(invalid());
        }
        let body = axum::body::to_bytes(request.into_body(), 4096)
            .await
            .map_err(|_| {
                ApiError::new(
                    StatusCode::PAYLOAD_TOO_LARGE,
                    "invalidInput",
                    "catalogue request exceeds 4 KiB",
                )
            })?;
        (batch(path, &body)?, BTreeMap::new())
    } else {
        return Err(invalid());
    };
    let broker = state.curseforge.as_ref().ok_or_else(|| {
        ApiError::new(
            StatusCode::SERVICE_UNAVAILABLE,
            "network",
            "catalogue broker is not configured",
        )
    })?;
    let _permit = broker
        .concurrent
        .try_acquire()
        .map_err(|_| ApiError::rate_limited(1))?;
    let key = format!("{endpoint:?}:{query:?}");
    let cached = broker
        .cache
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(&key);
    let body = if let Some(body) = cached {
        body
    } else {
        let value = tokio::time::timeout(TIMEOUT, broker.fetch(endpoint, &query))
            .await
            .map_err(|_| failed())?
            .map_err(|e| {
                if is_batch && e.status == StatusCode::NOT_FOUND {
                    failed()
                } else {
                    e
                }
            })?;
        let body = serde_json::to_vec(&value).map_err(|_| failed())?;
        if body.len() > BODY_LIMIT {
            return Err(failed());
        }
        broker
            .cache
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .put(key, body.clone());
        body
    };
    Ok((
        [(axum::http::header::CONTENT_TYPE, "application/json")],
        body,
    )
        .into_response())
}

#[cfg(test)]
mod tests;
