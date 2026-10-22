//! JVM proxy properties for `launcherAndGame` mode.
//!
//! They cover `java.net` sockets and `HttpURLConnection` (authlib, skins,
//! realms, mod HTTP calls). Netty NIO channels used for server connections
//! ignore them, so multiplayer traffic itself stays direct (no TUN by design).

/// Hosts that must stay direct: loopback (P2P tunnel ports) and LAN.
const NON_PROXY: &str = "localhost|127.*|[::1]|10.*|192.168.*|*.local";

pub(crate) fn jvm_proxy_args(port: u16) -> Vec<String> {
    let host = "127.0.0.1";
    vec![
        format!("-DsocksProxyHost={host}"),
        format!("-DsocksProxyPort={port}"),
        format!("-Dhttp.proxyHost={host}"),
        format!("-Dhttp.proxyPort={port}"),
        format!("-Dhttps.proxyHost={host}"),
        format!("-Dhttps.proxyPort={port}"),
        format!("-Dhttp.nonProxyHosts={NON_PROXY}"),
        format!("-DsocksNonProxyHosts={NON_PROXY}"),
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
    }
}
