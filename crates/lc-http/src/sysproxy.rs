//! OS-level proxy settings: the last fallback of `Route::Auto` / `Route::Proxied`
//! (after the built-in VPN and the manual proxy).
//!
//! Sources, in order: `HTTPS_PROXY` / `ALL_PROXY` / `HTTP_PROXY` (any case),
//! then on Windows the per-user WinINet settings (`ProxyEnable`,
//! `ProxyServer`, `ProxyOverride`) that tools such as FlClash or Clash Verge
//! set in "system proxy" mode.

use std::net::IpAddr;

use url::Url;

/// A detected system proxy plus the hosts that must bypass it.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct SystemProxy {
    pub url: Url,
    /// Lower-case patterns: `*.example.com`, `10.*`, `<local>`, `example.org`.
    pub bypass: Vec<String>,
}

impl std::fmt::Debug for SystemProxy {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SystemProxy")
            .field("url", &redacted(&self.url))
            .field("bypass", &self.bypass.len())
            .finish()
    }
}

impl SystemProxy {
    /// Loopback, private and link-local targets never go through the system
    /// proxy; neither do hosts matching the OS bypass list.
    pub fn bypasses(&self, url: &Url) -> bool {
        let Some(host) = url.host_str() else {
            return true;
        };
        let host = host.trim_start_matches('[').trim_end_matches(']');
        let host = host.to_ascii_lowercase();
        if is_local_host(&host) {
            return true;
        }
        self.bypass.iter().any(|p| matches_pattern(p, &host))
    }
}

/// `scheme://host:port` without credentials, safe for logs.
pub(crate) fn redacted(url: &Url) -> String {
    format!(
        "{}://{}:{}",
        url.scheme(),
        url.host_str().unwrap_or_default(),
        url.port_or_known_default().unwrap_or_default()
    )
}

/// Reads the current system proxy. Cheap enough to call every few seconds.
pub(crate) fn detect() -> Option<SystemProxy> {
    if let Some(p) = from_env(|k| std::env::var(k).ok()) {
        return Some(p);
    }
    #[cfg(windows)]
    {
        windows::detect()
    }
    #[cfg(not(windows))]
    {
        None
    }
}

/// Proxy from environment variables (`get` abstracts the environment for tests).
pub(crate) fn from_env(get: impl Fn(&str) -> Option<String>) -> Option<SystemProxy> {
    let var = |name: &str| {
        get(name)
            .or_else(|| get(&name.to_ascii_lowercase()))
            .map(|v| v.trim().to_owned())
            .filter(|v| !v.is_empty())
    };
    let raw = var("HTTPS_PROXY")
        .or_else(|| var("ALL_PROXY"))
        .or_else(|| var("HTTP_PROXY"))?;
    let url = parse_proxy_url(&raw, "http")?;
    let bypass = var("NO_PROXY")
        .map(|v| split_list(&v, ','))
        .unwrap_or_default();
    Some(SystemProxy { url, bypass })
}

/// Parses WinINet `ProxyServer` (`host:port` or `http=h:p;https=h:p;socks=h:p`)
/// and `ProxyOverride` (`;`-separated patterns).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn parse_wininet(server: &str, overrides: &str) -> Option<SystemProxy> {
    let server = server.trim();
    if server.is_empty() {
        return None;
    }
    let url = if server.contains('=') {
        let mut entries = std::collections::HashMap::new();
        for part in server.split(';') {
            if let Some((proto, addr)) = part.split_once('=') {
                let addr = addr.trim();
                if !addr.is_empty() {
                    entries.insert(proto.trim().to_ascii_lowercase(), addr.to_owned());
                }
            }
        }
        if let Some(a) = entries.get("https").or_else(|| entries.get("http")) {
            parse_proxy_url(a, "http")?
        } else {
            parse_proxy_url(entries.get("socks")?, "socks5h")?
        }
    } else {
        parse_proxy_url(server, "http")?
    };
    Some(SystemProxy {
        url,
        bypass: split_list(overrides, ';'),
    })
}

/// `host:port` or a full URL; `socks5://` becomes `socks5h://` so DNS is
/// resolved by the proxy (local DNS may be poisoned).
fn parse_proxy_url(raw: &str, default_scheme: &str) -> Option<Url> {
    let raw = raw.trim();
    let with_scheme = if raw.contains("://") {
        raw.to_owned()
    } else {
        format!("{default_scheme}://{raw}")
    };
    let mut url = Url::parse(&with_scheme).ok()?;
    match url.scheme() {
        "http" | "https" | "socks5h" => {}
        "socks5" | "socks" => {
            url = Url::parse(&with_scheme.replacen(url.scheme(), "socks5h", 1)).ok()?;
        }
        _ => return None,
    }
    url.host_str().filter(|h| !h.is_empty())?;
    url.port_or_known_default()?;
    Some(url)
}

fn split_list(v: &str, sep: char) -> Vec<String> {
    v.split(sep)
        .map(|s| s.trim().to_ascii_lowercase())
        .filter(|s| !s.is_empty())
        .collect()
}

