//! End-to-end tests against a local axum server imitating Drasl 4
//! (authlib-injector API + `/drasl/api/v3/users`) and the authlib-injector
//! artifact host. No external network.

use std::collections::{HashMap, HashSet};
use std::net::SocketAddr;
use std::sync::{Arc, Mutex};

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use lc_http::{DownloaderConfig, HttpDownloader};
use lc_runtime::{MemorySettingsStore, Paths};
use lc_types::account::AccountKind;
use lc_types::skin::SkinModel;
use lc_types::ErrorCode;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use url::Url;

use crate::authlib::AuthlibManager;
use crate::contract::{AccountService, AuthProvider, SkinService, TokenStore, UserType};
use crate::fsutil::testutil::TempDir;
use crate::service::{token_key, AccountsService};
use crate::skins::SkinsService;
use crate::tokens::MemoryTokenStore;

const PROFILE_ID: &str = "0123456789abcdef0123456789abcdef";
const JAR: &[u8] = b"PK\x03\x04 fake authlib-injector jar";

#[derive(Default)]
struct Mock {
    users: HashMap<String, String>,
    valid: Option<String>,
    refreshable: HashSet<String>,
    counter: u32,
    skin: Option<(String, Vec<u8>)>,
    cape: bool,
    calls: Vec<String>,
}

type Shared = Arc<Mutex<Mock>>;

