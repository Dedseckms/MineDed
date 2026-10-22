//! Subscription parsing: mihomo/Clash YAML, base64 link lists and plain
//! `vless://` / `trojan://` / `ss://` / `vmess://` / `hysteria2://` / `tuic://`
//! link lists → mihomo proxy definitions.
//!
//! Proxy definitions hold credentials (UUIDs, passwords): they are never
//! logged and have no `Debug` output of their own beyond `serde_json`'s, so
//! callers must not log them either.

use std::collections::{HashMap, HashSet};

use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use base64::engine::DecodePaddingMode;
use base64::Engine;
use lc_types::{AppError, AppResult, ErrorCode};
use percent_encoding::percent_decode_str;
use serde_json::{json, Map, Value};
use url::Url;
use yaml_rust2::{Yaml, YamlLoader};

/// One mihomo proxy (`proxies:` entry). Always has string `name` and `type`.
pub(crate) type ProxyMap = Map<String, Value>;

/// Names mihomo reserves for built-in policies, plus our own group.
const RESERVED: &[&str] = &[
    "DIRECT",
    "REJECT",
    "REJECT-DROP",
    "PASS",
    "COMPATIBLE",
    "GLOBAL",
    crate::config::GROUP,
];

/// Parses any supported subscription format. Fails with `InvalidInput` when
/// nothing usable is found. Node names are made unique.
pub(crate) fn parse(bytes: &[u8]) -> AppResult<Vec<ProxyMap>> {
    let text = String::from_utf8_lossy(bytes);
    let text = text.trim_start_matches('\u{feff}').trim();
    if text.is_empty() {
        return Err(invalid("subscription is empty"));
    }
    let proxies = if looks_like_yaml(text) {
        parse_yaml(text)?
    } else if text.contains("://") {
        parse_links(text)
    } else {
        let decoded =
            decode_base64(text).ok_or_else(|| invalid("unrecognised subscription format"))?;
        if looks_like_yaml(&decoded) {
            parse_yaml(&decoded)?
        } else {
            parse_links(&decoded)
        }
    };
    let proxies = dedupe_names(proxies);
    if proxies.is_empty() {
        return Err(invalid("subscription has no supported nodes"));
    }
    Ok(proxies)
}

/// `name` and `type` of a proxy.
pub(crate) fn name_of(p: &ProxyMap) -> &str {
    p.get("name").and_then(Value::as_str).unwrap_or_default()
}

pub(crate) fn type_of(p: &ProxyMap) -> &str {
    p.get("type").and_then(Value::as_str).unwrap_or_default()
}

fn invalid(msg: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, msg)
}

fn looks_like_yaml(text: &str) -> bool {
    text.starts_with('{')
        || text
            .lines()
            .any(|l| l.starts_with("proxies:") || l.starts_with("\"proxies\":"))
}

fn decode_base64(text: &str) -> Option<String> {
    let compact: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let cfg = GeneralPurposeConfig::new().with_decode_padding_mode(DecodePaddingMode::Indifferent);
    let engines = [
        GeneralPurpose::new(&base64::alphabet::STANDARD, cfg),
        GeneralPurpose::new(&base64::alphabet::URL_SAFE, cfg),
    ];
    engines
        .iter()
        .find_map(|e| e.decode(compact.as_bytes()).ok())
        .and_then(|b| String::from_utf8(b).ok())
}

// ---------- YAML ----------

fn parse_yaml(text: &str) -> AppResult<Vec<ProxyMap>> {
    let docs =
        YamlLoader::load_from_str(text).map_err(|_| invalid("subscription YAML is malformed"))?;
    let Some(Yaml::Hash(root)) = docs.into_iter().next() else {
        return Err(invalid("subscription YAML has no top-level mapping"));
    };
    let Some(Yaml::Array(list)) = root.get(&Yaml::String("proxies".into())) else {
        return Err(invalid("subscription YAML has no `proxies` list"));
    };
    Ok(list
        .iter()
        .filter_map(|item| match yaml_to_json(item) {
            Value::Object(mut m) => {
                let name = match m.get("name") {
                    Some(Value::String(s)) => s.clone(),
                    Some(Value::Number(n)) => n.to_string(),
                    _ => return None,
                };
                m.insert("name".into(), Value::String(name));
                m.get("type").and_then(Value::as_str)?;
                Some(m)
            }
            _ => None,
        })
        .collect())
}

