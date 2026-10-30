//! Local cape imports and authenticated authlib-injector cape selection.

use std::io::Read;
use std::path::Path;
use std::time::Duration;

use url::Url;

use super::*;

const INDEX: &str = "capes.json";
const PREFIX: &str = "local-";

#[derive(Default, Serialize, Deserialize)]
struct CapeLibrary {
    #[serde(default)]
    capes: Vec<StoredCape>,
}

#[derive(Clone, Serialize, Deserialize)]
struct StoredCape {
    hash: String,
    name: String,
    #[serde(default)]
    uploads: Vec<CapeUpload>,
}

#[derive(Clone, Serialize, Deserialize)]
struct CapeUpload {
    account: AccountId,
    url: String,
}

fn invalid(message: &str) -> AppError {
    AppError::new(ErrorCode::InvalidInput, message).detail("field", "cape")
}

/// Reject links in every existing component, including Windows junctions.
fn reject_links(path: &Path) -> AppResult<()> {
    for ancestor in path.ancestors() {
        match std::fs::symlink_metadata(ancestor) {
            Ok(meta) => {
                let linked = meta.file_type().is_symlink();
                #[cfg(windows)]
                let linked = {
                    use std::os::windows::fs::MetadataExt;
                    linked || meta.file_attributes() & 0x400 != 0
                };
                if linked {
                    return Err(invalid("cape paths must not contain symbolic links"));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_err("stat cape path", ancestor, e)),
        }
    }
    Ok(())
}

fn read_capped(path: &Path) -> AppResult<Vec<u8>> {
    reject_links(path)?;
    let file = std::fs::File::open(path).map_err(|e| io_err("open cape", path, e))?;
    let metadata = file.metadata().map_err(|e| io_err("stat cape", path, e))?;
    if !metadata.is_file() || metadata.len() > MAX_PNG {
        return Err(invalid("cape must be a regular PNG file of at most 1 MiB"));
    }
    let mut bytes = Vec::new();
    file.take(MAX_PNG + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| io_err("read cape", path, e))?;
    if bytes.len() as u64 > MAX_PNG {
        return Err(invalid("cape file is too large"));
    }
    Ok(bytes)
}

fn validate_png(bytes: &[u8]) -> AppResult<()> {
    if bytes.len() as u64 > MAX_PNG || validate_skin_png(bytes)? != (64, 32) {
        return Err(invalid("cape must be a 64x32 PNG of at most 1 MiB"));
    }
    let mut decoder = png::Decoder::new(std::io::Cursor::new(bytes));
    decoder.set_limits(png::Limits { bytes: 256 * 1024 });
    let mut reader = decoder
        .read_info()
        .map_err(|_| invalid("invalid cape PNG"))?;
    if reader.info().animation_control.is_some() {
        return Err(invalid("animated cape PNGs are not supported"));
    }
    let size = reader
        .output_buffer_size()
        .filter(|size| *size <= 64 * 32 * 8)
        .ok_or_else(|| invalid("cape PNG exceeds decoded size limit"))?;
    let mut pixels = vec![0; size];
    reader
        .next_frame(&mut pixels)
        .map_err(|_| invalid("invalid or truncated cape PNG"))?;
    reader
        .finish()
        .map_err(|_| invalid("invalid or truncated cape PNG"))?;
    Ok(())
}

enum CapeSource {
    File(PathBuf),
    Https(Url),
}

fn source(input: &str) -> AppResult<CapeSource> {
    let input = input.trim();
    if input.is_empty() || input.contains('\0') {
        return Err(invalid("expected a cape file path or HTTPS URL"));
    }
    if input.contains("://") {
        let url = Url::parse(input).map_err(|_| invalid("invalid cape URL"))?;
        if url.scheme() != "https"
            || url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
        {
            return Err(invalid("cape URL must use HTTPS without credentials"));
        }
        Ok(CapeSource::Https(url))
    } else {
        let path = PathBuf::from(input);
        // Absolute paths make both preview scope and link checks unambiguous.
        if !path.is_absolute() {
            return Err(invalid("cape file path must be absolute"));
        }
        Ok(CapeSource::File(path))
    }
}

async fn download(client: &reqwest::Client, url: Url) -> AppResult<Vec<u8>> {
    let response = client
        .get(url)
        .timeout(Duration::from_secs(20))
        .send()
        .await
        .map_err(|_| AppError::new(ErrorCode::Network, "cape download failed"))?;
    if !response.status().is_success() || response.url().scheme() != "https" {
        return Err(AppError::new(ErrorCode::Network, "cape download failed"));
    }
    read_response(response).await
}

async fn read_response(mut response: reqwest::Response) -> AppResult<Vec<u8>> {
    if response.content_length().is_some_and(|n| n > MAX_PNG) {
        return Err(invalid("cape file is too large"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|_| AppError::new(ErrorCode::Network, "cape download failed"))?
    {
        if bytes.len().saturating_add(chunk.len()) > MAX_PNG as usize {
            return Err(invalid("cape file is too large"));
        }
        bytes.extend_from_slice(&chunk);
    }
    Ok(bytes)
}

impl SkinsService {
    fn cape_dir(&self) -> PathBuf {
        self.dir.join("capes")
    }

    fn cape_path(&self, hash: &str) -> AppResult<PathBuf> {
        if !is_sha256_hex(hash) {
            return Err(invalid("invalid local cape ID"));
        }
        Ok(self.cape_dir().join(format!("{hash}.png")))
    }

    fn load_capes(&self) -> AppResult<CapeLibrary> {
        let path = self.cape_dir().join(INDEX);
        reject_links(&path)?;
        let Some(bytes) = read_optional(&path)? else {
            return Ok(CapeLibrary::default());
        };
        let mut index: CapeLibrary = serde_json::from_slice(&bytes)
            .map_err(|_| AppError::new(ErrorCode::Io, "invalid cape library"))?;
        index.capes.retain(|c| is_sha256_hex(&c.hash));
        let mut seen = std::collections::HashSet::new();
        index.capes.retain(|c| seen.insert(c.hash.clone()));
        Ok(index)
    }

    fn save_capes(&self, index: &CapeLibrary) -> AppResult<()> {
        let path = self.cape_dir().join(INDEX);
        reject_links(&path)?;
        reject_links(&path.with_file_name(format!("{INDEX}.tmp")))?;
        let bytes = serde_json::to_vec_pretty(index)
            .map_err(|_| AppError::internal("serialize cape library"))?;
        write_atomic(&path, &bytes, false)
    }

    fn cape_dto(&self, cape: &StoredCape, active: bool) -> AppResult<Cape> {
        let path = self.cape_path(&cape.hash)?;
        let bytes = read_capped(&path)?;
        validate_png(&bytes)?;
        if hex_sha256(&bytes) != cape.hash {
            return Err(invalid("local cape hash mismatch"));
        }
        Ok(Cape {
            id: format!("{PREFIX}{}", cape.hash),
            name: cape.name.clone(),
            url: path.display().to_string(),
            active,
        })
    }

    pub(super) async fn import_cape(&self, input: &str) -> AppResult<Cape> {
        let (bytes, name) = match source(input)? {
            CapeSource::File(path) => {
                let name = remote::sanitize_name(
                    path.file_stem().and_then(|s| s.to_str()).unwrap_or("Cape"),
                );
                (read_capped(&path)?, name)
            }
            CapeSource::Https(url) => {
                let name = remote::name_from_url(&url);
                (download(&self.accounts.http(), url).await?, name)
            }
        };
        validate_png(&bytes)?;
        let hash = hex_sha256(&bytes);
        let _guard = self.lock.lock().await;
        let mut index = self.load_capes()?;
        if let Some(cape) = index.capes.iter().find(|c| c.hash == hash) {
            return self.cape_dto(cape, false);
        }
        let path = self.cape_path(&hash)?;
        reject_links(&path)?;
        reject_links(&path.with_extension("png.tmp"))?;
        write_atomic(&path, &bytes, false)?;
        let cape = StoredCape {
            hash,
            name,
            uploads: Vec::new(),
        };
        index.capes.push(cape.clone());
        self.save_capes(&index)?;
        self.cape_dto(&cape, false)
    }

    pub(super) async fn list_capes(&self, account: AccountId) -> AppResult<Vec<Cape>> {
        let target = self.accounts.ygg_target(Some(account)).await?;
        let current = ygg::fetch_textures(&self.accounts.http(), &target.root, &target.uuid)
            .await?
            .cape_url;
        let _guard = self.lock.lock().await;
        let index = self.load_capes()?;
        let mut out = Vec::new();
        let mut matched = false;
        for cape in &index.capes {
            let active = current.as_ref().is_some_and(|url| {
                cape.uploads
                    .iter()
                    .any(|u| u.account == account && &u.url == url)
                    || url.rsplit('/').next() == Some(cape.hash.as_str())
            });
            let dto = self.cape_dto(cape, active)?;
            matched |= active;
            out.push(dto);
        }
        if let Some(url) = current.filter(|_| !matched) {
            out.push(Cape {
                id: url.rsplit('/').next().unwrap_or_default().to_owned(),
                name: "Cape".into(),
                url,
                active: true,
            });
        }
        Ok(out)
    }

    pub(super) async fn select_cape(
        &self,
        account: AccountId,
        id: Option<String>,
    ) -> AppResult<()> {
        let target = self.accounts.ygg_target(Some(account)).await?;
        let client = self.accounts.http();
        let Some(id) = id else {
            let _guard = self.lock.lock().await;
            return ygg::delete_texture(
                &client,
                &target.root,
                &target.session,
                &target.uuid,
                TextureKind::Cape,
            )
            .await;
        };
        let Some(hash) = id.strip_prefix(PREFIX) else {
            let current = ygg::fetch_textures(&client, &target.root, &target.uuid).await?;
            return if current
                .cape_url
                .as_ref()
                .is_some_and(|url| url.rsplit('/').next() == Some(id.as_str()))
            {
                Ok(())
            } else {
                Err(AppError::new(ErrorCode::NotFound, "cape not available"))
            };
        };
        // Hold the catalog lock through upload so concurrent selections cannot
        // associate a server URL with a different local cape.
        let _guard = self.lock.lock().await;
        let mut index = self.load_capes()?;
        if !index.capes.iter().any(|c| c.hash == hash) {
            return Err(AppError::new(ErrorCode::NotFound, "cape not available"));
        }
        let bytes = read_capped(&self.cape_path(hash)?)?;
        validate_png(&bytes)?;
        if hex_sha256(&bytes) != hash {
            return Err(invalid("local cape hash mismatch"));
        }
        ygg::upload_texture(
            &client,
            &target.root,
            &target.session,
            &target.uuid,
            TextureKind::Cape,
            bytes,
            SkinModel::Classic,
        )
        .await?;
        if let Ok(textures) = ygg::fetch_textures(&client, &target.root, &target.uuid).await {
            if let Some(url) = textures.cape_url {
                // A URL can be reused by servers; retain a single local match.
                for entry in &mut index.capes {
                    entry
                        .uploads
                        .retain(|u| u.account != account || u.url != url);
                    if entry.hash == hash {
                        entry.uploads.push(CapeUpload {
                            account,
                            url: url.clone(),
                        });
                    }
                }
                self.save_capes(&index)?;
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fsutil::testutil::TempDir;

    pub(crate) fn cape_png(value: u8) -> Vec<u8> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, 64, 32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().unwrap();
            writer.write_image_data(&vec![value; 64 * 32 * 4]).unwrap();
        }
        bytes
    }

    fn service(paths: &Paths) -> SkinsService {
        SkinsService::new(
            paths,
            Arc::new(AccountsService::new(
                paths,
                Arc::new(lc_http::StubDownloader),
                Arc::new(crate::MemoryTokenStore::new()),
            )),
        )
    }

    #[test]
    fn requires_fully_decoded_static_64x32_png() {
        let bytes = cape_png(42);
        validate_png(&bytes).unwrap();
        for end in [20, 33, bytes.len() / 2, bytes.len() - 1, bytes.len() - 12] {
            assert!(validate_png(&bytes[..end]).is_err(), "truncated at {end}");
        }
        assert!(validate_png(&crate::remote::tests::skin_png(false)).is_err());
        assert!(validate_png(&super::super::tests::png(64, 32)).is_err());
        assert!(validate_png(&super::super::tests::png(u32::MAX, u32::MAX)).is_err());
        assert!(validate_png(&vec![0; MAX_PNG as usize + 1]).is_err());
        let mut animated = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut animated, 64, 32);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            encoder.set_animated(1, 0).unwrap();
            encoder
                .write_header()
                .unwrap()
                .write_image_data(&vec![0; 64 * 32 * 4])
                .unwrap();
        }
        assert!(validate_png(&animated).is_err());
        let mut corrupt = bytes;
        corrupt[40] ^= 0xff;
        assert!(validate_png(&corrupt).is_err());
    }

    #[test]
    fn source_rules_and_file_bounds() {
        for bad in [
            "",
            "relative.png",
            "http://example.org/a.png",
            "file:///a.png",
            "ftp://example.org/a",
            "https://user:password@example.org/a.png",
        ] {
            assert!(source(bad).is_err(), "{bad}");
        }
        assert!(matches!(
            source("https://example.org/cape.png").unwrap(),
            CapeSource::Https(_)
        ));
        let dir = TempDir::new();
        assert!(matches!(
            source(dir.path().to_str().unwrap()).unwrap(),
            CapeSource::File(_)
        ));
        assert!(read_capped(dir.path()).is_err());
        let path = dir.path().join("large.png");
        std::fs::write(&path, vec![0; MAX_PNG as usize + 1]).unwrap();
        assert!(read_capped(&path).is_err());
    }

    #[tokio::test]
    async fn downloaded_body_is_bounded_with_and_without_content_length() {
        use lc_http::Downloader;
        use tokio::io::{AsyncReadExt, AsyncWriteExt};

        for chunked in [false, true] {
            let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
            let addr = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                let (mut socket, _) = listener.accept().await.unwrap();
                let mut request = [0; 4096];
                let _ = socket.read(&mut request).await.unwrap();
                if chunked {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n")
                        .await
                        .unwrap();
                    let block = vec![0; 16 * 1024];
                    for _ in 0..65 {
                        if socket.write_all(b"4000\r\n").await.is_err()
                            || socket.write_all(&block).await.is_err()
                            || socket.write_all(b"\r\n").await.is_err()
                        {
                            return;
                        }
                    }
                    let _ = socket.write_all(b"0\r\n\r\n").await;
                } else {
                    socket
                        .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 1048577\r\n\r\n")
                        .await
                        .unwrap();
                }
            });
            let downloader =
                lc_http::HttpDownloader::new(lc_http::DownloaderConfig::default()).unwrap();
            let response = downloader
                .client(lc_http::Route::Direct)
                .get(format!("http://{addr}/cape.png"))
                .send()
                .await
                .unwrap();
            assert_eq!(
                read_response(response).await.unwrap_err().code,
                ErrorCode::InvalidInput
            );
            server.await.unwrap();
        }
    }

    #[tokio::test]
    async fn imports_deduplicate_persist_and_detect_tampering() {
        let dir = TempDir::new();
        let paths = Paths::new(dir.path());
        let svc = service(&paths);
        let input = dir.path().join("Example.png");
        let bytes = cape_png(17);
        std::fs::write(&input, &bytes).unwrap();
        let cape = svc.import_cape(input.to_str().unwrap()).await.unwrap();
        assert!(!cape.active);
        assert_eq!(cape.name, "Example");
        assert_eq!(cape.id, format!("local-{}", hex_sha256(&bytes)));
        assert!(Path::new(&cape.url).starts_with(dir.path().join("skins/capes")));
        let other = dir.path().join("Other.png");
        std::fs::write(&other, &bytes).unwrap();
        assert_eq!(
            svc.import_cape(other.to_str().unwrap()).await.unwrap().id,
            cape.id
        );
        let reloaded = service(&paths);
        let index = reloaded.load_capes().unwrap();
        assert_eq!(index.capes.len(), 1);
        assert_eq!(
            reloaded.cape_dto(&index.capes[0], false).unwrap().url,
            cape.url
        );
        std::fs::write(&cape.url, cape_png(18)).unwrap();
        assert!(reloaded.cape_dto(&index.capes[0], false).is_err());
        assert!(svc.cape_path("../../escape").is_err());
        assert!(svc.cape_path(&"A".repeat(64)).is_err());
        let bad_index = serde_json::json!({"capes":[
            {"hash":"../../escape", "name":"bad"},
            {"hash":hex_sha256(&bytes), "name":"ok"},
            {"hash":hex_sha256(&bytes), "name":"duplicate"}
        ]});
        std::fs::write(svc.cape_dir().join(INDEX), bad_index.to_string()).unwrap();
        assert_eq!(svc.load_capes().unwrap().capes.len(), 1);
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn rejects_source_and_cache_symlinks() {
        use std::os::unix::fs::symlink;
        let dir = TempDir::new();
        let paths = Paths::new(dir.path());
        let svc = service(&paths);
        let input = dir.path().join("cape.png");
        std::fs::write(&input, cape_png(3)).unwrap();
        let link = dir.path().join("link.png");
        symlink(&input, &link).unwrap();
        assert!(svc.import_cape(link.to_str().unwrap()).await.is_err());
        std::fs::create_dir_all(&svc.dir).unwrap();
        symlink(dir.path(), svc.cape_dir()).unwrap();
        assert!(svc.import_cape(input.to_str().unwrap()).await.is_err());
    }
}