fn sha256_hex(d: &[u8]) -> String {
    Sha256::digest(d)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn json_resp(status: StatusCode, v: Value) -> Response {
    (
        status,
        [("content-type", "application/json")],
        v.to_string(),
    )
        .into_response()
}

fn forbidden() -> Response {
    json_resp(
        StatusCode::FORBIDDEN,
        json!({"error": "ForbiddenOperationException", "errorMessage": "Invalid token."}),
    )
}

fn issue(m: &mut Mock) -> String {
    m.counter += 1;
    let t = format!("access-{}", m.counter);
    m.valid = Some(t.clone());
    m.refreshable.insert(t.clone());
    t
}

fn latest(sha: &str) -> Value {
    json!({
        "build_number": 53,
        "version": "1.2.5",
        "download_url": "https://authlib-injector.yushi.moe/artifact/53/authlib-injector-1.2.5.jar",
        "checksums": { "sha256": sha }
    })
}

async fn handle(
    state: Shared,
    method: Method,
    uri: Uri,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    let path = uri.path().to_owned();
    let body_json: Value = serde_json::from_slice(&body).unwrap_or(Value::Null);
    let mut m = state.lock().unwrap();
    m.calls.push(format!("{method} {path}"));
    let bearer = headers
        .get("authorization")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::to_owned);
    let tex_base = "/authlib-injector/api/user/profile/";
    match (method.as_str(), path.as_str()) {
        ("GET", "/") => (
            StatusCode::OK,
            [("x-authlib-injector-api-location", "/authlib-injector")],
            "<html>Drasl</html>",
        )
            .into_response(),
        ("GET", "/authlib-injector" | "/authlib-injector/") => json_resp(
            StatusCode::OK,
            json!({"meta": {"serverName": "Mock", "implementationName": "Drasl"},
                   "skinDomains": ["127.0.0.1"], "signaturePublickey": "KEY"}),
        ),
        ("POST", "/authlib-injector/authserver/authenticate") => {
            let u = body_json["username"].as_str().unwrap_or_default();
            let p = body_json["password"].as_str().unwrap_or_default();
            if m.users.get(u).map(String::as_str) != Some(p) {
                return json_resp(
                    StatusCode::FORBIDDEN,
                    json!({"error": "ForbiddenOperationException",
                           "errorMessage": "Invalid credentials."}),
                );
            }
            let at = issue(&mut m);
            json_resp(
                StatusCode::OK,
                json!({"accessToken": at, "clientToken": body_json["clientToken"],
                       "availableProfiles": [{"id": PROFILE_ID, "name": u}],
                       "selectedProfile": {"id": PROFILE_ID, "name": u}}),
            )
        }
        ("POST", "/authlib-injector/authserver/validate") => {
            if body_json["accessToken"].as_str() == m.valid.as_deref() {
                StatusCode::NO_CONTENT.into_response()
            } else {
                forbidden()
            }
        }
        ("POST", "/authlib-injector/authserver/refresh") => {
            let old = body_json["accessToken"].as_str().unwrap_or_default();
            if !m.refreshable.remove(old) {
                return forbidden();
            }
            let at = issue(&mut m);
            json_resp(
                StatusCode::OK,
                json!({"accessToken": at, "clientToken": body_json["clientToken"],
                       "selectedProfile": {"id": PROFILE_ID, "name": "Renamed"}}),
            )
        }
        ("POST", "/authlib-injector/authserver/invalidate") => {
            m.valid = None;
            StatusCode::NO_CONTENT.into_response()
        }
        ("PUT" | "DELETE", p) if p.starts_with(tex_base) => {
            if bearer.is_none() || bearer != m.valid {
                return StatusCode::UNAUTHORIZED.into_response();
            }
            let rest = &p[tex_base.len()..];
            match (method.as_str(), rest) {
                ("PUT", r) if r == format!("{PROFILE_ID}/skin") => {
                    let text = String::from_utf8_lossy(&body).to_string();
                    let model = if text.contains("slim") { "slim" } else { "" };
                    m.skin = Some((model.to_owned(), body.to_vec()));
                    StatusCode::NO_CONTENT.into_response()
                }
                ("DELETE", r) if r == format!("{PROFILE_ID}/cape") => {
                    m.cape = false;
                    StatusCode::NO_CONTENT.into_response()
                }
                _ => StatusCode::BAD_REQUEST.into_response(),
            }
        }
        ("GET", p)
            if p == format!(
                "/authlib-injector/sessionserver/session/minecraft/profile/{PROFILE_ID}"
            ) =>
        {
            let mut tex = serde_json::Map::new();
            if let Some((model, _)) = &m.skin {
                let mut skin = json!({"url": "http://127.0.0.1/texture/skin/abc"});
                if model == "slim" {
                    skin["metadata"] = json!({"model": "slim"});
                }
                tex.insert("SKIN".into(), skin);
            }
            if m.cape {
                tex.insert(
                    "CAPE".into(),
                    json!({"url": "http://127.0.0.1/texture/cape/cape1"}),
                );
            }
            let value = base64::engine::general_purpose::STANDARD
                .encode(json!({"textures": tex}).to_string());
            json_resp(
                StatusCode::OK,
                json!({"id": PROFILE_ID, "name": "Steve",
                       "properties": [{"name": "textures", "value": value}]}),
            )
        }
        ("POST", "/drasl/api/v3/users") => {
            if body_json["inviteCode"].as_str() != Some("INVITE") {
                return json_resp(
                    StatusCode::FORBIDDEN,
                    json!({"message": "Registration requires an invite"}),
                );
            }
            let u = body_json["username"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            if m.users.contains_key(&u) {
                return json_resp(StatusCode::CONFLICT, json!({"message": "Username taken"}));
            }
            let p = body_json["password"]
                .as_str()
                .unwrap_or_default()
                .to_owned();
            m.users.insert(u.clone(), p);
            json_resp(StatusCode::OK, json!({"username": u}))
        }
        ("GET", "/dead/artifact/latest.json") => StatusCode::BAD_GATEWAY.into_response(),
        ("GET", "/primary/artifact/latest.json") => {
            json_resp(StatusCode::OK, latest(&sha256_hex(JAR)))
        }
        // Must never be used: checksums come only from the primary source.
        ("GET", "/mirror/artifact/latest.json") => {
            json_resp(StatusCode::OK, latest(&"1".repeat(64)))
        }
        ("GET", "/bad/artifact/latest.json") => json_resp(StatusCode::OK, latest(&"0".repeat(64))),
        ("GET", "/mirror/artifact/53/authlib-injector-1.2.5.jar")
        | ("GET", "/bad/artifact/53/authlib-injector-1.2.5.jar") => {
            (StatusCode::OK, JAR.to_vec()).into_response()
        }
        _ => StatusCode::NOT_FOUND.into_response(),
    }
}

async fn start() -> (SocketAddr, Shared) {
    let state: Shared = Arc::new(Mutex::new(Mock::default()));
    state
        .lock()
        .unwrap()
        .users
        .insert("Steve".into(), "pw".into());
    let s = state.clone();
    let app = axum::Router::new().fallback(
        move |method: Method, uri: Uri, headers: HeaderMap, body: Bytes| {
            handle(s.clone(), method, uri, headers, body)
        },
    );
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    (addr, state)
}

struct Env {
    _dir: TempDir,
    addr: SocketAddr,
    mock: Shared,
    tokens: Arc<MemoryTokenStore>,
    svc: Arc<AccountsService>,
    skins: SkinsService,
    dir_path: std::path::PathBuf,
}

async fn env_with_bases(bases: &[&str]) -> Env {
    let (addr, mock) = start().await;
    let dir = TempDir::new();
    let paths = Paths::new(dir.path());
    let tokens = Arc::new(MemoryTokenStore::new());
    let dl = Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap());
    let bases = bases
        .iter()
        .map(|b| Url::parse(&format!("http://{addr}/{b}/")).unwrap())
        .collect();
    let svc = Arc::new(
        AccountsService::new(&paths, dl, tokens.clone())
            .with_settings(Arc::new(MemorySettingsStore::default()))
            .with_authlib(AuthlibManager::with_bases(
                paths.cache().join("authlib-injector"),
                bases,
            )),
    );
    let skins = SkinsService::new(&paths, svc.clone());
    Env {
        dir_path: dir.path().to_owned(),
        _dir: dir,
        addr,
        mock,
        tokens,
        svc,
        skins,
    }
}

