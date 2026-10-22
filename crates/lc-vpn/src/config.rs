//! Generated mihomo configuration. Emitted as JSON, which mihomo's YAML
//! loader accepts (JSON is a subset of YAML 1.2).

use serde_json::{json, Value};

use crate::subscription::{name_of, ProxyMap};

/// The single selector group all traffic goes through.
pub(crate) const GROUP: &str = "LC-PROXY";

/// Local ports and controller secret of one mihomo run.
pub(crate) struct RunParams<'a> {
    pub mixed_port: u16,
    pub controller_port: u16,
    pub secret: &'a str,
}

/// Targets that always bypass the proxy (game-mode LAN, loopback).
const DIRECT_CIDRS: &[&str] = &[
    "127.0.0.0/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "100.64.0.0/10",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];

/// Full config: loopback-only listeners, no TUN, no DNS server, no GEO data,
/// one selector group with `selected` (if present) first so it is the
/// default choice.
pub(crate) fn render(
    params: &RunParams<'_>,
    proxies: &[ProxyMap],
    selected: Option<&str>,
) -> String {
    let mut names: Vec<&str> = proxies.iter().map(name_of).collect();
    if let Some(pos) = selected.and_then(|s| names.iter().position(|n| *n == s)) {
        let s = names.remove(pos);
        names.insert(0, s);
    }
    let mut rules: Vec<String> = DIRECT_CIDRS
        .iter()
        .map(|c| {
            let kind = if c.contains(':') {
                "IP-CIDR6"
            } else {
                "IP-CIDR"
            };
            format!("{kind},{c},DIRECT,no-resolve")
        })
        .collect();
    rules.push("DOMAIN-SUFFIX,local,DIRECT".into());
    rules.push(format!("MATCH,{GROUP}"));

    let cfg = json!({
        "mixed-port": params.mixed_port,
        "bind-address": "127.0.0.1",
        "allow-lan": false,
        "mode": "rule",
        // Node addresses must not end up in log files.
        "log-level": "silent",
        "ipv6": true,
        "external-controller": format!("127.0.0.1:{}", params.controller_port),
        "secret": params.secret,
        "find-process-mode": "off",
        "geo-auto-update": false,
        "unified-delay": true,
        "tcp-concurrent": true,
        "profile": { "store-selected": false, "store-fake-ip": false },
        "dns": { "enable": false },
        "tun": { "enable": false },
        "proxies": proxies.iter().cloned().map(Value::Object).collect::<Vec<_>>(),
        "proxy-groups": [{ "name": GROUP, "type": "select", "proxies": names }],
        "rules": rules,
    });
    // Serialising a `Value` cannot fail.
    serde_json::to_string_pretty(&cfg).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subscription::parse_link;

    #[test]
    fn renders_loopback_only_config() {
        let proxies = vec![
            parse_link("trojan://pw@a.example:443#A").unwrap(),
            parse_link("trojan://pw@b.example:443#B").unwrap(),
        ];
        let text = render(
            &RunParams {
                mixed_port: 51001,
                controller_port: 51002,
                secret: "s3cr3t",
            },
            &proxies,
            Some("B"),
        );
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mixed-port"], 51001);
        assert_eq!(v["bind-address"], "127.0.0.1");
        assert_eq!(v["allow-lan"], false);
        assert_eq!(v["external-controller"], "127.0.0.1:51002");
        assert_eq!(v["secret"], "s3cr3t");
        assert_eq!(v["tun"]["enable"], false);
        assert_eq!(v["dns"]["enable"], false);
        assert_eq!(v["proxies"].as_array().unwrap().len(), 2);
        assert_eq!(v["proxy-groups"][0]["name"], GROUP);
        assert_eq!(v["proxy-groups"][0]["proxies"], json!(["B", "A"]));
        let rules = v["rules"].as_array().unwrap();
        assert_eq!(rules.last().unwrap(), &json!("MATCH,LC-PROXY"));
        assert!(rules.contains(&json!("IP-CIDR,127.0.0.0/8,DIRECT,no-resolve")));
        assert!(rules.contains(&json!("IP-CIDR6,::1/128,DIRECT,no-resolve")));
        // No well-known ports leak in from defaults.
        for key in ["port", "socks-port", "redir-port", "tproxy-port"] {
            assert!(v.get(key).is_none(), "{key}");
        }

        // Unknown selection keeps the subscription order.
        let text = render(
            &RunParams {
                mixed_port: 1,
                controller_port: 2,
                secret: "x",
            },
            &proxies,
            Some("missing"),
        );
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["proxy-groups"][0]["proxies"], json!(["A", "B"]));
    }
}
