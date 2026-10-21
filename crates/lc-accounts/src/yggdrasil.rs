//! Yggdrasil (authlib-injector) protocol client and the Drasl registration API.
//!
//! Spec: <https://github.com/yushijinhun/authlib-injector/wiki> ("Yggdrasil server
//! technical specification"). Drasl API: `/drasl/api/v3` (Drasl 4).

use std::time::Duration;

use base64::Engine as _;
use lc_types::skin::SkinModel;
use lc_types::{AppError, AppResult, ErrorCode};
use reqwest::{Client, RequestBuilder, Response, StatusCode};
use serde::{Deserialize, Serialize};
use serde_json::json;
use url::Url;

/// API root of the project's own server, used when `Settings.skin_server` is empty.
pub const DEFAULT_API_ROOT: &str = "https://skins.dedseckms.ru/authlib-injector";
/// Public Ely.by authlib-injector root (offered in the UI as an alternative).
pub const ELY_BY_API_ROOT: &str = "https://authserver.ely.by/api/authlib-injector";
/// ALI discovery header (authlib-injector "API Location Indication").
pub const ALI_HEADER: &str = "x-authlib-injector-api-location";
/// Drasl admin/user API prefix relative to the Drasl base URL.
pub const DRASL_API_PREFIX: &str = "drasl/api/v3";

const TIMEOUT: Duration = Duration::from_secs(30);
/// Metadata is passed on the JVM command line; keep it reasonably small.
const MAX_METADATA: usize = 64 * 1024;

/// Access + client token pair. Stored as JSON in the `TokenStore`, never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Session {
    pub access_token: String,
    pub client_token: String,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Session(***)")
    }
}

/// Game profile as returned by the auth server (`id` is undashed).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct GameProfile {
    pub id: String,
    pub name: String,
}

impl GameProfile {
    /// Dashed UUID, validated.
    pub fn dashed_uuid(&self) -> AppResult<String> {
        uuid::Uuid::parse_str(&self.id)
            .map(|u| u.hyphenated().to_string())
            .map_err(|_| protocol_error("profile id is not a UUID"))
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct AuthResponse {
    access_token: String,
    #[serde(default)]
    client_token: Option<String>,
    #[serde(default)]
    selected_profile: Option<GameProfile>,
    #[serde(default)]
    available_profiles: Vec<GameProfile>,
}

/// Textures of a profile (from the signed `textures` property).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Textures {
    pub skin_url: Option<String>,
    pub slim: bool,
    pub cape_url: Option<String>,
}

fn protocol_error(msg: &str) -> AppError {
    AppError::new(
        ErrorCode::Network,
        format!("unexpected server response: {msg}"),
    )
}

/// Joins `path` to an API root regardless of a trailing slash.
pub(crate) fn endpoint(root: &Url, path: &str) -> AppResult<Url> {
    let base = root.as_str().trim_end_matches('/');
    Url::parse(&format!("{base}/{}", path.trim_start_matches('/')))
        .map_err(|e| AppError::new(ErrorCode::InvalidInput, format!("bad URL: {e}")))
}

/// User input ("skins.example.ru", "https://host/authlib-injector") → URL.
/// A missing scheme means `https://` (authlib-injector convention).
pub fn parse_server_input(input: &str) -> AppResult<Url> {
    let s = input.trim();
    let invalid = || {
        AppError::new(ErrorCode::InvalidInput, "invalid server address").detail("field", "apiRoot")
    };
    if s.is_empty() {
        return Err(invalid());
    }
    let with_scheme = if s.contains("://") {
        s.to_owned()
    } else {
        format!("https://{s}")
    };
    let url = Url::parse(&with_scheme).map_err(|_| invalid())?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid());
    }
    Ok(url)
}

/// Drasl base URL: the API root without its trailing `authlib-injector` segment.
pub(crate) fn drasl_base(root: &Url) -> Url {
    let mut base = root.clone();
    let path = root.path().trim_end_matches('/');
    let trimmed = path.strip_suffix("/authlib-injector").unwrap_or(path);
    base.set_path(if trimmed.is_empty() { "/" } else { trimmed });
    base.set_query(None);
    base.set_fragment(None);
    base
}