fn yaml_to_json(y: &Yaml) -> Value {
    match y {
        Yaml::Real(s) => s
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map_or_else(|| Value::String(s.clone()), Value::Number),
        Yaml::Integer(i) => json!(i),
        Yaml::String(s) => Value::String(s.clone()),
        Yaml::Boolean(b) => Value::Bool(*b),
        Yaml::Array(a) => Value::Array(a.iter().map(yaml_to_json).collect()),
        Yaml::Hash(h) => Value::Object(
            h.iter()
                .filter_map(|(k, v)| {
                    let key = match k {
                        Yaml::String(s) | Yaml::Real(s) => s.clone(),
                        Yaml::Integer(i) => i.to_string(),
                        Yaml::Boolean(b) => b.to_string(),
                        _ => return None,
                    };
                    Some((key, yaml_to_json(v)))
                })
                .collect(),
        ),
        Yaml::Alias(_) | Yaml::Null | Yaml::BadValue => Value::Null,
    }
}

// ---------- links ----------

fn parse_links(text: &str) -> Vec<ProxyMap> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .filter_map(parse_link)
        .collect()
}

/// One share link → proxy; `None` for unsupported or malformed links.
pub(crate) fn parse_link(line: &str) -> Option<ProxyMap> {
    let (scheme, _) = line.split_once("://")?;
    match scheme.to_ascii_lowercase().as_str() {
        "vless" => vless(&Url::parse(line).ok()?),
        "trojan" => trojan(&Url::parse(line).ok()?),
        "hysteria2" | "hy2" => hysteria2(&Url::parse(line).ok()?),
        "tuic" => tuic(&Url::parse(line).ok()?),
        "ss" => shadowsocks(line),
        "vmess" => vmess(line),
        _ => None,
    }
}

fn decode(s: &str) -> String {
    percent_decode_str(s).decode_utf8_lossy().into_owned()
}

fn query(url: &Url) -> HashMap<String, String> {
    url.query_pairs()
        .map(|(k, v)| (k.into_owned(), v.into_owned()))
        .filter(|(_, v)| !v.is_empty())
        .collect()
}

fn host(url: &Url) -> Option<String> {
    let h = url
        .host_str()?
        .trim_start_matches('[')
        .trim_end_matches(']');
    (!h.is_empty()).then(|| h.to_owned())
}

fn node_name(url: &Url, server: &str, port: u16) -> String {
    url.fragment()
        .map(decode)
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("{server}:{port}"))
}

fn base(kind: &str, name: String, server: String, port: u16) -> ProxyMap {
    let mut m = Map::new();
    m.insert("name".into(), Value::String(name));
    m.insert("type".into(), Value::String(kind.into()));
    m.insert("server".into(), Value::String(server));
    m.insert("port".into(), json!(port));
    m.insert("udp".into(), Value::Bool(true));
    m
}

fn truthy(v: Option<&String>) -> bool {
    v.is_some_and(|v| v == "1" || v.eq_ignore_ascii_case("true"))
}

fn alpn(q: &HashMap<String, String>) -> Option<Value> {
    q.get("alpn").map(|a| {
        Value::Array(
            a.split(',')
                .map(|s| Value::String(s.trim().to_owned()))
                .collect(),
        )
    })
}