fn is_local_host(host: &str) -> bool {
    if host == "localhost" || host.ends_with(".localhost") {
        return true;
    }
    match host.parse::<IpAddr>() {
        Ok(IpAddr::V4(v4)) => {
            v4.is_loopback() || v4.is_private() || v4.is_link_local() || v4.is_unspecified()
        }
        Ok(IpAddr::V6(v6)) => {
            let seg0 = v6.segments()[0];
            v6.is_loopback()
                || v6.is_unspecified()
                || (seg0 & 0xfe00) == 0xfc00
                || (seg0 & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

/// WinINet / `NO_PROXY` pattern match (`*` wildcard, leading-dot suffix,
/// `<local>` = names without dots).
fn matches_pattern(pattern: &str, host: &str) -> bool {
    match pattern {
        "*" => true,
        "<local>" => !host.contains('.') && host.parse::<IpAddr>().is_err(),
        p if p.starts_with('.') => host.ends_with(p) || host == &p[1..],
        p if p.contains('*') => glob(p, host),
        p => host == p || host.ends_with(&format!(".{p}")),
    }
}

fn glob(pattern: &str, text: &str) -> bool {
    let parts: Vec<&str> = pattern.split('*').collect();
    let mut rest = text;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            match rest.strip_prefix(part) {
                Some(r) => rest = r,
                None => return false,
            }
        } else if i == parts.len() - 1 {
            return rest.ends_with(part);
        } else {
            match rest.find(part) {
                Some(pos) => rest = &rest[pos + part.len()..],
                None => return false,
            }
        }
    }
    parts.last().is_some_and(|l| l.is_empty()) || rest.is_empty()
}

#[cfg(windows)]
mod windows {
    use winreg::enums::HKEY_CURRENT_USER;
    use winreg::RegKey;

    use super::{parse_wininet, SystemProxy};

    const KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Internet Settings";

    pub(super) fn detect() -> Option<SystemProxy> {
        let key = RegKey::predef(HKEY_CURRENT_USER).open_subkey(KEY).ok()?;
        let enabled: u32 = key.get_value("ProxyEnable").ok()?;
        if enabled == 0 {
            return None;
        }
        let server: String = key.get_value("ProxyServer").ok()?;
        let overrides: String = key.get_value("ProxyOverride").unwrap_or_default();
        parse_wininet(&server, &overrides)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn wininet_formats() {
        let p = parse_wininet("127.0.0.1:7890", "localhost;*.lan;<local>").unwrap();
        assert_eq!(p.url.as_str(), "http://127.0.0.1:7890/");
        assert!(p.bypasses(&u("http://printer/x")));
        assert!(p.bypasses(&u("http://nas.lan/x")));
        assert!(!p.bypasses(&u("https://meta.fabricmc.net/v2")));

        let p = parse_wininet("http=10.0.0.1:3128;https=10.0.0.2:3129;socks=s:1080", "").unwrap();
        assert_eq!(p.url.as_str(), "http://10.0.0.2:3129/");
        let p = parse_wininet("socks=127.0.0.1:1080", "").unwrap();
        assert_eq!(p.url.scheme(), "socks5h");
        assert!(parse_wininet("", "").is_none());
        assert!(parse_wininet("ftp=1.2.3.4:21", "").is_none());
    }

    #[test]
    fn env_order_and_no_proxy() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |k: &str| {
                pairs
                    .iter()
                    .find(|(n, _)| *n == k)
                    .map(|(_, v)| (*v).to_owned())
            }
        };
        let p = from_env(env(&[
            ("http_proxy", "http://h:1"),
            ("HTTPS_PROXY", "socks5://s:2"),
            ("NO_PROXY", "example.org, .corp"),
        ]))
        .unwrap();
        assert_eq!(p.url.as_str(), "socks5h://s:2");
        assert!(p.bypasses(&u("https://api.example.org/")));
        assert!(p.bypasses(&u("https://x.corp/")));
        assert!(!p.bypasses(&u("https://example.com/")));
        assert_eq!(
            from_env(env(&[("http_proxy", "h:3")]))
                .unwrap()
                .url
                .as_str(),
            "http://h:3/"
        );
        assert!(from_env(env(&[])).is_none());
    }

    #[test]
    fn local_targets_bypass() {
        let p = parse_wininet("127.0.0.1:7890", "").unwrap();
        for t in [
            "http://127.0.0.1:8080/",
            "http://localhost/",
            "http://192.168.1.5/",
            "http://10.1.2.3/",
            "http://[::1]:80/",
            "http://[fd00::1]/",
        ] {
            assert!(p.bypasses(&u(t)), "{t}");
        }
        assert!(!p.bypasses(&u("http://8.8.8.8/")));
    }

    #[test]
    fn glob_patterns() {
        assert!(glob("10.*", "10.1.2.3"));
        assert!(glob("*.zhimg.com", "pic.zhimg.com"));
        assert!(!glob("*.zhimg.com", "zhimg.com.evil"));
        assert!(glob("a*b*c", "aXXbYYc"));
        assert!(!glob("a*b", "ac"));
    }

    #[test]
    fn debug_redacts_credentials() {
        let p =
            from_env(|k| (k == "HTTPS_PROXY").then(|| "http://u:secret@h:8/".to_owned())).unwrap();
        assert!(!format!("{p:?}").contains("secret"));
    }
}
