//! JVM proxy properties for `launcherAndGame` mode.
//!
//! They cover `java.net` sockets and `HttpURLConnection` (authlib, skins,
//! realms, mod HTTP calls). Netty NIO channels used for server connections
//! ignore them, so multiplayer traffic itself stays direct (no TUN by design).

/// Hosts that must stay direct: loopback (P2P tunnel ports) and LAN.
/// Java wildcards only work at the ends, so 172.16/12 is spelled out.
fn non_proxy() -> String {
    let mut hosts = vec![
        "localhost".to_owned(),
        "127.*".into(),
        "[::1]".into(),
        "10.*".into(),
        "192.168.*".into(),
        "169.254.*".into(),
    ];
    hosts.extend((16..=31).map(|n| format!("172.{n}.*")));
    hosts.push("*.local".into());
    hosts.join("|")
}

pub(crate) fn jvm_proxy_args(port: u16) -> Vec<String> {
    let host = "127.0.0.1";
    let skip = non_proxy();
    vec![
        format!("-DsocksProxyHost={host}"),
        format!("-DsocksProxyPort={port}"),
        format!("-Dhttp.proxyHost={host}"),
        format!("-Dhttp.proxyPort={port}"),
        format!("-Dhttps.proxyHost={host}"),
        format!("-Dhttps.proxyPort={port}"),
        format!("-Dhttp.nonProxyHosts={skip}"),
        format!("-DsocksNonProxyHosts={skip}"),
        "-Djava.net.useSystemProxies=false".to_owned(),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn args_point_to_mixed_port() {
        let a = jvm_proxy_args(51234);
        for want in [
            "-DsocksProxyHost=127.0.0.1",
            "-DsocksProxyPort=51234",
            "-Dhttp.proxyPort=51234",
            "-Dhttps.proxyHost=127.0.0.1",
            "-Dhttps.proxyPort=51234",
        ] {
            assert!(a.iter().any(|x| x == want), "{want}");
        }
        assert!(a
            .iter()
            .any(|x| x.starts_with("-Dhttp.nonProxyHosts=localhost|127.*")));
        let skip = non_proxy();
        for h in ["10.*", "192.168.*", "172.16.*", "172.31.*"] {
            assert!(skip.split('|').any(|x| x == h), "{h}");
        }
        assert!(!skip.contains("172.32."));
    }
}
