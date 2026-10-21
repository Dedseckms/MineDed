//! authlib-injector jar: latest release from `authlib-injector.yushi.moe`
//! (BMCLAPI mirror as fallback), SHA-256 verified, cached in `Paths::cache()`.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use lc_types::{AppError, AppResult, ErrorCode};
use reqwest::Client;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::sync::Mutex;
use url::Url;

use crate::fsutil::{read_optional, write_atomic};

/// Official artifact host (base for `artifact/latest.json`).
pub const AUTHLIB_PRIMARY_BASE: &str = "https://authlib-injector.yushi.moe/";
/// BMCLAPI mirror of the same tree.
pub const AUTHLIB_MIRROR_BASE: &str = "https://bmclapi2.bangbang93.com/mirrors/authlib-injector/";

const STATE_FILE: &str = "state.json";
/// How long a verified cached jar is used without asking for a newer release.
const CHECK_INTERVAL: Duration = Duration::from_secs(24 * 3600);
const TIMEOUT: Duration = Duration::from_secs(60);
const MAX_JAR: usize = 16 * 1024 * 1024;

#[derive(Debug, Clone, Deserialize)]
struct Latest {
    version: String,
    download_url: String,
    checksums: Checksums,
}

#[derive(Debug, Clone, Deserialize)]
struct Checksums {
    sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheState {
    version: String,
    file: String,
    sha256: String,
    checked_at: u64,
}

/// Downloads and caches authlib-injector. Bases are tried in order.
#[derive(Debug)]
pub struct AuthlibManager {
    dir: PathBuf,
    bases: Vec<Url>,
    lock: Mutex<()>,
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn sha256_hex(data: &[u8]) -> String {
    Sha256::digest(data)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect()
}

fn safe_version(v: &str) -> String {
    v.chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
        .take(32)
        .collect()
}

impl AuthlibManager {
    /// `dir` = cache folder (e.g. `Paths::cache()/authlib-injector`).
    pub fn new(dir: PathBuf) -> Self {
        let bases = [AUTHLIB_PRIMARY_BASE, AUTHLIB_MIRROR_BASE]
            .iter()
            .filter_map(|b| Url::parse(b).ok())
            .collect();
        Self::with_bases(dir, bases)
    }

    /// Custom artifact bases (tests, self-hosted mirror). Each base must end with `/`.
    pub fn with_bases(dir: PathBuf, bases: Vec<Url>) -> Self {
        Self {
            dir,
            bases,
            lock: Mutex::new(()),
        }
    }

    fn load_state(&self) -> Option<CacheState> {
        let bytes = read_optional(&self.dir.join(STATE_FILE)).ok()??;
        serde_json::from_slice(&bytes).ok()
    }

    fn save_state(&self, st: &CacheState) -> AppResult<()> {
        let bytes = serde_json::to_vec_pretty(st)
            .map_err(|e| AppError::internal(format!("serialize authlib state: {e}")))?;
        write_atomic(&self.dir.join(STATE_FILE), &bytes, false)
    }

    /// Cached jar path if the file exists and matches the recorded hash.
    fn verified_cached(&self, st: &CacheState) -> Option<PathBuf> {
        let path = self.dir.join(Path::new(&st.file).file_name()?);
        let bytes = std::fs::read(&path).ok()?;
        (sha256_hex(&bytes) == st.sha256).then_some(path)
    }

    async fn get(client: &Client, url: &Url, limit: usize) -> AppResult<Vec<u8>> {
        let net = |e: String| AppError::new(ErrorCode::Network, e).detail("url", url.as_str());
        let resp = client
            .get(url.clone())
            .timeout(TIMEOUT)
            .send()
            .await
            .map_err(|e| net(format!("request failed: {e}")))?;
        if !resp.status().is_success() {
            return Err(net(format!("HTTP {}", resp.status())));
        }
        let bytes = resp
            .bytes()
            .await
            .map_err(|e| net(format!("read body: {e}")))?;
        if bytes.len() > limit {
            return Err(net("response too large".into()));
        }
        Ok(bytes.to_vec())
    }

    async fn fetch_latest(&self, client: &Client) -> AppResult<(Latest, usize)> {
        let mut last = None;
        for (i, base) in self.bases.iter().enumerate() {
            let Ok(url) = base.join("artifact/latest.json") else {
                continue;
            };
            match Self::get(client, &url, 64 * 1024).await {
                Ok(b) => match serde_json::from_slice::<Latest>(&b) {
                    Ok(l) => return Ok((l, i)),
                    Err(e) => {
                        last = Some(
                            AppError::new(ErrorCode::Network, format!("bad latest.json: {e}"))
                                .detail("url", url.as_str()),
                        )
                    }
                },
                Err(e) => last = Some(e),
            }
        }
        Err(last.unwrap_or_else(|| AppError::internal("no authlib-injector sources")))
    }