/// `type`/`path`/`host`/`serviceName`/`mode` → mihomo `network` + `*-opts`.
fn apply_transport(m: &mut ProxyMap, q: &HashMap<String, String>) {
    let net = q.get("type").map_or("tcp", String::as_str);
    let path = q.get("path").cloned();
    let host_hdr = q.get("host").cloned();
    match net {
        "ws" | "httpupgrade" => {
            let mut o = Map::new();
            o.insert("path".into(), json!(path.unwrap_or_else(|| "/".into())));
            if let Some(h) = host_hdr {
                o.insert("headers".into(), json!({ "Host": h }));
            }
            if net == "httpupgrade" {
                o.insert("v2ray-http-upgrade".into(), Value::Bool(true));
            }
            m.insert("network".into(), json!("ws"));
            m.insert("ws-opts".into(), Value::Object(o));
        }
        "grpc" => {
            m.insert("network".into(), json!("grpc"));
            let svc = q.get("serviceName").cloned().unwrap_or_default();
            m.insert("grpc-opts".into(), json!({ "grpc-service-name": svc }));
        }
        "h2" | "http" => {
            m.insert("network".into(), json!("h2"));
            let mut o = Map::new();
            if let Some(h) = host_hdr {
                o.insert("host".into(), json!([h]));
            }
            o.insert("path".into(), json!(path.unwrap_or_else(|| "/".into())));
            m.insert("h2-opts".into(), Value::Object(o));
        }
        "xhttp" | "splithttp" => {
            m.insert("network".into(), json!("xhttp"));
            let mut o = Map::new();
            o.insert("path".into(), json!(path.unwrap_or_else(|| "/".into())));
            if let Some(h) = host_hdr {
                o.insert("host".into(), json!(h));
            }
            if let Some(mode) = q.get("mode") {
                o.insert("mode".into(), json!(mode));
            }
            m.insert("xhttp-opts".into(), Value::Object(o));
        }
        _ => {}
    }
}

/// TLS / REALITY fields shared by vless (and vmess via its own keys).
fn apply_tls(m: &mut ProxyMap, q: &HashMap<String, String>, sni_key: &str) {
    if let Some(sni) = q.get("sni").or_else(|| q.get("peer")) {
        m.insert(sni_key.into(), json!(sni));
    }
    if let Some(fp) = q.get("fp") {
        m.insert("client-fingerprint".into(), json!(fp));
    }
    if let Some(a) = alpn(q) {
        m.insert("alpn".into(), a);
    }
    if truthy(q.get("allowInsecure")) || truthy(q.get("insecure")) {
        m.insert("skip-cert-verify".into(), Value::Bool(true));
    }
    if let Some(pbk) = q.get("pbk") {
        let mut r = Map::new();
        r.insert("public-key".into(), json!(pbk));
        if let Some(sid) = q.get("sid") {
            r.insert("short-id".into(), json!(sid));
        }
        m.insert("reality-opts".into(), Value::Object(r));
    }
}

fn vless(url: &Url) -> Option<ProxyMap> {
    let server = host(url)?;
    let port = url.port().filter(|p| *p > 0)?;
    let uuid = decode(url.username());
    if uuid.is_empty() {
        return None;
    }
    let q = query(url);
    let mut m = base("vless", node_name(url, &server, port), server, port);
    m.insert("uuid".into(), json!(uuid));
    if let Some(flow) = q.get("flow") {
        m.insert("flow".into(), json!(flow));
    }
    if let Some(enc) = q.get("encryption").filter(|e| *e != "none") {
        m.insert("encryption".into(), json!(enc));
    }
    let security = q.get("security").map_or("none", String::as_str);
    if matches!(security, "tls" | "reality" | "xtls") {
        m.insert("tls".into(), Value::Bool(true));
        apply_tls(&mut m, &q, "servername");
    }
    apply_transport(&mut m, &q);
    Some(m)
}

fn trojan(url: &Url) -> Option<ProxyMap> {
    let server = host(url)?;
    let port = url.port().unwrap_or(443);
    let password = decode(url.username());
    if password.is_empty() {
        return None;
    }
    let q = query(url);
    let mut m = base("trojan", node_name(url, &server, port), server, port);
    m.insert("password".into(), json!(password));
    apply_tls(&mut m, &q, "sni");
    apply_transport(&mut m, &q);
    Some(m)
}

fn hysteria2(url: &Url) -> Option<ProxyMap> {
    let server = host(url)?;
    let port = url.port().unwrap_or(443);
    let mut password = decode(url.username());
    if let Some(p) = url.password() {
        password = format!("{password}:{}", decode(p));
    }
    let q = query(url);
    let mut m = base("hysteria2", node_name(url, &server, port), server, port);
    if !password.is_empty() {
        m.insert("password".into(), json!(password));
    }
    if let Some(sni) = q.get("sni") {
        m.insert("sni".into(), json!(sni));
    }
    if truthy(q.get("insecure")) {
        m.insert("skip-cert-verify".into(), Value::Bool(true));
    }
    if let Some(obfs) = q.get("obfs").filter(|o| *o != "none") {
        m.insert("obfs".into(), json!(obfs));
        if let Some(pw) = q.get("obfs-password") {
            m.insert("obfs-password".into(), json!(pw));
        }
    }
    if let Some(ports) = q.get("mport") {
        m.insert("ports".into(), json!(ports));
    }
    if let Some(a) = alpn(&q) {
        m.insert("alpn".into(), a);
    }
    Some(m)
}