async fn send(rb: RequestBuilder, url: &Url) -> AppResult<Response> {
    rb.timeout(TIMEOUT).send().await.map_err(|e| {
        AppError::new(ErrorCode::Network, format!("request failed: {e}"))
            .detail("url", url.as_str())
    })
}

async fn body(resp: Response, url: &Url, limit: usize) -> AppResult<Vec<u8>> {
    let bytes = resp.bytes().await.map_err(|e| {
        AppError::new(ErrorCode::Network, format!("read body: {e}")).detail("url", url.as_str())
    })?;
    if bytes.len() > limit {
        return Err(protocol_error("response too large").detail("url", url.as_str()));
    }
    Ok(bytes.to_vec())
}

/// Non-success status → `AppError`, with the server message (Yggdrasil
/// `errorMessage` or Drasl `message`) as a detail. Bodies never contain secrets.
async fn status_error(resp: Response, url: &Url) -> AppError {
    let status = resp.status();
    let text = body(resp, url, 16 * 1024).await.unwrap_or_default();
    let msg = serde_json::from_slice::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("errorMessage")
                .or_else(|| v.get("message"))
                .and_then(|m| m.as_str())
                .map(str::to_owned)
        })
        .unwrap_or_default();
    let code = match status {
        StatusCode::UNAUTHORIZED | StatusCode::FORBIDDEN => ErrorCode::PermissionDenied,
        StatusCode::NOT_FOUND => ErrorCode::NotFound,
        StatusCode::CONFLICT => ErrorCode::Conflict,
        StatusCode::BAD_REQUEST | StatusCode::UNPROCESSABLE_ENTITY => ErrorCode::InvalidInput,
        _ => ErrorCode::Network,
    };
    let mut err = AppError::new(code, format!("HTTP {status}"))
        .detail("url", url.as_str())
        .detail("status", status.as_u16().to_string());
    if !msg.is_empty() {
        err = err.detail("server", msg);
    }
    err
}

fn check_metadata(bytes: &[u8], url: &Url) -> AppResult<()> {
    let ok = serde_json::from_slice::<serde_json::Value>(bytes)
        .ok()
        .is_some_and(|v| v.get("meta").is_some_and(|m| m.is_object()));
    if ok {
        Ok(())
    } else {
        Err(AppError::new(
            ErrorCode::InvalidInput,
            "not an authlib-injector (Yggdrasil) server",
        )
        .detail("url", url.as_str()))
    }
}

/// `GET <api root>`: server metadata (JSON with `meta`, `signaturePublickey`...).
pub(crate) async fn fetch_metadata(client: &Client, root: &Url) -> AppResult<Vec<u8>> {
    let resp = send(client.get(root.clone()), root).await?;
    if !resp.status().is_success() {
        return Err(status_error(resp, root).await);
    }
    let bytes = body(resp, root, MAX_METADATA).await?;
    check_metadata(&bytes, root)?;
    Ok(bytes)
}

/// ALI discovery: follows `X-Authlib-Injector-API-Location` (relative or
/// absolute) once; otherwise `input` itself must be the API root.
/// Returns the API root and its metadata.
pub(crate) async fn discover(client: &Client, input: &Url) -> AppResult<(Url, Vec<u8>)> {
    let resp = send(client.get(input.clone()), input).await?;
    let base = resp.url().clone();
    let located = resp
        .headers()
        .get(ALI_HEADER)
        .and_then(|v| v.to_str().ok())
        .and_then(|loc| base.join(loc).ok());
    match located {
        Some(root) if root != base && root != *input => {
            let meta = fetch_metadata(client, &root).await?;
            Ok((root, meta))
        }
        _ => {
            if !resp.status().is_success() {
                return Err(status_error(resp, input).await);
            }
            let bytes = body(resp, input, MAX_METADATA).await?;
            check_metadata(&bytes, input)?;
            Ok((input.clone(), bytes))
        }
    }
}