    /// Download candidates: the URL rewritten onto the base that answered,
    /// then onto every other base, then the original URL.
    fn candidates(&self, download_url: &str, first: usize) -> Vec<Url> {
        let rel = self
            .bases
            .iter()
            .map(Url::as_str)
            .chain(std::iter::once(AUTHLIB_PRIMARY_BASE))
            .chain(std::iter::once(AUTHLIB_MIRROR_BASE))
            .find_map(|b| download_url.strip_prefix(b));
        let mut order: Vec<usize> = vec![first];
        order.extend((0..self.bases.len()).filter(|&i| i != first));
        let mut out: Vec<Url> = Vec::new();
        if let Some(rel) = rel {
            for i in order {
                if let Some(u) = self.bases.get(i).and_then(|b| b.join(rel).ok()) {
                    if !out.contains(&u) {
                        out.push(u);
                    }
                }
            }
        }
        if let Ok(u) = Url::parse(download_url) {
            if !out.contains(&u) {
                out.push(u);
            }
        }
        out
    }

    /// Returns the path of a verified jar, downloading the latest release when
    /// needed. Falls back to the cached jar when every source is unreachable.
    pub async fn ensure(&self, client: &Client) -> AppResult<PathBuf> {
        let _g = self.lock.lock().await;
        let state = self.load_state();
        let cached = state
            .as_ref()
            .and_then(|s| self.verified_cached(s).map(|p| (s.clone(), p)));
        if let Some((st, path)) = &cached {
            if now().saturating_sub(st.checked_at) < CHECK_INTERVAL.as_secs() {
                return Ok(path.clone());
            }
        }

        let (latest, idx) = match self.fetch_latest(client).await {
            Ok(v) => v,
            Err(e) => {
                if let Some((_, path)) = cached {
                    tracing::warn!(error = %e.message, "authlib-injector check failed; using cache");
                    return Ok(path);
                }
                return Err(e);
            }
        };
        let expected = latest.checksums.sha256.trim().to_ascii_lowercase();

        if let Some((mut st, path)) = cached {
            if st.sha256 == expected {
                st.checked_at = now();
                self.save_state(&st)?;
                return Ok(path);
            }
        }

        let mut last = None;
        for url in self.candidates(&latest.download_url, idx) {
            let bytes = match Self::get(client, &url, MAX_JAR).await {
                Ok(b) => b,
                Err(e) => {
                    last = Some(e);
                    continue;
                }
            };
            let actual = sha256_hex(&bytes);
            if actual != expected {
                last = Some(
                    AppError::new(ErrorCode::HashMismatch, "authlib-injector hash mismatch")
                        .detail("url", url.as_str())
                        .detail("expected", expected.clone())
                        .detail("actual", actual),
                );
                continue;
            }
            let file = format!("authlib-injector-{}.jar", safe_version(&latest.version));
            let path = self.dir.join(&file);
            write_atomic(&path, &bytes, false)?;
            if let Some(old) = &state {
                if old.file != file {
                    let _ = std::fs::remove_file(self.dir.join(&old.file));
                }
            }
            self.save_state(&CacheState {
                version: latest.version.clone(),
                file,
                sha256: expected.clone(),
                checked_at: now(),
            })?;
            return Ok(path);
        }
        Err(last.unwrap_or_else(|| AppError::internal("no authlib-injector download URL")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn candidates_rewrite_to_mirror() {
        let m = AuthlibManager::new(PathBuf::from("x"));
        let c = m.candidates(
            "https://authlib-injector.yushi.moe/artifact/53/authlib-injector-1.2.5.jar",
            1,
        );
        assert_eq!(
            c.iter().map(Url::as_str).collect::<Vec<_>>(),
            vec![
                "https://bmclapi2.bangbang93.com/mirrors/authlib-injector/artifact/53/authlib-injector-1.2.5.jar",
                "https://authlib-injector.yushi.moe/artifact/53/authlib-injector-1.2.5.jar",
            ]
        );
    }

    #[test]
    fn version_is_sanitized() {
        assert_eq!(safe_version("1.2.5/../../x"), "1.2.5....x");
    }
}
