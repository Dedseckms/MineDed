//! Manual smoke test against the real pinned mihomo (downloads ~22 MB from
//! GitHub). Not run in CI: `cargo test -p lc-vpn --test live_mihomo -- --ignored`.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::sync::Arc;

use lc_http::{Downloader, DownloaderConfig, HttpDownloader};
use lc_runtime::{NullSink, Paths};
use lc_types::vpn::VpnMode;
use lc_vpn::{MihomoVpn, VpnController};

#[tokio::test]
#[ignore = "downloads mihomo from GitHub"]
async fn start_select_stop() {
    let root = std::env::temp_dir().join("lc-vpn-live");
    let paths = Paths::new(&root);
    let dl: Arc<dyn Downloader> =
        Arc::new(HttpDownloader::new(DownloaderConfig::default()).unwrap());
    let vpn = MihomoVpn::new(paths, dl);

    // Unreachable documentation-range nodes: mihomo must still start.
    let sub = root.join("sub.txt");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        &sub,
        "trojan://pw@192.0.2.1:443#A\nvless://u@192.0.2.2:443?security=tls#B\n",
    )
    .unwrap();
    vpn.import_subscription_file(sub.to_str().unwrap())
        .await
        .unwrap();
    vpn.refresh(&NullSink::default()).await.unwrap();

    let st = vpn.set_mode(VpnMode::LauncherAndGame).await.unwrap();
    assert!(st.running, "{st:?}");
    let proxy = vpn.proxy_url().unwrap();
    assert_eq!(proxy.host_str(), Some("127.0.0.1"));
    let port = proxy.port().unwrap();
    assert!(!(7890..=7899).contains(&port));

    vpn.select("B").await.unwrap();
    let nodes = vpn.test_latency().await.unwrap();
    assert!(nodes.iter().any(|n| n.selected && n.name == "B"));

    // Crash → supervisor restarts it (new ports allowed).
    let pid_file = root.join("sidecars").join("mihomo").join("mihomo.pid");
    let pid_text = std::fs::read_to_string(&pid_file).unwrap();
    let pid: u32 = pid_text.lines().next().unwrap().trim().parse().unwrap();
    let mut sys = sysinfo::System::new();
    let spid = sysinfo::Pid::from_u32(pid);
    sys.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[spid]), true);
    assert!(sys.process(spid).unwrap().kill());
    let mut restarted = false;
    for _ in 0..40 {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        let st = vpn.state().await.unwrap();
        let new_pid = std::fs::read_to_string(&pid_file).unwrap_or_default();
        if st.running && new_pid.lines().next() != Some(pid.to_string().as_str()) {
            restarted = true;
            break;
        }
    }
    assert!(restarted, "mihomo was not restarted");
    let port = vpn.proxy_url().unwrap().port().unwrap();

    let st = vpn.set_mode(VpnMode::Off).await.unwrap();
    assert!(!st.running);
    assert!(vpn.proxy_url().is_none());
    assert!(std::net::TcpListener::bind(("127.0.0.1", port)).is_ok());
}
