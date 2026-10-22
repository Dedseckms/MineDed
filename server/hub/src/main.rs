use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use hub::auth::DraslValidator;
use hub::rooms::Limits;
use hub::{AppState, Config};

fn env_parse<T: std::str::FromStr>(key: &str, default: T) -> T {
    std::env::var(key)
        .ok()
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(default)
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
    };

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
