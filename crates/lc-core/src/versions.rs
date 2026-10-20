//! Version manifest (cached) and version json store under `Paths::versions()`.

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use lc_http::{DownloadReq, Downloader, ResourceKind};
use lc_runtime::{NullSink, Paths, ProgressSink};
use lc_types::version::{VersionEntry, VersionFilter, VersionKind};
use lc_types::{AppError, AppResult, ErrorCode};
use tokio::sync::Mutex;

use crate::meta::{merge_inherited, ManifestVersion, VersionJson, VersionManifest, MANIFEST_URL};
use crate::util::{
    file_sha1, json_err, parse_url, read_json, safe_component, write_atomic, StageSink,
};

const MANIFEST_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_INHERIT_DEPTH: usize = 8;
pub(crate) const DEFAULT_JAVA_MAJOR: u32 = 8;

pub(crate) struct Versions {
    paths: Paths,
    dl: Arc<dyn Downloader>,
    manifest: Mutex<Option<(Instant, Arc<VersionManifest>)>>,
}

fn kind_of(s: &str) -> Option<VersionKind> {
    match s {
        "release" => Some(VersionKind::Release),
        "snapshot" => Some(VersionKind::Snapshot),
        "old_beta" => Some(VersionKind::OldBeta),
        "old_alpha" => Some(VersionKind::OldAlpha),
        _ => None,
    }
}

/// Applies `VersionFilter` to manifest entries (pure, for tests).
pub(crate) fn filter_entries(
    manifest: &VersionManifest,
    filter: &VersionFilter,
    java_major: impl Fn(&str) -> Option<u32>,
) -> Vec<VersionEntry> {
    let search = filter
        .search
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_lowercase);
    manifest
        .versions
        .iter()
        .filter_map(|v| {
            let kind = kind_of(&v.kind)?;
            if !filter.kinds.is_empty() && !filter.kinds.contains(&kind) {
                return None;
            }
            if let Some(s) = &search {
                if !v.id.to_lowercase().contains(s.as_str()) {
                    return None;
                }
            }
            Some(VersionEntry {
                id: v.id.clone(),
                kind,
                release_time: v.release_time.clone(),
                java_major: java_major(&v.id),
            })
        })
        .collect()
}

impl Versions {
    pub fn new(paths: Paths, dl: Arc<dyn Downloader>) -> Self {
        Self {
            paths,
            dl,
            manifest: Mutex::new(None),
        }
    }

    fn manifest_cache_path(&self) -> PathBuf {
        self.paths.cache().join("version_manifest_v2.json")
    }

    /// `versions/<id>/<id>.json`; `InvalidInput` for ids that are not a single safe component.
    pub fn json_path(&self, id: &str) -> AppResult<PathBuf> {
        safe_component(
            &safe_component(&self.paths.versions(), id)?,
            &format!("{id}.json"),
        )
    }

    /// Manifest from memory (fresh), network, or the disk cache when offline.
    pub async fn manifest(&self) -> AppResult<Arc<VersionManifest>> {
        let mut guard = self.manifest.lock().await;
        if let Some((at, m)) = guard.as_ref() {
            if at.elapsed() < MANIFEST_TTL {
                return Ok(m.clone());
            }
        }
        let url = parse_url(MANIFEST_URL)?;
        let cache = self.manifest_cache_path();
        let fresh = match self.dl.get_bytes(&url, ResourceKind::VersionMeta).await {
            Ok(bytes) => match serde_json::from_slice::<VersionManifest>(&bytes) {
                Ok(m) => {
                    // A failed cache write must not break listing.
                    let _ = write_atomic(&cache, &bytes).await;
                    Ok(m)
                }
                Err(e) => Err(json_err("version manifest", e)),
            },
            Err(e) => Err(e),
        };
        let manifest = match fresh {
            Ok(m) => m,
            Err(net_err) => match read_json::<VersionManifest>(&cache).await {
                Ok(m) => m,
                Err(_) => {
                    // Keep a stale in-memory copy rather than failing.
                    if let Some((_, m)) = guard.as_ref() {
                        return Ok(m.clone());
                    }
                    return Err(net_err);
                }
            },
        };
        let m = Arc::new(manifest);
        *guard = Some((Instant::now(), m.clone()));
        Ok(m)
    }

    async fn manifest_entry(&self, id: &str) -> Option<ManifestVersion> {
        let m = self.manifest().await.ok()?;
        m.versions.iter().find(|v| v.id == id).cloned()
    }

    pub async fn list(&self, filter: VersionFilter) -> AppResult<Vec<VersionEntry>> {
        let manifest = self.manifest().await?;
        let local = self.local_java_majors().await;
        Ok(filter_entries(&manifest, &filter, |id| {
            local
                .iter()
                .find(|(lid, _)| lid == id)
                .map(|(_, major)| *major)
        }))
    }

    /// Java majors of version jsons already on disk (no network).
    async fn local_java_majors(&self) -> Vec<(String, u32)> {
        let mut out = Vec::new();
        let Ok(mut rd) = tokio::fs::read_dir(self.paths.versions()).await else {
            return out;
        };
        while let Ok(Some(entry)) = rd.next_entry().await {
            let id = entry.file_name().to_string_lossy().into_owned();
            let Ok(path) = self.json_path(&id) else {
                continue;
            };
            if let Ok(v) = read_json::<VersionJson>(&path).await {
                if v.inherits_from.is_none() {
                    let major = v
                        .java_version
                        .map(|j| j.major_version)
                        .unwrap_or(DEFAULT_JAVA_MAJOR);
                    out.push((id, major));
                }
            }
        }
        out
    }