/// `POST /authserver/authenticate`. When the server does not select a
/// profile, the first available one is selected via `refresh`.
pub(crate) async fn authenticate(
    client: &Client,
    root: &Url,
    username: &str,
    password: &str,
    client_token: &str,
) -> AppResult<(Session, GameProfile)> {
    let url = endpoint(root, "authserver/authenticate")?;
    let req = json!({
        "agent": { "name": "Minecraft", "version": 1 },
        "username": username,
        "password": password,
        "clientToken": client_token,
        "requestUser": false,
    });
    let resp = send(client.post(url.clone()).json(&req), &url).await?;
    if !resp.status().is_success() {
        let mut err = status_error(resp, &url).await;
        if err.code == ErrorCode::PermissionDenied {
            err.message = "invalid username or password".into();
        }
        return Err(err);
    }
    let parsed: AuthResponse = serde_json::from_slice(&body(resp, &url, 1 << 20).await?)
        .map_err(|_| protocol_error("bad authenticate response"))?;
    let session = Session {
        access_token: parsed.access_token,
        client_token: parsed
            .client_token
            .unwrap_or_else(|| client_token.to_owned()),
    };
    if let Some(p) = parsed.selected_profile {
        return Ok((session, p));
    }
    let Some(first) = parsed.available_profiles.into_iter().next() else {
        return Err(AppError::new(
            ErrorCode::NotFound,
            "this account has no game profile",
        ));
    };
    match refresh(client, root, &session, Some(&first)).await? {
        Some((s, p)) => Ok((s, p.unwrap_or(first))),
        None => Err(AppError::new(
            ErrorCode::AuthExpired,
            "profile selection rejected",
        )),
    }
}

/// `POST /authserver/validate`: `Ok(false)` when the token is no longer valid.
pub(crate) async fn validate(client: &Client, root: &Url, s: &Session) -> AppResult<bool> {
    let url = endpoint(root, "authserver/validate")?;
    let req = json!({ "accessToken": s.access_token, "clientToken": s.client_token });
    let resp = send(client.post(url.clone()).json(&req), &url).await?;
    match resp.status() {
        st if st.is_success() => Ok(true),
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED | StatusCode::BAD_REQUEST => Ok(false),
        _ => Err(status_error(resp, &url).await),
    }
}

/// `POST /authserver/refresh`: new session (+ profile), `Ok(None)` when the
/// server rejects the tokens (re-login needed). Network errors stay errors.
pub(crate) async fn refresh(
    client: &Client,
    root: &Url,
    s: &Session,
    select: Option<&GameProfile>,
) -> AppResult<Option<(Session, Option<GameProfile>)>> {
    let url = endpoint(root, "authserver/refresh")?;
    let mut req = json!({
        "accessToken": s.access_token,
        "clientToken": s.client_token,
        "requestUser": false,
    });
    if let Some(p) = select {
        req["selectedProfile"] = json!({ "id": p.id, "name": p.name });
    }
    let resp = send(client.post(url.clone()).json(&req), &url).await?;
    match resp.status() {
        st if st.is_success() => {}
        StatusCode::FORBIDDEN | StatusCode::UNAUTHORIZED | StatusCode::BAD_REQUEST => {
            return Ok(None)
        }
        _ => return Err(status_error(resp, &url).await),
    }
    let parsed: AuthResponse = serde_json::from_slice(&body(resp, &url, 1 << 20).await?)
        .map_err(|_| protocol_error("bad refresh response"))?;
    let session = Session {
        access_token: parsed.access_token,
        client_token: parsed
            .client_token
            .unwrap_or_else(|| s.client_token.clone()),
    };
    Ok(Some((session, parsed.selected_profile)))
}

/// `POST /authserver/invalidate`, best effort (logout).
pub(crate) async fn invalidate(client: &Client, root: &Url, s: &Session) {
    let Ok(url) = endpoint(root, "authserver/invalidate") else {
        return;
    };
    let req = json!({ "accessToken": s.access_token, "clientToken": s.client_token });
    let _ = client
        .post(url)
        .json(&req)
        .timeout(Duration::from_secs(5))
        .send()
        .await;
}

/// Texture kind in `/api/user/profile/{uuid}/{kind}`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TextureKind {
    Skin,
    Cape,
}

impl TextureKind {
    fn path(self) -> &'static str {
        match self {
            Self::Skin => "skin",
            Self::Cape => "cape",
        }
    }
}

