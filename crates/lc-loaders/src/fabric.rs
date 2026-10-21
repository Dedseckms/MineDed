//! Fabric and Quilt: the meta API serves a ready version json with `inheritsFrom`.

use std::sync::Arc;

use async_trait::async_trait;
use lc_http::Downloader;
use lc_runtime::ProgressSink;
use lc_types::version::{LoaderKind, LoaderVersion};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::Deserialize;

use crate::contract::{InstallCtx, LoaderInstaller, VersionProfileId};
use crate::libs::{library_requests, Library};
use crate::stage::StageSink;
use crate::util::{get_json, loader_err, parse_json, parse_url, safe_join, write_json};

pub(crate) fn check_token(what: &str, s: &str) -> AppResult<()> {
    let ok = !s.is_empty()
        && s.len() <= 128
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'));
    if ok {
        Ok(())
    } else {
        Err(AppError::new(ErrorCode::InvalidInput, format!("bad {what}")).detail(what, s))
    }
}

/// Installer for meta-API loaders (Fabric, Quilt).
#[derive(Clone)]
pub struct FabricLikeInstaller {
    dl: Arc<dyn Downloader>,
    kind: LoaderKind,
    meta: String,
    /// Prefix of profile ids: `fabric-loader` / `quilt-loader`.
    id_prefix: &'static str,
}

#[derive(Debug, Deserialize)]
struct LoaderEntry {
    loader: LoaderInfo,
}

#[derive(Debug, Deserialize)]
struct LoaderInfo {
    version: String,
    #[serde(default)]
    stable: Option<bool>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct ProfileHead {
    id: String,
    #[serde(default)]
    inherits_from: Option<String>,
    #[serde(default)]
    libraries: Vec<Library>,
}

impl FabricLikeInstaller {
    pub fn fabric(dl: Arc<dyn Downloader>) -> Self {
        Self::with_meta(dl, LoaderKind::Fabric, "https://meta.fabricmc.net/v2")
    }

    pub fn quilt(dl: Arc<dyn Downloader>) -> Self {
        Self::with_meta(dl, LoaderKind::Quilt, "https://meta.quiltmc.org/v3")
    }

    /// Custom meta base (tests, mirrors). `dl` is used for `versions()`;
    /// `install()` uses the downloader from `InstallCtx`.
    pub fn with_meta(dl: Arc<dyn Downloader>, kind: LoaderKind, meta: &str) -> Self {
        Self {
            dl,
            kind,
            meta: meta.trim_end_matches('/').to_owned(),
            id_prefix: if kind == LoaderKind::Quilt {
                "quilt-loader"
            } else {
                "fabric-loader"
            },
        }
    }

    fn parse_versions(&self, mc: &str, entries: Vec<LoaderEntry>) -> Vec<LoaderVersion> {
        let mut out: Vec<LoaderVersion> = entries
            .into_iter()
            .map(|e| {
                let v = e.loader.version;
                let stable = e.loader.stable.unwrap_or_else(|| {
                    let l = v.to_ascii_lowercase();
                    !(l.contains("beta") || l.contains("alpha") || l.contains("pre"))
                });
                LoaderVersion {
                    loader: self.kind,
                    version: v,
                    mc: mc.to_owned(),
                    stable,
                    recommended: false,
                }
            })
            .collect();
        if let Some(first) = out.iter_mut().find(|v| v.stable) {
            first.recommended = true;
        }
        out
    }
}

#[async_trait]
impl LoaderInstaller for FabricLikeInstaller {
    fn kind(&self) -> LoaderKind {
        self.kind
    }

    async fn versions(&self, mc: &str) -> AppResult<Vec<LoaderVersion>> {
        check_token("mc", mc)?;
        let url = format!("{}/versions/loader/{mc}", self.meta);
        let entries: Vec<LoaderEntry> = get_json(self.dl.as_ref(), &url, "loader list").await?;
        Ok(self.parse_versions(mc, entries))
    }