fn tuic(url: &Url) -> Option<ProxyMap> {
    let server = host(url)?;
    let port = url.port().filter(|p| *p > 0)?;
    let uuid = decode(url.username());
    let password = url.password().map(decode)?;
    let q = query(url);
    let mut m = base("tuic", node_name(url, &server, port), server, port);
    m.insert("uuid".into(), json!(uuid));
    m.insert("password".into(), json!(password));
    if let Some(cc) = q.get("congestion_control") {
        m.insert("congestion-controller".into(), json!(cc));
    }
    if let Some(mode) = q.get("udp_relay_mode") {
        m.insert("udp-relay-mode".into(), json!(mode));
    }
    if let Some(sni) = q.get("sni") {
        m.insert("sni".into(), json!(sni));
    }
    if truthy(q.get("allow_insecure")) || truthy(q.get("insecure")) {
        m.insert("skip-cert-verify".into(), Value::Bool(true));
    }
    if let Some(a) = alpn(&q) {
        m.insert("alpn".into(), a);
    }
    Some(m)
}

/// SIP002 (`ss://base64(method:pass)@host:port#name`, userinfo may also be
/// percent-encoded plain text) and legacy `ss://base64(method:pass@host:port)`.
/// Links with plugins are skipped (not supported yet).
fn shadowsocks(line: &str) -> Option<ProxyMap> {
    let rest = &line[line.find("://")? + 3..];
    let (body, fragment) = match rest.split_once('#') {
        Some((b, f)) => (b, Some(f)),
        None => (rest, None),
    };
    let (main, query_str) = match body.split_once('?') {
        Some((m, q)) => (m.trim_end_matches('/'), Some(q)),
        None => (body.trim_end_matches('/'), None),
    };
    let main = if main.contains('@') {
        main.to_owned()
    } else {
        decode_base64(main)?
    };
    let mut rebuilt = format!("ss://{main}");
    if let Some(q) = query_str {
        rebuilt.push_str("/?");
        rebuilt.push_str(q);
    }
    if let Some(f) = fragment {
        rebuilt.push('#');
        rebuilt.push_str(f);
    }
    let url = Url::parse(&rebuilt).ok()?;
    let q = query(&url);
    if q.contains_key("plugin") {
        return None;
    }
    let server = host(&url)?;
    let port = url.port().filter(|p| *p > 0)?;
    let (method, password) = match url.password() {
        Some(p) => (decode(url.username()), decode(p)),
        None => {
            let user = decode(url.username());
            match user.split_once(':') {
                Some((m, p)) => (m.to_owned(), p.to_owned()),
                None => {
                    let d = decode_base64(&user)?;
                    let (m, p) = d.split_once(':')?;
                    (m.to_owned(), p.to_owned())
                }
            }
        }
    };
    if method.is_empty() {
        return None;
    }
    let mut m = base("ss", node_name(&url, &server, port), server, port);
    m.insert("cipher".into(), json!(method));
    m.insert("password".into(), json!(password));
    Some(m)
}