async fn env() -> Env {
    env_with_bases(&["primary", "mirror"]).await
}

impl Env {
    fn domain(&self) -> Url {
        Url::parse(&format!("http://{}", self.addr)).unwrap()
    }

    fn root(&self) -> String {
        format!("http://{}/authlib-injector", self.addr)
    }
}

#[tokio::test]
async fn login_via_ali_discovery_stores_tokens() {
    let e = env().await;
    let acc = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();
    assert_eq!(acc.kind, AccountKind::Yggdrasil);
    assert_eq!(acc.name, "Steve");
    assert_eq!(acc.uuid, "01234567-89ab-cdef-0123-456789abcdef");
    assert_eq!(acc.server.as_deref(), Some(e.root().as_str()));
    assert!(acc.is_default && acc.valid);

    let stored = e.tokens.get(&token_key(acc.id)).await.unwrap().unwrap();
    assert!(stored.expose().contains("access-1"));
    let file = std::fs::read_to_string(e.dir_path.join(crate::ACCOUNTS_FILE)).unwrap();
    assert!(!file.contains("access-1"));

    // Re-login updates the same account instead of duplicating it.
    let again = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();
    assert_eq!(again.id, acc.id);
    assert_eq!(e.svc.list().await.unwrap().len(), 1);
}

#[tokio::test]
async fn bad_credentials_and_non_yggdrasil_server() {
    let e = env().await;
    let err = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "wrong")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert!(!format!("{err:?}").contains("wrong"));

    let not_ygg = Url::parse(&format!("http://{}/nothing", e.addr)).unwrap();
    assert!(e
        .svc
        .yggdrasil_login(&not_ygg, "Steve", "pw")
        .await
        .is_err());
    assert!(e.svc.list().await.unwrap().is_empty());
}

#[tokio::test]
async fn launch_auth_validate_refresh_expire() {
    let e = env().await;
    let acc = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();

    // Valid token: used as is, authlib-injector downloaded via the mirror.
    let auth = e.svc.launch_auth(None).await.unwrap();
    assert_eq!(auth.access_token.expose(), "access-1");
    assert_eq!(auth.user_type, UserType::Mojang);
    assert!(!format!("{auth:?}").contains("access-1"));
    let ai = auth.authlib.unwrap();
    assert_eq!(ai.api_root.as_str(), e.root());
    assert_eq!(std::fs::read(&ai.jar).unwrap(), JAR);
    let meta = base64::engine::general_purpose::STANDARD
        .decode(ai.prefetched.unwrap())
        .unwrap();
    let meta: Value = serde_json::from_slice(&meta).unwrap();
    assert_eq!(meta["meta"]["serverName"], "Mock");

    // Server-side expiry of the access token → refresh.
    e.mock.lock().unwrap().valid = None;
    let auth = e.svc.launch_auth(Some(acc.id)).await.unwrap();
    assert_eq!(auth.access_token.expose(), "access-2");
    assert_eq!(auth.name, "Renamed", "profile name comes from the server");
    let stored = e.tokens.get(&token_key(acc.id)).await.unwrap().unwrap();
    assert!(stored.expose().contains("access-2"));

    // Neither validate nor refresh work → AuthExpired, account marked invalid.
    {
        let mut m = e.mock.lock().unwrap();
        m.valid = None;
        m.refreshable.clear();
    }
    let err = e.svc.launch_auth(Some(acc.id)).await.unwrap_err();
    assert_eq!(err.code, ErrorCode::AuthExpired);
    assert!(!e.svc.list().await.unwrap()[0].valid);

    // Logging in again restores validity.
    e.svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();
    assert!(e.svc.list().await.unwrap()[0].valid);
    assert!(e.svc.launch_auth(None).await.is_ok());
}

