//! Generated mihomo configuration. Emitted as JSON, which mihomo's YAML
//! loader accepts (JSON is a subset of YAML 1.2).

use std::net::IpAddr;

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

/// Per-app TUN routing (`VpnMode::Apps`).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct TunRules {
    /// `PROCESS-NAME` values sent through [`GROUP`]; everything else is direct.
    pub processes: Vec<String>,
    /// Extra hosts that must never go through the tunnel (our hub/relay).
    /// VPN node servers are always added from the proxies.
    pub bypass_hosts: Vec<String>,
}

/// Targets that always bypass the proxy (game-mode LAN, loopback).
const DIRECT_CIDRS: &[&str] = &[
    "127.0.0.0/8",
    "10.0.0.0/8",
    "172.16.0.0/12",
    "192.168.0.0/16",
    "169.254.0.0/16",
    "100.64.0.0/10",
    "224.0.0.0/4",
    "::1/128",
    "fc00::/7",
    "fe80::/10",
];

/// Process names of mihomo itself: its own traffic must never loop.
const SELF_PROCESSES: &[&str] = &["mihomo.exe", "mihomo"];

/// Upstream resolvers for TUN mode (plain DNS, sent direct). Hijacked system
/// queries are answered with real IPs (`redir-host`), so apps outside the
/// VPN keep working exactly as without it.
const TUN_NAMESERVERS: &[&str] = &["1.1.1.1", "8.8.8.8"];

/// A value usable inside a comma-separated mihomo rule.
pub(crate) fn rule_safe(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && !s
            .chars()
            .any(|c| c == ',' || c.is_control() || c == '/' || c == '\\')
}

fn cidr_rule(cidr: &str) -> String {
    let kind = if cidr.contains(':') {
        "IP-CIDR6"
    } else {
        "IP-CIDR"
    };
    format!("{kind},{cidr},DIRECT,no-resolve")
}

/// Rules for a direct-by-default TUN: LAN, VPN servers and our own hosts
/// first (loop protection), then the selected programs, then `MATCH,DIRECT`.
fn tun_rules(proxies: &[ProxyMap], tun: &TunRules) -> Vec<String> {
    let mut rules: Vec<String> = DIRECT_CIDRS.iter().map(|c| cidr_rule(c)).collect();
    rules.push("DOMAIN-SUFFIX,local,DIRECT".into());
    let mut hosts: Vec<String> = Vec::new();
    let servers = proxies
        .iter()
        .filter_map(|p| p.get("server").and_then(Value::as_str));
    for h in servers.chain(tun.bypass_hosts.iter().map(String::as_str)) {
        let h = h.trim().trim_start_matches('[').trim_end_matches(']');
        if rule_safe(h) && !hosts.iter().any(|x| x.eq_ignore_ascii_case(h)) {
            hosts.push(h.to_owned());
        }
    }
    for h in &hosts {
        rules.push(match h.parse::<IpAddr>() {
            Ok(IpAddr::V4(ip)) => cidr_rule(&format!("{ip}/32")),
            Ok(IpAddr::V6(ip)) => cidr_rule(&format!("{ip}/128")),
            Err(_) => format!("DOMAIN,{},DIRECT", h.to_ascii_lowercase()),
        });
    }
    for p in SELF_PROCESSES {
        rules.push(format!("PROCESS-NAME,{p},DIRECT"));
    }
    let mut seen: Vec<&str> = Vec::new();
    for p in &tun.processes {
        let p = p.trim();
        if rule_safe(p) && !SELF_PROCESSES.contains(&p) && !seen.contains(&p) {
            seen.push(p);
            rules.push(format!("PROCESS-NAME,{p},{GROUP}"));
        }
    }
    rules.push("MATCH,DIRECT".into());
    rules
}

