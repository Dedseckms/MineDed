//! `/me/vpn` (hub.yaml `getVpnAccess` / `provisionVpnAccess`) against a mocked s-ui `/apiv2`.

use std::sync::Mutex;

use axum::extract::Form;
use axum::routing::{get, post};
use std::collections::HashMap;

use super::*;

const TOKEN: &str = "sui-token";
const GIB: u64 = 1 << 30;

#[derive(Default)]
struct Panel {
    clients: Vec<Value>,
    creates: usize,
    saves: Vec<Value>,
}

type Shared = Arc<Mutex<Panel>>;

fn token_ok(h: &HeaderMap) -> bool {
    h.get("token").and_then(|v| v.to_str().ok()) == Some(TOKEN)
}

/// Mock s-ui: errors are HTTP 200 with `success:false`, like the real panel.
async fn fake_sui(panel: Shared) -> String {
    let p1 = panel.clone();
    let app = Router::new()
        .route(
            "/panel/apiv2/clients",
            get(move |h: HeaderMap| {
                let p = p1.clone();
                async move {
                    if !token_ok(&h) {
                        return Json(
                            json!({"success": false, "msg": "invalid token", "obj": null}),
                        );
                    }
                    let list = p.lock().unwrap().clients.clone();
                    Json(json!({"success": true, "msg": "", "obj": {"clients": list}}))
                }
            }),
        )
        .route(
            "/panel/apiv2/save",
            post(
                move |h: HeaderMap, Form(f): Form<HashMap<String, String>>| {
                    let p = panel.clone();
                    async move {
                        if !token_ok(&h) || f.get("object").map(String::as_str) != Some("clients") {
                            return Json(json!({"success": false, "msg": "bad", "obj": null}));
                        }
                        let data: Value = serde_json::from_str(&f["data"]).unwrap();
                        let mut p = p.lock().unwrap();
                        assert_eq!(f["action"], "new");
                        let mut c = data.clone();
                        c["id"] = json!(p.clients.len() + 1);
                        // The list endpoint returns summaries without `config`/`links`.
                        c.as_object_mut().unwrap().remove("config");
                        c.as_object_mut().unwrap().remove("links");
                        p.clients.push(c);
                        p.creates += 1;
                        p.saves.push(data);
                        Json(json!({"success": true, "msg": "", "obj": null}))
                    }
                },
            ),
        );
    let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = l.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
    format!("http://{addr}/panel/apiv2/")
}

fn vpn_app(api_url: String, token: &str) -> Router {
    let cfg = Config {
        vpn: Some(VpnConfig {
            api_url,
            api_token: token.into(),
            sub_base: "https://relay.example.ru/subsecret/".into(),
            quota_bytes: 256 * GIB,
        }),
        ..Config::default()
    };
    router(AppState::new(cfg, Arc::new(FakeValidator)))
}

fn req(method: &str, token: Option<&str>) -> Request<Body> {
    let mut b = Request::builder().method(method).uri("/api/v1/me/vpn");
    if let Some(t) = token {
        b = b.header("authorization", format!("Bearer {t}"));
    }
    b.body(Body::empty()).unwrap()
}

/// `VpnAccess` schema.
fn assert_vpn_access(v: &Value) {
    let url = v["subscriptionUrl"].as_str().unwrap();
    assert!(
        url.starts_with("https://relay.example.ru/subsecret/u"),
        "{url}"
    );
    assert!(url.ends_with("?format=clash"), "{url}");
    assert!(v["quotaBytes"].as_u64().is_some());
    assert!(v["usedBytes"].as_u64().is_some());
    assert!(v["enabled"].is_boolean());
    let reset = v["resetAt"].as_str().unwrap();
    assert!(reset.ends_with("T21:00:00Z"), "{reset}");
}

#[tokio::test]
async fn auth_required_and_profile_required() {
    let app = vpn_app(fake_sui(Shared::default()).await, TOKEN);
    for m in ["GET", "POST"] {
        let (st, _, v) = call(&app, req(m, None)).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert_app_error(&v, "authRequired");
        let (st, _, v) = call(&app, req(m, Some("bad"))).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert_app_error(&v, "authExpired");
        let (st, _, v) = call(&app, req(m, Some("noprofile"))).await;
        assert_eq!(st, StatusCode::UNAUTHORIZED);
        assert_app_error(&v, "authExpired");
    }
}