#[tokio::test]
async fn missing_token_is_auth_expired() {
    let e = env().await;
    let acc = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();
    e.tokens.delete(&token_key(acc.id)).await.unwrap();
    assert_eq!(
        e.svc.launch_auth(None).await.unwrap_err().code,
        ErrorCode::AuthExpired
    );
}

#[tokio::test]
async fn register_with_and_without_invite() {
    let e = env().await;
    let err = e
        .svc
        .yggdrasil_register(&e.domain(), "", "Alex", "secret")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert_eq!(
        err.details.as_ref().unwrap()["server"],
        "Registration requires an invite"
    );
    let err = e
        .svc
        .yggdrasil_register(&e.domain(), "WRONG", "Alex", "secret")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::PermissionDenied);
    assert!(e.svc.list().await.unwrap().is_empty());

    let acc = e
        .svc
        .yggdrasil_register(&e.domain(), "INVITE", "Alex", "secret")
        .await
        .unwrap();
    assert_eq!(acc.name, "Alex");
    assert!(e.tokens.get(&token_key(acc.id)).await.unwrap().is_some());

    let err = e
        .svc
        .yggdrasil_register(&e.domain(), "INVITE", "Alex", "secret")
        .await
        .unwrap_err();
    assert_eq!(err.code, ErrorCode::Conflict);
}

#[tokio::test]
async fn skin_library_apply_and_capes() {
    let e = env().await;
    let png_path = e.dir_path.join("in.png");
    std::fs::write(&png_path, crate::skins::tests::png(64, 64)).unwrap();
    let bad_path = e.dir_path.join("bad.png");
    std::fs::write(&bad_path, crate::skins::tests::png(32, 32)).unwrap();

    let p = png_path.to_str().unwrap();
    assert_eq!(
        e.skins
            .add(bad_path.to_str().unwrap(), "Bad", SkinModel::Classic)
            .await
            .unwrap_err()
            .code,
        ErrorCode::InvalidInput
    );
    let skin = e.skins.add(p, "Mine", SkinModel::Slim).await.unwrap();
    assert!(skin.local_only);
    assert!(std::path::Path::new(skin.local_path.as_ref().unwrap()).exists());
    assert_eq!(
        e.skins.add(p, "Dup", SkinModel::Slim).await.unwrap().id,
        skin.id
    );
    assert_eq!(e.skins.library().await.unwrap().len(), 1);

    // Offline accounts have no skin server.
    let off = e.svc.offline_add("Offline").await.unwrap();
    assert_eq!(
        e.skins.apply(off.id, skin.id).await.unwrap_err().code,
        ErrorCode::InvalidInput
    );

    let acc = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();
    e.skins.apply(acc.id, skin.id).await.unwrap();
    {
        let m = e.mock.lock().unwrap();
        let (model, body) = m.skin.clone().unwrap();
        assert_eq!(model, "slim");
        assert!(String::from_utf8_lossy(&body).contains("name=\"file\""));
    }
    let lib = e.skins.library().await.unwrap();
    assert!(!lib[0].local_only);
    assert_eq!(lib[0].url, "http://127.0.0.1/texture/skin/abc");
    let accounts = e.svc.list().await.unwrap();
    let a = accounts.iter().find(|a| a.id == acc.id).unwrap();
    assert_eq!(
        a.skin_url.as_deref(),
        Some("http://127.0.0.1/texture/skin/abc")
    );

    // Capes: only the active one is known; removal goes through DELETE.
    assert!(e.skins.capes(acc.id).await.unwrap().is_empty());
    e.mock.lock().unwrap().cape = true;
    let capes = e.skins.capes(acc.id).await.unwrap();
    assert_eq!(capes.len(), 1);
    assert_eq!(capes[0].id, "cape1");
    e.skins
        .set_cape(acc.id, Some("cape1".into()))
        .await
        .unwrap();
    assert_eq!(
        e.skins
            .set_cape(acc.id, Some("other".into()))
            .await
            .unwrap_err()
            .code,
        ErrorCode::NotFound
    );
    e.skins.set_cape(acc.id, None).await.unwrap();
    assert!(!e.mock.lock().unwrap().cape);

    e.skins.remove(skin.id).await.unwrap();
    assert!(e.skins.library().await.unwrap().is_empty());
    assert!(!std::path::Path::new(skin.local_path.as_ref().unwrap()).exists());
}

