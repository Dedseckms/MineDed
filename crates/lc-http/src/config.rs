use std::fmt;
use std::time::Duration;

use lc_types::settings::{ProxyCfg, Settings};
use url::Url;

/// Default `User-Agent` for every launcher request.
pub fn default_user_agent() -> String {
    format!(
        "MineUP/{} (+https://dedseckms.ru)",
        env!("CARGO_PKG_VERSION")
    )
}

/// Tuning knobs for [`crate::HttpDownloader`].
///
/// `Debug` is implemented by hand so the proxy password never reaches logs.
#[derive(Clone)]
pub struct DownloaderConfig {
    pub user_agent: String,
    /// Maximum simultaneous downloads across all hosts.
    pub max_parallel: usize,
    /// Maximum simultaneous downloads to one host.
    pub max_per_host: usize,
    pub connect_timeout: Duration,
    /// Maximum silence between two reads of a response body.
    pub read_timeout: Duration,
    /// Whole-request timeout for `get_bytes` (small metadata requests only).
    pub request_timeout: Duration,
    /// Attempts per candidate URL before moving on to the next one.
    pub attempts_per_url: u32,
    /// First backoff delay; doubles on every retry (capped at 8 s).
    pub backoff_base: Duration,
    /// Put mirrors before official hosts.
    pub prefer_mirrors: bool,
    /// Use BMCLAPI as a mirror for Mojang hosts.
    pub use_bmclapi: bool,
    /// Initial manual proxy. `Route::Direct` never uses any proxy.
    pub proxy: Option<ProxyCfg>,
    /// Last-resort proxy for `Route::Auto` / `Route::Proxied` after the
    /// built-in VPN and the manual proxy.
    pub system_proxy: SystemProxyMode,
    /// Maximum body size accepted by `get_bytes` (default 32 MiB).
    pub max_body_bytes: u64,
    /// How long a host stays "proxy first" after `Route::Auto` needed the
    /// proxy for it (default 30 min).
    pub proxy_first_ttl: Duration,
}

/// Where the OS-level fallback proxy comes from.
#[derive(Clone, PartialEq, Eq, Default)]
pub enum SystemProxyMode {
    /// Environment variables, then (Windows) the WinINet user settings;
    /// re-read periodically so toggling e.g. FlClash takes effect.
    #[default]
    Detect,
    /// Never use a system proxy.
    Off,
    /// Fixed URL (tests, embedding). Loopback/private targets still bypass it.
    Fixed(Url),
}

impl fmt::Debug for SystemProxyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Detect => f.write_str("Detect"),
            Self::Off => f.write_str("Off"),
            Self::Fixed(u) => write!(f, "Fixed({})", crate::sysproxy::redacted(u)),
        }
    }
}

impl Default for DownloaderConfig {
    fn default() -> Self {
        Self {
            user_agent: default_user_agent(),
            max_parallel: 16,
            max_per_host: 8,
            connect_timeout: Duration::from_secs(10),
            read_timeout: Duration::from_secs(30),
            request_timeout: Duration::from_secs(60),
            attempts_per_url: 3,
            backoff_base: Duration::from_millis(500),
            prefer_mirrors: false,
            use_bmclapi: true,
            proxy: None,
            system_proxy: SystemProxyMode::Detect,
            max_body_bytes: 32 * 1024 * 1024,
            proxy_first_ttl: Duration::from_secs(30 * 60),
        }
    }
}

impl DownloaderConfig {
    /// Defaults overridden by the user-facing settings
    /// (`max_parallel_downloads`, `prefer_mirrors`, `proxy`).
    pub fn from_settings(settings: &Settings) -> Self {
        let max_parallel = usize::try_from(settings.max_parallel_downloads)
            .unwrap_or(16)
            .clamp(1, 64);
        Self {
            max_parallel,
            max_per_host: max_parallel.min(8),
            prefer_mirrors: settings.prefer_mirrors,
            proxy: settings.proxy.clone(),
            ..Self::default()
        }
    }
}

impl fmt::Debug for DownloaderConfig {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DownloaderConfig")
            .field("user_agent", &self.user_agent)
            .field("max_parallel", &self.max_parallel)
            .field("max_per_host", &self.max_per_host)
            .field("connect_timeout", &self.connect_timeout)
            .field("read_timeout", &self.read_timeout)
            .field("request_timeout", &self.request_timeout)
            .field("attempts_per_url", &self.attempts_per_url)
            .field("backoff_base", &self.backoff_base)
            .field("prefer_mirrors", &self.prefer_mirrors)
            .field("use_bmclapi", &self.use_bmclapi)
            .field("proxy", &self.proxy.as_ref().map(redacted_proxy))
            .field("system_proxy", &self.system_proxy)
            .field("max_body_bytes", &self.max_body_bytes)
            .field("proxy_first_ttl", &self.proxy_first_ttl)
            .finish()
    }
}

/// `kind://host:port` with credentials omitted, safe for logs.
pub(crate) fn redacted_proxy(p: &ProxyCfg) -> String {
    let auth = if p.username.is_some() { " (auth)" } else { "" };
    format!("{:?}://{}:{}{auth}", p.kind, p.host, p.port)
}

#[cfg(test)]
mod tests {
    use super::*;
    use lc_types::settings::ProxyKind;

    #[test]
    fn user_agent_format() {
        let ua = default_user_agent();
        assert!(ua.starts_with("MineUP/"));
        assert!(ua.ends_with("(+https://dedseckms.ru)"));
    }

    #[test]
    fn debug_hides_password() {
        let cfg = DownloaderConfig {
            proxy: Some(ProxyCfg {
                kind: ProxyKind::Socks5,
                host: "127.0.0.1".into(),
                port: 1080,
                username: Some("user".into()),
                password: Some("hunter2".into()),
            }),
            ..DownloaderConfig::default()
        };
        let s = format!("{cfg:?}");
        assert!(!s.contains("hunter2"));
        assert!(s.contains("1080"));
    }

    #[test]
    fn from_settings_clamps() {
        let mut s = Settings {
            max_parallel_downloads: 0,
            ..Settings::default()
        };
        assert_eq!(DownloaderConfig::from_settings(&s).max_parallel, 1);
        s.max_parallel_downloads = 4;
        s.prefer_mirrors = true;
        let c = DownloaderConfig::from_settings(&s);
        assert_eq!((c.max_parallel, c.max_per_host), (4, 4));
        assert!(c.prefer_mirrors);
    }
}