#[tokio::test]
async fn feature_off_is_503() {
    for m in ["GET", "POST"] {
        let (st, _, v) = call(&app(), req(m, Some("alice"))).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
        assert_app_error(&v, "network");
    }
}

#[tokio::test]
async fn provision_is_idempotent_and_get_reports_usage() {
    let panel = Shared::default();
    let app = vpn_app(fake_sui(panel.clone()).await, TOKEN);

    let (st, _, v) = call(&app, req("GET", Some("alice"))).await;
    assert_eq!(st, StatusCode::NOT_FOUND);
    assert_app_error(&v, "notFound");

    let (st, _, a1) = call(&app, req("POST", Some("alice"))).await;
    assert_eq!(st, StatusCode::OK, "{a1}");
    assert_vpn_access(&a1);
    assert_eq!(a1["quotaBytes"], 256 * GIB);
    assert_eq!(a1["usedBytes"], 0);
    assert_eq!(a1["enabled"], true);

    let (st, _, a2) = call(&app, req("POST", Some("alice"))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(a1["subscriptionUrl"], a2["subscriptionUrl"]);
    let (st, _, g) = call(&app, req("GET", Some("alice"))).await;
    assert_eq!(st, StatusCode::OK);
    assert_eq!(g["subscriptionUrl"], a1["subscriptionUrl"]);

    let (st, _, b) = call(&app, req("POST", Some("bob"))).await;
    assert_eq!(st, StatusCode::OK);
    assert_ne!(b["subscriptionUrl"], a1["subscriptionUrl"]);

    {
        let p = panel.lock().unwrap();
        assert_eq!(p.creates, 2);
        let s = &p.saves[0];
        assert_eq!(s["remark"], "mc-0000000000000000000000000000000a");
        assert_eq!(s["inbounds"], json!([1, 2]));
        assert_eq!(s["volume"], 256 * GIB);
        assert_eq!(s["enable"], true);
        let name = s["name"].as_str().unwrap();
        // The subscription secret is random, not derived from the public UUID.
        assert!(name.starts_with('u') && name.len() == 33 && !name.contains("00000000000"));
        assert_eq!(s["config"]["vless"]["flow"], "xtls-rprx-vision");
        assert_eq!(s["config"]["vless"]["name"], name);
        assert!(s["config"]["hysteria2"]["password"].as_str().unwrap().len() >= 32);
    }

    // Usage and depletion come from the panel counters.
    panel.lock().unwrap().clients[0]["up"] = json!(200 * GIB);
    panel.lock().unwrap().clients[0]["down"] = json!(56 * GIB);
    let (_, _, g) = call(&app, req("GET", Some("alice"))).await;
    assert_eq!(g["usedBytes"], 256 * GIB);
    assert_eq!(g["enabled"], false);
    panel.lock().unwrap().clients[0]["down"] = json!(0);
    panel.lock().unwrap().clients[0]["enable"] = json!(false);
    let (_, _, g) = call(&app, req("GET", Some("alice"))).await;
    assert_eq!(g["enabled"], false);
}

#[tokio::test]
async fn backend_failures_are_503() {
    // Wrong token: s-ui answers HTTP 200 with success:false.
    let app = vpn_app(fake_sui(Shared::default()).await, "wrong");
    for m in ["GET", "POST"] {
        let (st, _, v) = call(&app, req(m, Some("alice"))).await;
        assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE, "{v}");
        assert_app_error(&v, "network");
        assert!(!v.to_string().contains("wrong"));
    }
    // Unreachable panel.
    let app = vpn_app("http://127.0.0.1:1/panel/apiv2".into(), TOKEN);
    let (st, _, v) = call(&app, req("POST", Some("alice"))).await;
    assert_eq!(st, StatusCode::SERVICE_UNAVAILABLE);
    assert_app_error(&v, "network");
}

#[tokio::test]
async fn provision_is_rate_limited_per_profile() {
    let app = vpn_app(fake_sui(Shared::default()).await, TOKEN);
    for _ in 0..3 {
        let (st, _, _) = call(&app, req("POST", Some("alice"))).await;
        assert_eq!(st, StatusCode::OK);
    }
    let (st, h, v) = call(&app, req("POST", Some("alice"))).await;
    assert_eq!(st, StatusCode::TOO_MANY_REQUESTS);
    assert_app_error(&v, "rateLimited");
    assert!(h.get("retry-after").is_some());
    // Another profile is unaffected.
    let (st, _, _) = call(&app, req("POST", Some("bob"))).await;
    assert_eq!(st, StatusCode::OK);
}
