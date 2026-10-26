use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hub::auth::DraslValidator;
use hub::rooms::Limits;
use hub::vpn::VpnConfig;
use hub::{AppState, Config};

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
}

fn env_nonempty(key: &str) -> Option<String> {
    std::env::var(key)
        .ok()
        .map(|v| v.trim().to_owned())
        .filter(|v| !v.is_empty())
}

/// VPN is on only when the API URL, token and public subscription base are all set.
fn vpn_config() -> Option<VpnConfig> {
    Some(VpnConfig {
        api_url: env_nonempty("HUB_VPN_API_URL")?,
        api_token: env_nonempty("HUB_VPN_API_TOKEN")?,
        sub_base: env_nonempty("HUB_VPN_SUB_BASE")?,
        quota_bytes: env_parse("HUB_VPN_QUOTA_GB", 256u64).saturating_mul(1 << 30),
    })
}

#[tokio::main]
async fn main() -> std::io::Result<()> {
    let addr: SocketAddr = env_parse("HUB_ADDR", SocketAddr::from(([0, 0, 0, 0], 8080)));
    let drasl = std::env::var("HUB_DRASL_URL")
        .unwrap_or_else(|_| "http://127.0.0.1:25585/authlib-injector".into());
    let defaults = Config::default().limits;
    let cfg = Config {
        relay_urls: std::env::var("HUB_RELAY_URLS")
            .unwrap_or_default()
            .split(',')
            .map(|s| s.trim().to_owned())
            .filter(|s| !s.is_empty())
            .collect(),
        limits: Limits {
            ttl: Duration::from_secs(env_parse("HUB_ROOM_TTL_SECS", defaults.ttl.as_secs())),
            max_rooms: env_parse("HUB_MAX_ROOMS", defaults.max_rooms),
            max_rooms_per_user: env_parse("HUB_MAX_ROOMS_PER_USER", defaults.max_rooms_per_user),
        },
        trust_proxy: env_parse("HUB_TRUST_PROXY", false),
        vpn: vpn_config(),
    };
    eprintln!("vpn: {}", if cfg.vpn.is_some() { "on" } else { "off" });

    let state = AppState::new(cfg, Arc::new(DraslValidator::new(&drasl)));
    state.spawn_reaper();
    let listener = tokio::net::TcpListener::bind(addr).await?;
    eprintln!("hub {} listening on {addr}", env!("CARGO_PKG_VERSION"));
    let app = hub::router(state).into_make_service_with_connect_info::<SocketAddr>();
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            #[cfg(unix)]
            {
                use tokio::signal::unix::{signal, SignalKind};
                if let Ok(mut term) = signal(SignalKind::terminate()) {
                    tokio::select! {
                        _ = term.recv() => {}
                        _ = tokio::signal::ctrl_c() => {}
                    }
                    return;
                }
            }
            let _ = tokio::signal::ctrl_c().await;
        })
        .await
}