/// `vmess://base64(json)` (v2rayN format).
fn vmess(line: &str) -> Option<ProxyMap> {
    let raw = &line[line.find("://")? + 3..];
    let v: Value = serde_json::from_str(&decode_base64(raw)?).ok()?;
    let s = |k: &str| match v.get(k) {
        Some(Value::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(Value::Number(n)) => Some(n.to_string()),
        _ => None,
    };
    let server = s("add")?;
    let port: u16 = s("port")?.parse().ok().filter(|p| *p > 0)?;
    let name = s("ps").unwrap_or_else(|| format!("{server}:{port}"));
    let mut m = base("vmess", name, server, port);
    m.insert("uuid".into(), json!(s("id")?));
    m.insert(
        "alterId".into(),
        json!(s("aid").and_then(|a| a.parse::<u32>().ok()).unwrap_or(0)),
    );
    m.insert(
        "cipher".into(),
        json!(s("scy").unwrap_or_else(|| "auto".into())),
    );
    // Reuse the link transport/TLS mapping through an equivalent query map.
    let mut q: HashMap<String, String> = HashMap::new();
    for (from, to) in [
        ("net", "type"),
        ("path", "path"),
        ("host", "host"),
        ("sni", "sni"),
        ("alpn", "alpn"),
        ("fp", "fp"),
    ] {
        if let Some(val) = s(from) {
            q.insert(to.into(), val);
        }
    }
    if q.get("type").is_some_and(|t| t == "grpc") {
        if let Some(p) = q.get("path").cloned() {
            q.insert("serviceName".into(), p);
        }
    }
    if s("tls").is_some_and(|t| t == "tls") {
        m.insert("tls".into(), Value::Bool(true));
        apply_tls(&mut m, &q, "servername");
    }
    apply_transport(&mut m, &q);
    Some(m)
}

/// Makes names unique and keeps them clear of mihomo's built-in policy names.
fn dedupe_names(proxies: Vec<ProxyMap>) -> Vec<ProxyMap> {
    let mut seen: HashSet<String> = HashSet::new();
    proxies
        .into_iter()
        .map(|mut p| {
            let mut name = name_of(&p).trim().to_owned();
            if name.is_empty() {
                name = "node".into();
            }
            if RESERVED.iter().any(|r| r.eq_ignore_ascii_case(&name)) {
                name = format!("{name} (node)");
            }
            let mut candidate = name.clone();
            let mut i = 2;
            while !seen.insert(candidate.clone()) {
                candidate = format!("{name} ({i})");
                i += 1;
            }
            p.insert("name".into(), Value::String(candidate));
            p
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const VLESS: &str = "vless://11111111-2222-3333-4444-555555555555@example.com:443?type=tcp&security=reality&pbk=PUBKEY&sid=ab12&sni=www.microsoft.com&fp=chrome&flow=xtls-rprx-vision&encryption=none#%D0%9D%D0%B8%D0%B4%D0%B5%D1%80%D0%BB%D0%B0%D0%BD%D0%B4%D1%8B";

    #[test]
    fn vless_reality() {
        let p = parse_link(VLESS).unwrap();
        assert_eq!(name_of(&p), "Нидерланды");
        assert_eq!(type_of(&p), "vless");
        assert_eq!(p["server"], "example.com");
        assert_eq!(p["port"], 443);
        assert_eq!(p["uuid"], "11111111-2222-3333-4444-555555555555");
        assert_eq!(p["tls"], true);
        assert_eq!(p["servername"], "www.microsoft.com");
        assert_eq!(p["client-fingerprint"], "chrome");
        assert_eq!(p["flow"], "xtls-rprx-vision");
        assert_eq!(p["reality-opts"]["public-key"], "PUBKEY");
        assert_eq!(p["reality-opts"]["short-id"], "ab12");
        assert!(p.get("encryption").is_none());
        assert!(p.get("network").is_none());
    }

    #[test]
    fn vless_ws_tls_ipv6() {
        let p = parse_link(
            "vless://u@[2001:db8::1]:8443?type=ws&security=tls&path=%2Fws%3Fed%3D2048&host=cdn.example&sni=cdn.example&allowInsecure=1",
        )
        .unwrap();
        assert_eq!(p["server"], "2001:db8::1");
        assert_eq!(name_of(&p), "2001:db8::1:8443");
        assert_eq!(p["network"], "ws");
        assert_eq!(p["ws-opts"]["path"], "/ws?ed=2048");
        assert_eq!(p["ws-opts"]["headers"]["Host"], "cdn.example");
        assert_eq!(p["skip-cert-verify"], true);
    }

    #[test]
    fn other_link_kinds() {
        let t =
            parse_link("trojan://p%40ss@t.example:443?sni=s.example&type=grpc&serviceName=svc#T")
                .unwrap();
        assert_eq!((type_of(&t), &t["password"]), ("trojan", &json!("p@ss")));
        assert_eq!(t["grpc-opts"]["grpc-service-name"], "svc");

        let h = parse_link("hy2://secret@h.example:8443/?sni=h.example&obfs=salamander&obfs-password=x&insecure=1#H")
            .unwrap();
        assert_eq!(type_of(&h), "hysteria2");
        assert_eq!(h["password"], "secret");
        assert_eq!(h["obfs"], "salamander");

        // SIP002 with base64 userinfo; legacy fully-encoded form.
        let ss = parse_link("ss://YWVzLTI1Ni1nY206cGFzcw@s.example:8388#SS").unwrap();
        assert_eq!(
            (&ss["cipher"], &ss["password"]),
            (&json!("aes-256-gcm"), &json!("pass"))
        );
        let legacy = format!(
            "ss://{}#L",
            base64::engine::general_purpose::STANDARD
                .encode("chacha20-ietf-poly1305:pw@1.2.3.4:443")
        );
        let ss = parse_link(&legacy).unwrap();
        assert_eq!(ss["server"], "1.2.3.4");
        assert_eq!(ss["cipher"], "chacha20-ietf-poly1305");
        // Plain 2022 userinfo.
        let ss = parse_link("ss://2022-blake3-aes-128-gcm:a%2Bb%3D@s.example:443#P").unwrap();
        assert_eq!(ss["password"], "a+b=");
        assert!(
            parse_link("ss://YWVzLTI1Ni1nY206cGFzcw@s.example:8388/?plugin=v2ray-plugin#X")
                .is_none()
        );

        let vm = json!({"v":"2","ps":"VM","add":"v.example","port":"443","id":"uuid-1","aid":"0","net":"ws","path":"/p","host":"v.example","tls":"tls","sni":"v.example"});
        let link = format!(
            "vmess://{}",
            base64::engine::general_purpose::STANDARD.encode(vm.to_string())
        );
        let v = parse_link(&link).unwrap();
        assert_eq!((name_of(&v), type_of(&v)), ("VM", "vmess"));
        assert_eq!(v["tls"], true);
        assert_eq!(v["ws-opts"]["path"], "/p");

        let tu =
            parse_link("tuic://uuid:pw@t.example:443?congestion_control=bbr&alpn=h3#TU").unwrap();
        assert_eq!(tu["congestion-controller"], "bbr");
        assert_eq!(tu["alpn"], json!(["h3"]));

        assert!(parse_link("socks://x").is_none());
        assert!(parse_link("vless://@host:443").is_none());
        assert!(parse_link("vless://u@host").is_none());
    }

    #[test]
    fn base64_list_and_plain_list() {
        let list = format!("{VLESS}\ntrojan://pw@t.example:443#T\nunknown://x\n");
        let plain = parse(list.as_bytes()).unwrap();
        assert_eq!(plain.len(), 2);
        for engine in [
            base64::engine::general_purpose::STANDARD,
            base64::engine::general_purpose::URL_SAFE_NO_PAD,
        ] {
            let encoded = engine.encode(&list);
            // Line-wrapped base64 is common.
            let wrapped: String = encoded
                .as_bytes()
                .chunks(76)
                .map(|c| String::from_utf8_lossy(c).into_owned() + "\r\n")
                .collect();
            assert_eq!(parse(wrapped.as_bytes()).unwrap(), plain);
        }
    }

    #[test]
    fn clash_yaml() {
        let yaml = "\u{feff}mixed-port: 7890\nproxies:\n  - {name: A, type: ss, server: 1.1.1.1, port: 443, cipher: aes-128-gcm, password: x}\n  - name: A\n    type: vless\n    server: b.example\n    port: 443\n    uuid: u\n    tls: true\n    reality-opts:\n      public-key: k\n  - name: 42\n    type: trojan\n    server: c\n    port: 1\n    password: p\n  - name: DIRECT\n    type: http\n    server: d\n    port: 2\n  - {type: ss}\nproxy-groups: []\n";
        let p = parse(yaml.as_bytes()).unwrap();
        let names: Vec<&str> = p.iter().map(name_of).collect();
        assert_eq!(names, ["A", "A (2)", "42", "DIRECT (node)"]);
        assert_eq!(p[0]["port"], 443);
        assert_eq!(p[1]["tls"], true);
        assert_eq!(p[1]["reality-opts"]["public-key"], "k");
    }

    #[test]
    fn rejects_garbage() {
        for bad in [
            "",
            "   ",
            "hello world",
            "proxies: []",
            "proxies: [",
            "aGVsbG8=",
        ] {
            let err = parse(bad.as_bytes()).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
        }
    }
}