    /// Vanilla version json: verified against the manifest sha1 and stored in
    /// `versions/<id>/<id>.json`. Works offline when the file is already there.
    pub async fn ensure_vanilla_json(
        &self,
        id: &str,
        sink: &dyn ProgressSink,
    ) -> AppResult<VersionJson> {
        let path = self.json_path(id)?;
        let entry = self.manifest_entry(id).await;
        match entry {
            Some(entry) => {
                let local = file_sha1(&path).await?;
                let valid = match (&local, &entry.sha1) {
                    (Some(l), Some(want)) => l.eq_ignore_ascii_case(want),
                    (Some(_), None) => true,
                    (None, _) => false,
                };
                if !valid {
                    let req = DownloadReq {
                        urls: vec![parse_url(&entry.url)?],
                        sha1: entry.sha1.clone(),
                        size: None,
                        dest: path.clone(),
                        kind: ResourceKind::VersionMeta,
                    };
                    let staged = StageSink {
                        inner: sink,
                        stage: "stages.versionMeta",
                    };
                    self.dl.fetch_many(vec![req], &staged).await?;
                    if let (Some(got), Some(want)) = (file_sha1(&path).await?, &entry.sha1) {
                        if !got.eq_ignore_ascii_case(want) {
                            return Err(AppError::new(
                                ErrorCode::HashMismatch,
                                "version json hash mismatch",
                            )
                            .detail("expected", want.as_str())
                            .detail("actual", got)
                            .detail("path", path.to_string_lossy()));
                        }
                    }
                }
                read_json(&path).await
            }
            None => match read_json::<VersionJson>(&path).await {
                Ok(v) => Ok(v),
                Err(e) if e.code == ErrorCode::NotFound => Err(AppError::new(
                    ErrorCode::NotFound,
                    format!("unknown Minecraft version {id}"),
                )
                .detail("version", id)),
                Err(e) => Err(e),
            },
        }
    }

    /// Loads a profile (vanilla or loader) and resolves its `inheritsFrom` chain.
    pub async fn load_profile(&self, id: &str, sink: &dyn ProgressSink) -> AppResult<VersionJson> {
        let mut chain = Vec::new();
        let mut seen = HashSet::new();
        let mut next = Some(id.to_owned());
        while let Some(cur) = next.take() {
            if !seen.insert(cur.clone()) || chain.len() >= MAX_INHERIT_DEPTH {
                return Err(AppError::new(
                    ErrorCode::InvalidInput,
                    "inheritsFrom cycle or chain too deep",
                )
                .detail("version", cur));
            }
            let path = self.json_path(&cur)?;
            let json = match read_json::<VersionJson>(&path).await {
                Ok(v) if v.inherits_from.is_some() => v,
                // Vanilla jsons are always re-verified against the manifest.
                _ => self.ensure_vanilla_json(&cur, sink).await?,
            };
            next = json.inherits_from.clone();
            chain.push(json);
        }
        let mut merged = chain
            .pop()
            .ok_or_else(|| AppError::internal("empty profile chain"))?;
        while let Some(child) = chain.pop() {
            merged = merge_inherited(child, merged);
        }
        Ok(merged)
    }

    pub async fn java_major(&self, mc: &str) -> AppResult<u32> {
        let v = self.load_profile(mc, &NullSink::default()).await?;
        Ok(v.java_version
            .map(|j| j.major_version)
            .unwrap_or(DEFAULT_JAVA_MAJOR))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn manifest() -> VersionManifest {
        serde_json::from_str(
            r#"{"latest":{"release":"1.21.1","snapshot":"24w99a"},"versions":[
              {"id":"24w99a","type":"snapshot","url":"u","sha1":"x","releaseTime":"2024-09-01T00:00:00+00:00"},
              {"id":"1.21.1","type":"release","url":"u","sha1":"x","releaseTime":"2024-08-08T12:24:45+00:00"},
              {"id":"1.12.2","type":"release","url":"u","sha1":"x","releaseTime":"2017-09-18T08:39:46+00:00"},
              {"id":"b1.7.3","type":"old_beta","url":"u","sha1":"x","releaseTime":"2011-07-08T00:00:00+00:00"},
              {"id":"weird","type":"unknown","url":"u","releaseTime":""}]}"#,
        )
        .unwrap()
    }

    #[test]
    fn filters_by_kind_and_search() {
        let m = manifest();
        let all = filter_entries(&m, &VersionFilter::default(), |_| None);
        assert_eq!(all.len(), 4);

        let releases = filter_entries(
            &m,
            &VersionFilter {
                kinds: vec![VersionKind::Release],
                search: None,
            },
            |id| (id == "1.21.1").then_some(21),
        );
        assert_eq!(
            releases.iter().map(|v| v.id.as_str()).collect::<Vec<_>>(),
            vec!["1.21.1", "1.12.2"]
        );
        assert_eq!(releases[0].java_major, Some(21));
        assert_eq!(releases[1].java_major, None);

        let search = filter_entries(
            &m,
            &VersionFilter {
                kinds: vec![],
                search: Some(" 1.12 ".into()),
            },
            |_| None,
        );
        assert_eq!(search.len(), 1);
        assert_eq!(search[0].id, "1.12.2");
    }
}