#[tokio::test]
async fn authlib_download_mirror_cache_and_hash() {
    let e = env().await;
    let client = e.svc.http();
    let base = |p: &str| Url::parse(&format!("http://{}/{p}/", e.addr)).unwrap();

    // Primary down: the mirror's latest.json is never trusted → error without cache.
    let untrusted = AuthlibManager::with_bases(
        e.dir_path.join("cache-0"),
        vec![base("dead"), base("mirror")],
    );
    assert_eq!(
        untrusted.ensure(&client).await.unwrap_err().code,
        ErrorCode::Network
    );
    assert!(!e
        .mock
        .lock()
        .unwrap()
        .calls
        .iter()
        .any(|c| c.contains("/mirror/")));

    // Hash from the primary, jar (missing on the primary) from the mirror.
    let dir = e.dir_path.join("cache-a");
    let mgr = AuthlibManager::with_bases(dir.clone(), vec![base("primary"), base("mirror")]);
    let jar = mgr.ensure(&client).await.unwrap();
    assert_eq!(std::fs::read(&jar).unwrap(), JAR);
    assert!(jar.ends_with("authlib-injector-1.2.5.jar"));
    {
        let m = e.mock.lock().unwrap();
        assert!(m.calls.iter().any(|c| c.contains("/primary/artifact/53/")));
        assert!(!m
            .calls
            .iter()
            .any(|c| c.contains("/mirror/artifact/latest.json")));
    }

    // Fresh cache: no network at all.
    let before = e.mock.lock().unwrap().calls.len();
    assert_eq!(mgr.ensure(&client).await.unwrap(), jar);
    assert_eq!(e.mock.lock().unwrap().calls.len(), before);

    // All sources down but a verified cache exists → cache is used.
    let offline = AuthlibManager::with_bases(
        dir.clone(),
        vec![Url::parse(&format!("http://{}/dead/", e.addr)).unwrap()],
    );
    let state = dir.join("state.json");
    let mut st: Value = serde_json::from_str(&std::fs::read_to_string(&state).unwrap()).unwrap();
    st["checkedAt"] = json!(0);
    std::fs::write(&state, st.to_string()).unwrap();
    assert_eq!(offline.ensure(&client).await.unwrap(), jar);

    // Hash mismatch is rejected and nothing is cached.
    let bad_dir = e.dir_path.join("cache-b");
    let bad = AuthlibManager::with_bases(
        bad_dir.clone(),
        vec![Url::parse(&format!("http://{}/bad/", e.addr)).unwrap()],
    );
    assert_eq!(
        bad.ensure(&client).await.unwrap_err().code,
        ErrorCode::HashMismatch
    );
    assert!(!bad_dir.join("authlib-injector-1.2.5.jar").exists());
}

#[tokio::test]
async fn remove_invalidates_server_session() {
    let e = env().await;
    let acc = e
        .svc
        .yggdrasil_login(&e.domain(), "Steve", "pw")
        .await
        .unwrap();
    e.svc.remove(acc.id).await.unwrap();
    assert!(e.mock.lock().unwrap().valid.is_none());
    assert!(e.tokens.get(&token_key(acc.id)).await.unwrap().is_none());
}
