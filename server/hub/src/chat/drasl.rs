//! Server-to-server Drasl v3 directory. Player UUID is not the owning user's UUID.
//! Protocol verified against unmojang/drasl v4.0.1 api.go (GPL-3.0).
use axum::body::Bytes;
use http_body_util::{BodyExt, Full, Limited};
use hyper::{header, Request};
use hyper_util::{
    client::legacy::{connect::HttpConnector, Client},
    rt::TokioExecutor,
};
use serde::Deserialize;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tokio::sync::Semaphore;

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Player {
    pub uuid: String,
    pub name: String,
    pub skin_url: Option<String>,
    pub user_uuid: String,
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct User {
    is_admin: bool,
    is_locked: bool,
    players: Vec<Player>,
}
#[derive(Clone)]
pub(super) struct Info {
    pub admin: bool,
    pub skin: Option<String>,
}
pub(super) struct Directory {
    url: Option<String>,
    token: Option<String>,
    client: Client<HttpConnector, Full<Bytes>>,
    cache: Mutex<std::collections::HashMap<String, (Instant, Info)>>,
    requests: Semaphore,
}
pub(super) struct PublicPlayer {
    pub uuid: String,
    pub name: String,
    pub skin: Option<String>,
}
impl Directory {
    pub fn from_env() -> Self {
        Self {
            url: std::env::var("HUB_DRASL_ADMIN_URL")
                .ok()
                .filter(|s| s.starts_with("http://"))
                .map(|s| s.trim_end_matches('/').into()),
            token: std::env::var("HUB_DRASL_ADMIN_TOKEN")
                .ok()
                .filter(|s| !s.is_empty()),
            client: Client::builder(TokioExecutor::new()).build_http(),
            cache: Mutex::new(Default::default()),
            requests: Semaphore::new(4),
        }
    }
    async fn get<T: serde::de::DeserializeOwned>(&self, path: &str) -> Option<T> {
        let _permit = self.requests.try_acquire().ok()?;
        let req = Request::builder()
            .uri(format!("{}{path}", self.url.as_ref()?))
            .header(
                header::AUTHORIZATION,
                format!("Bearer {}", self.token.as_ref()?),
            )
            .body(Full::new(Bytes::new()))
            .ok()?;
        tokio::time::timeout(Duration::from_secs(5), async {
            let response = self.client.request(req).await.ok()?;
            if response.status() != hyper::StatusCode::OK {
                return None;
            }
            let bytes = Limited::new(response.into_body(), 2 * 1024 * 1024)
                .collect()
                .await
                .ok()?
                .to_bytes();
            serde_json::from_slice(&bytes).ok()
        })
        .await
        .ok()
        .flatten()
    }
    pub async fn profile(&self, id: &str) -> Option<Info> {
        let id = super::store::uuid(id).ok()?;
        if let Some((until, info)) = self.cache.lock().ok()?.get(&id) {
            if *until > Instant::now() {
                return Some(info.clone());
            }
        }
        let player: Player = self
            .get(&format!(
                "/players/{}",
                uuid::Uuid::parse_str(&id).ok()?.hyphenated()
            ))
            .await?;
        if super::store::uuid(&player.uuid).ok()? != id {
            return None;
        }
        let owner = uuid::Uuid::parse_str(&player.user_uuid).ok()?;
        let user: User = self.get(&format!("/users/{}", owner.hyphenated())).await?;
        if !user
            .players
            .iter()
            .any(|p| super::store::uuid(&p.uuid).ok().as_deref() == Some(&id))
        {
            return None;
        }
        let info = Info {
            admin: user.is_admin && !user.is_locked,
            skin: player.skin_url,
        };
        let mut cache = self.cache.lock().ok()?;
        if cache.len() >= 10000 {
            cache.retain(|_, v| v.0 > Instant::now());
            if cache.len() >= 10000 {
                return Some(info);
            }
        }
        cache.insert(id, (Instant::now() + Duration::from_secs(30), info.clone()));
        Some(info)
    }
    pub async fn players(&self) -> Vec<PublicPlayer> {
        self.get::<Vec<Player>>("/players")
            .await
            .unwrap_or_default()
            .into_iter()
            .filter_map(|p| {
                let uuid = super::store::uuid(&p.uuid).ok()?;
                if p.name.is_empty()
                    || p.name.chars().count() > 16
                    || p.name.chars().any(char::is_control)
                {
                    return None;
                }
                Some(PublicPlayer {
                    uuid,
                    name: p.name,
                    skin: p.skin_url,
                })
            })
            .take(10000)
            .collect()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]
    use super::*;
    use axum::{extract::Path, http::HeaderMap, routing::get, Json, Router};
    use serde_json::json;
    #[tokio::test]
    async fn admin_comes_from_verified_owner_not_player_or_name() {
        let profile = "00000000-0000-0000-0000-000000000001";
        let owner = "00000000-0000-0000-0000-000000000099";
        let app=Router::new().route("/{mode}/players/{id}",get(move|Path((mode,id)):Path<(String,String)>,headers:HeaderMap|async move{
            assert_eq!(headers["authorization"],"Bearer server-only");assert_eq!(id,profile);
            Json(json!({"uuid":if mode=="wrong"{owner}else{profile},"userUuid":owner,"name":"Admin","skinUrl":null,"isAdmin":true}))
        })).route("/{mode}/users/{id}",get(move|Path((mode,id)):Path<(String,String)>|async move{
            assert_eq!(id,owner);
            Json(json!({"isAdmin":true,"isLocked":mode=="locked","players":[{"uuid":if mode=="mismatch"{owner}else{profile},"userUuid":owner,"name":"Player","skinUrl":null}]}))
        }));
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let task = tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        for (mode, expected) in [
            ("good", Some(true)),
            ("locked", Some(false)),
            ("mismatch", None),
            ("wrong", None),
        ] {
            let mut directory = Directory::from_env();
            directory.url = Some(format!("http://{addr}/{mode}"));
            directory.token = Some("server-only".into());
            assert_eq!(
                directory.profile(profile).await.map(|p| p.admin),
                expected,
                "{mode}"
            );
        }
        task.abort();
    }
}