fn texture_url(root: &Url, uuid: &str, kind: TextureKind) -> AppResult<Url> {
    endpoint(
        root,
        &format!("api/user/profile/{}/{}", uuid.replace('-', ""), kind.path()),
    )
}

fn auth_status(err: AppError) -> AppError {
    if err.code == ErrorCode::PermissionDenied {
        AppError {
            code: ErrorCode::AuthExpired,
            ..err
        }
    } else {
        err
    }
}

/// `PUT /api/user/profile/{uuid}/{skin|cape}` (multipart `model` + `file`),
/// authorized with the Yggdrasil access token. Supported by Drasl.
pub(crate) async fn upload_texture(
    client: &Client,
    root: &Url,
    s: &Session,
    uuid: &str,
    kind: TextureKind,
    png: Vec<u8>,
    model: SkinModel,
) -> AppResult<()> {
    let url = texture_url(root, uuid, kind)?;
    let part = reqwest::multipart::Part::bytes(png)
        .file_name(format!("{}.png", kind.path()))
        .mime_str("image/png")
        .map_err(|e| AppError::internal(format!("multipart: {e}")))?;
    let mut form = reqwest::multipart::Form::new();
    if kind == TextureKind::Skin {
        let m = match model {
            SkinModel::Classic => "",
            SkinModel::Slim => "slim",
        };
        form = form.text("model", m);
    }
    form = form.part("file", part);
    let rb = client
        .put(url.clone())
        .bearer_auth(&s.access_token)
        .multipart(form);
    let resp = send(rb, &url).await?;
    if !resp.status().is_success() {
        return Err(auth_status(status_error(resp, &url).await));
    }
    Ok(())
}

/// `DELETE /api/user/profile/{uuid}/{skin|cape}`.
pub(crate) async fn delete_texture(
    client: &Client,
    root: &Url,
    s: &Session,
    uuid: &str,
    kind: TextureKind,
) -> AppResult<()> {
    let url = texture_url(root, uuid, kind)?;
    let rb = client.delete(url.clone()).bearer_auth(&s.access_token);
    let resp = send(rb, &url).await?;
    if !resp.status().is_success() {
        return Err(auth_status(status_error(resp, &url).await));
    }
    Ok(())
}

/// `GET /sessionserver/session/minecraft/profile/{uuid}`: decoded textures.
pub(crate) async fn fetch_textures(client: &Client, root: &Url, uuid: &str) -> AppResult<Textures> {
    let url = endpoint(
        root,
        &format!(
            "sessionserver/session/minecraft/profile/{}",
            uuid.replace('-', "")
        ),
    )?;
    let resp = send(client.get(url.clone()), &url).await?;
    if resp.status() == StatusCode::NO_CONTENT {
        return Err(AppError::new(ErrorCode::NotFound, "profile not found"));
    }
    if !resp.status().is_success() {
        return Err(status_error(resp, &url).await);
    }
    let v: serde_json::Value = serde_json::from_slice(&body(resp, &url, 1 << 20).await?)
        .map_err(|_| protocol_error("bad profile response"))?;
    parse_textures(&v)
}

/// Extracts skin/cape URLs from a profile with a base64 `textures` property.
pub(crate) fn parse_textures(profile: &serde_json::Value) -> AppResult<Textures> {
    let value = profile
        .get("properties")
        .and_then(|p| p.as_array())
        .and_then(|props| {
            props
                .iter()
                .find(|p| p.get("name").and_then(|n| n.as_str()) == Some("textures"))
        })
        .and_then(|p| p.get("value"))
        .and_then(|v| v.as_str());
    let Some(value) = value else {
        return Ok(Textures::default());
    };
    let raw = base64::engine::general_purpose::STANDARD
        .decode(value)
        .map_err(|_| protocol_error("textures is not base64"))?;
    let t: serde_json::Value =
        serde_json::from_slice(&raw).map_err(|_| protocol_error("textures is not JSON"))?;
    let tex = t.get("textures");
    let url_of = |k: &str| {
        tex.and_then(|x| x.get(k))
            .and_then(|x| x.get("url"))
            .and_then(|u| u.as_str())
            .map(str::to_owned)
    };
    let slim = tex
        .and_then(|x| x.get("SKIN"))
        .and_then(|x| x.get("metadata"))
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        == Some("slim");
    Ok(Textures {
        skin_url: url_of("SKIN"),
        slim,
        cape_url: url_of("CAPE"),
    })
}