/// Full config: loopback-only mixed-port and controller, no GEO data, one
/// selector group with `selected` (if present) first so it is the default
/// choice. Without `tun`: no TUN, no DNS server, everything via the group.
/// With `tun`: per-app TUN routing (see [`tun_rules`]).
pub(crate) fn render(
    params: &RunParams<'_>,
    proxies: &[ProxyMap],
    selected: Option<&str>,
    tun: Option<&TunRules>,
) -> String {
    let mut names: Vec<&str> = proxies.iter().map(name_of).collect();
    if let Some(pos) = selected.and_then(|s| names.iter().position(|n| *n == s)) {
        let s = names.remove(pos);
        names.insert(0, s);
    }
    let rules = match tun {
        Some(t) => tun_rules(proxies, t),
        None => {
            let mut r: Vec<String> = DIRECT_CIDRS.iter().map(|c| cidr_rule(c)).collect();
            r.push("DOMAIN-SUFFIX,local,DIRECT".into());
            r.push(format!("MATCH,{GROUP}"));
            r
        }
    };
    let (tun_cfg, dns_cfg, sniffer, find_process) = match tun {
        Some(_) => (
            json!({
                "enable": true,
                "stack": "mixed",
                "auto-route": true,
                "auto-detect-interface": true,
                "strict-route": false,
                "dns-hijack": ["any:53", "tcp://any:53"],
            }),
            json!({
                "enable": true,
                "ipv6": true,
                "enhanced-mode": "redir-host",
                "use-hosts": true,
                "use-system-hosts": true,
                "nameserver": TUN_NAMESERVERS,
            }),
            // Recovers domains for proxied flows so the node resolves them.
            json!({
                "enable": true,
                "parse-pure-ip": true,
                "override-destination": true,
                "sniff": {
                    "HTTP": { "ports": [80, "8080-8880"] },
                    "TLS": { "ports": [443, 8443] },
                    "QUIC": { "ports": [443, 8443] },
                },
            }),
            "strict",
        ),
        None => (
            json!({ "enable": false }),
            json!({ "enable": false }),
            json!({ "enable": false }),
            "off",
        ),
    };

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
        "find-process-mode": find_process,
        "geo-auto-update": false,
        "unified-delay": true,
        "tcp-concurrent": true,
        "profile": { "store-selected": false, "store-fake-ip": false },
        "dns": dns_cfg,
        "tun": tun_cfg,
        "sniffer": sniffer,
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

    fn params() -> RunParams<'static> {
        RunParams {
            mixed_port: 51001,
            controller_port: 51002,
            secret: "s3cr3t",
        }
    }

    #[test]
    fn renders_loopback_only_config() {
        let proxies = vec![
            parse_link("trojan://pw@a.example:443#A").unwrap(),
            parse_link("trojan://pw@b.example:443#B").unwrap(),
        ];
        let text = render(&params(), &proxies, Some("B"), None);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["mixed-port"], 51001);
        assert_eq!(v["bind-address"], "127.0.0.1");
        assert_eq!(v["allow-lan"], false);
        assert_eq!(v["external-controller"], "127.0.0.1:51002");
        assert_eq!(v["secret"], "s3cr3t");
        assert_eq!(v["tun"]["enable"], false);
        assert_eq!(v["dns"]["enable"], false);
        assert_eq!(v["find-process-mode"], "off");
        assert_eq!(v["proxies"].as_array().unwrap().len(), 2);
        assert_eq!(v["proxy-groups"][0]["name"], GROUP);
        assert_eq!(v["proxy-groups"][0]["proxies"], json!(["B", "A"]));
        let rules = v["rules"].as_array().unwrap();
        assert_eq!(rules.last().unwrap(), &json!("MATCH,LC-PROXY"));
        assert!(rules.contains(&json!("IP-CIDR,127.0.0.0/8,DIRECT,no-resolve")));
        assert!(rules.contains(&json!("IP-CIDR6,::1/128,DIRECT,no-resolve")));
        assert!(!rules
            .iter()
            .any(|r| r.as_str().unwrap().contains("PROCESS")));
        // No well-known ports leak in from defaults.
        for key in ["port", "socks-port", "redir-port", "tproxy-port"] {
            assert!(v.get(key).is_none(), "{key}");
        }

        // Unknown selection keeps the subscription order.
        let text = render(&params(), &proxies, Some("missing"), None);
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["proxy-groups"][0]["proxies"], json!(["A", "B"]));
    }

    #[test]
    fn renders_per_app_tun() {
        let proxies = vec![
            parse_link("trojan://pw@vpn.example:443#A").unwrap(),
            parse_link("trojan://pw@203.0.113.7:443#B").unwrap(),
            parse_link("trojan://pw@[2001:db8::1]:443#C").unwrap(),
        ];
        let tun = TunRules {
            processes: vec![
                "Telegram.exe".into(),
                "launcher.exe".into(),
                "javaw.exe".into(),
                "Telegram.exe".into(),
                "evil.exe,DIRECT\nMATCH".into(),
                "mihomo.exe".into(),
            ],
            bypass_hosts: vec!["relay.example.ru".into(), "VPN.example".into()],
        };
        let text = render(&params(), &proxies, None, Some(&tun));
        let v: Value = serde_json::from_str(&text).unwrap();
        assert_eq!(v["tun"]["enable"], true);
        assert_eq!(v["tun"]["stack"], "mixed");
        assert_eq!(v["tun"]["auto-route"], true);
        assert_eq!(v["tun"]["strict-route"], false);
        assert_eq!(v["tun"]["dns-hijack"], json!(["any:53", "tcp://any:53"]));
        assert_eq!(v["dns"]["enable"], true);
        assert_eq!(v["dns"]["enhanced-mode"], "redir-host");
        assert_eq!(v["find-process-mode"], "strict");
        // Listeners stay loopback-only.
        assert_eq!(v["bind-address"], "127.0.0.1");
        assert_eq!(v["allow-lan"], false);

        let rules: Vec<&str> = v["rules"]
            .as_array()
            .unwrap()
            .iter()
            .map(|r| r.as_str().unwrap())
            .collect();
        let pos = |r: &str| {
            rules
                .iter()
                .position(|x| *x == r)
                .unwrap_or_else(|| panic!("{r}"))
        };
        // Loop protection before any app rule; default is direct.
        let first_app = pos("PROCESS-NAME,Telegram.exe,LC-PROXY");
        for r in [
            "IP-CIDR,192.168.0.0/16,DIRECT,no-resolve",
            "DOMAIN,vpn.example,DIRECT",
            "IP-CIDR,203.0.113.7/32,DIRECT,no-resolve",
            "IP-CIDR6,2001:db8::1/128,DIRECT,no-resolve",
            "DOMAIN,relay.example.ru,DIRECT",
            "PROCESS-NAME,mihomo.exe,DIRECT",
        ] {
            assert!(pos(r) < first_app, "{r}");
        }
        assert!(pos("PROCESS-NAME,launcher.exe,LC-PROXY") > first_app);
        assert!(pos("PROCESS-NAME,javaw.exe,LC-PROXY") > first_app);
        assert_eq!(*rules.last().unwrap(), "MATCH,DIRECT");
        // Deduplicated; injection attempts and mihomo itself never proxied.
        assert_eq!(rules.iter().filter(|r| r.contains("Telegram")).count(), 1);
        assert_eq!(
            rules.iter().filter(|r| r.contains("vpn.example")).count(),
            1
        );
        assert!(!rules.iter().any(|r| r.contains("evil")));
        assert!(!rules.contains(&"PROCESS-NAME,mihomo.exe,LC-PROXY"));
        assert_eq!(rules.iter().filter(|r| r.starts_with("MATCH")).count(), 1);
    }
}