    async fn install(
        &self,
        ctx: &InstallCtx,
        mc: &str,
        loader_version: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<VersionProfileId> {
        check_token("mc", mc)?;
        check_token("loader", loader_version)?;
        sink.check_cancelled()?;
        let expected_id = format!("{}-{loader_version}-{mc}", self.id_prefix);
        let url = format!(
            "{}/versions/loader/{mc}/{loader_version}/profile/json",
            self.meta
        );
        let fetched = ctx
            .downloader
            .get_bytes(&parse_url(&url)?, lc_http::ResourceKind::Loader)
            .await;
        let (json, id) = match fetched {
            Ok(bytes) => {
                let json: serde_json::Value = parse_json(&bytes, "loader profile")?;
                let head: ProfileHead = parse_json(&bytes, "loader profile")?;
                check_token("profile id", &head.id)?;
                if head.inherits_from.as_deref() != Some(mc) {
                    return Err(loader_err("loader profile inherits from another version")
                        .detail("expected", mc)
                        .detail("actual", head.inherits_from.unwrap_or_default()));
                }
                let id = head.id;
                let path = profile_path(ctx, &id)?;
                write_json(&path, &json).await?;
                (head.libraries, id)
            }
            Err(e) => {
                // Offline repair: keep using an already installed profile.
                let path = profile_path(ctx, &expected_id)?;
                match tokio::fs::read(&path).await {
                    Ok(bytes) => {
                        let head: ProfileHead = parse_json(&bytes, "loader profile")?;
                        (head.libraries, head.id)
                    }
                    Err(_) => return Err(e),
                }
            }
        };
        sink.check_cancelled()?;
        let reqs = library_requests(&json, &ctx.paths.libraries())?;
        let staged = StageSink::new(sink, "stages.loaderLibraries");
        ctx.downloader.fetch_many(reqs, &staged).await?;
        Ok(id)
    }
}

pub(crate) fn profile_path(ctx: &InstallCtx, id: &str) -> AppResult<std::path::PathBuf> {
    check_token("profile id", id)?;
    safe_join(&ctx.paths.versions(), &format!("{id}/{id}.json"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_from_meta_fixture() {
        let i = FabricLikeInstaller::fabric(Arc::new(lc_http::StubDownloader));
        let entries: Vec<LoaderEntry> = serde_json::from_str(
            r#"[{"loader":{"version":"0.16.10","stable":false}},
                {"loader":{"version":"0.16.9","stable":true}},
                {"loader":{"version":"0.16.8","stable":true}}]"#,
        )
        .unwrap();
        let v = i.parse_versions("1.21.1", entries);
        assert_eq!(v.len(), 3);
        assert!(!v[0].recommended);
        assert!(v[1].recommended && v[1].stable);
        assert!(!v[2].recommended);

        let q = FabricLikeInstaller::quilt(Arc::new(lc_http::StubDownloader));
        let entries: Vec<LoaderEntry> = serde_json::from_str(
            r#"[{"loader":{"version":"0.27.0-beta.1"}},{"loader":{"version":"0.26.4"}}]"#,
        )
        .unwrap();
        let v = q.parse_versions("1.21.1", entries);
        assert!(!v[0].stable);
        assert!(v[1].recommended);
    }

    #[tokio::test]
    async fn install_writes_profile_and_libraries() {
        let dir = tempfile::tempdir().unwrap();
        let paths = lc_runtime::Paths::new(dir.path());
        let dl = Arc::new(crate::mock::MockDl::default());
        let lib = b"loader-jar".to_vec();
        let profile = serde_json::json!({
            "id": "fabric-loader-0.16.9-1.21.1", "inheritsFrom": "1.21.1",
            "mainClass": "net.fabricmc.loader.impl.launch.knot.KnotClient",
            "libraries": [{"name": "net.fabricmc:fabric-loader:0.16.9",
                "url": "https://maven.fabricmc.net/", "sha1": crate::util::sha1_hex(&lib)}]
        });
        dl.put(
            "https://meta.fabricmc.net/v2/versions/loader/1.21.1/0.16.9/profile/json",
            profile.to_string(),
        );
        dl.put(
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar",
            lib,
        );
        let ctx = InstallCtx {
            paths: paths.clone(),
            downloader: dl.clone(),
            java: "java".into(),
        };
        let i = FabricLikeInstaller::fabric(dl.clone());
        let sink = lc_runtime::NullSink::default();
        let id = i.install(&ctx, "1.21.1", "0.16.9", &sink).await.unwrap();
        assert_eq!(id, "fabric-loader-0.16.9-1.21.1");
        assert!(paths
            .versions()
            .join(&id)
            .join(format!("{id}.json"))
            .exists());
        assert!(paths
            .libraries()
            .join("net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar")
            .exists());
        // Offline repair reuses the stored profile.
        dl.files.lock().unwrap().clear();
        dl.put(
            "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.9/fabric-loader-0.16.9.jar",
            b"loader-jar".to_vec(),
        );
        assert_eq!(
            i.install(&ctx, "1.21.1", "0.16.9", &sink).await.unwrap(),
            id
        );
        // Profile for another Minecraft version is rejected.
        dl.put(
            "https://meta.fabricmc.net/v2/versions/loader/1.20.1/0.16.9/profile/json",
            profile.to_string(),
        );
        assert!(i.install(&ctx, "1.20.1", "0.16.9", &sink).await.is_err());
    }

    #[test]
    fn tokens_are_validated() {
        assert!(check_token("mc", "1.21.1").is_ok());
        assert!(check_token("mc", "../x").is_err());
        assert!(check_token("mc", "").is_err());
    }
}