/// `POST /drasl/api/v3/users` without admin auth: self-registration with an
/// invite code (Drasl decides whether an invite is required).
pub(crate) async fn drasl_register(
    client: &Client,
    root: &Url,
    invite: &str,
    username: &str,
    password: &str,
) -> AppResult<()> {
    let url = endpoint(&drasl_base(root), &format!("{DRASL_API_PREFIX}/users"))?;
    let mut req = json!({ "username": username, "password": password });
    if !invite.trim().is_empty() {
        req["inviteCode"] = json!(invite.trim());
    }
    let resp = send(client.post(url.clone()).json(&req), &url).await?;
    if !resp.status().is_success() {
        return Err(status_error(resp, &url).await);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn server_input_parsing() {
        assert_eq!(
            parse_server_input(" skins.example.ru ").unwrap().as_str(),
            "https://skins.example.ru/"
        );
        assert_eq!(
            parse_server_input("http://h:8080/authlib-injector")
                .unwrap()
                .as_str(),
            "http://h:8080/authlib-injector"
        );
        for bad in ["", "   ", "ftp://x", "https://"] {
            assert_eq!(
                parse_server_input(bad).unwrap_err().code,
                ErrorCode::InvalidInput
            );
        }
    }

    #[test]
    fn drasl_base_strips_ali_segment() {
        let u = |s: &str| Url::parse(s).unwrap();
        assert_eq!(
            drasl_base(&u("https://h/authlib-injector")).as_str(),
            "https://h/"
        );
        assert_eq!(
            drasl_base(&u("https://h/sub/authlib-injector/")).as_str(),
            "https://h/sub"
        );
        assert_eq!(
            endpoint(
                &drasl_base(&u("https://h/sub/authlib-injector")),
                "drasl/api/v3/users"
            )
            .unwrap()
            .as_str(),
            "https://h/sub/drasl/api/v3/users"
        );
        assert_eq!(
            endpoint(&u("https://h/authlib-injector/"), "/authserver/validate")
                .unwrap()
                .as_str(),
            "https://h/authlib-injector/authserver/validate"
        );
    }

    #[test]
    fn auth_response_fixture() {
        let raw = r#"{"accessToken":"at","clientToken":"ct",
            "availableProfiles":[{"id":"0123456789abcdef0123456789abcdef","name":"Steve"}],
            "selectedProfile":{"id":"0123456789abcdef0123456789abcdef","name":"Steve"},
            "user":{"id":"u","properties":[]}}"#;
        let r: AuthResponse = serde_json::from_str(raw).unwrap();
        let p = r.selected_profile.unwrap();
        assert_eq!(p.name, "Steve");
        assert_eq!(
            p.dashed_uuid().unwrap(),
            "01234567-89ab-cdef-0123-456789abcdef"
        );
        let s = Session {
            access_token: r.access_token,
            client_token: "secret-ct".into(),
        };
        assert!(!format!("{s:?}").contains("secret-ct"));
    }

    #[test]
    fn textures_fixture() {
        let tex = json!({"textures": {
            "SKIN": {"url": "https://h/texture/skin/abc", "metadata": {"model": "slim"}},
            "CAPE": {"url": "https://h/texture/cape/def"}
        }});
        let value = base64::engine::general_purpose::STANDARD.encode(tex.to_string());
        let profile = json!({"id": "x", "name": "n",
            "properties": [{"name": "textures", "value": value, "signature": "sig"}]});
        let t = parse_textures(&profile).unwrap();
        assert_eq!(t.skin_url.as_deref(), Some("https://h/texture/skin/abc"));
        assert!(t.slim);
        assert_eq!(t.cape_url.as_deref(), Some("https://h/texture/cape/def"));
        assert_eq!(
            parse_textures(&json!({"id": "x", "properties": []})).unwrap(),
            Textures::default()
        );
    }
}
