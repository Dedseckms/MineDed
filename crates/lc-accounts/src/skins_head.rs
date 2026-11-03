//! `SkinService::head`: the 8x8 face + hat overlay of a skin as a 64x64 PNG data
//! URL (avatar fallback for players without a picture).
//!
//! The skin is downloaded through `lc_http::Downloader`, composited and cached
//! as a PNG in `<cache>/skins-heads/<sha256(url)>.png`; decoded data URLs are
//! kept in a small in-memory map in front of the file.

use std::collections::{HashMap, VecDeque};
use std::path::Path;
use std::sync::{Arc, Mutex};

use base64::Engine as _;
use lc_http::{Downloader, ResourceKind};
use lc_types::{AppError, AppResult, ErrorCode};
use tokio::sync::Mutex as AsyncMutex;
use url::Url;

use crate::fsutil::write_atomic;
use crate::skins::{hex_sha256, validate_skin_png};

/// Face rectangle on the skin; `(x, y)` of an 8x8 block.
const FACE: (u32, u32) = (8, 8);
/// Hat overlay rectangle.
const HAT: (u32, u32) = (40, 8);
const BLOCK: u32 = 8;
/// Output edge in pixels (nearest-neighbour upscale of the 8x8 head).
pub const HEAD_SIZE: u32 = 64;
/// Upper bound for the downloaded body; real skins are a few KB.
const MAX_PNG: u64 = 1024 * 1024;
const MAX_MEMORY: usize = 128;

const DATA_URL_PREFIX: &str = "data:image/png;base64,";

/// Bounded insertion-ordered map (oldest entry evicted first).
struct SmallCache {
    map: HashMap<String, String>,
    order: VecDeque<String>,
}

impl SmallCache {
    fn new() -> Self {
        Self {
            map: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    fn get(&self, key: &str) -> Option<String> {
        self.map.get(key).cloned()
    }

    fn insert(&mut self, key: String, value: String) {
        if self.map.insert(key.clone(), value).is_none() {
            self.order.push_back(key);
        }
        while self.order.len() > MAX_MEMORY {
            if let Some(old) = self.order.pop_front() {
                self.map.remove(&old);
            }
        }
    }
}

/// Guards concurrent head requests and holds the in-memory cache.
pub(crate) struct HeadCache {
    lock: AsyncMutex<()>,
    memory: Mutex<SmallCache>,
}

impl Default for HeadCache {
    fn default() -> Self {
        Self {
            lock: AsyncMutex::new(()),
            memory: Mutex::new(SmallCache::new()),
        }
    }
}

impl HeadCache {
    fn memory_get(&self, url: &str) -> Option<String> {
        let guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(url)
    }

    fn memory_insert(&self, url: &str, value: &str) {
        let mut guard = self.memory.lock().unwrap_or_else(|e| e.into_inner());
        guard.insert(url.to_owned(), value.to_owned());
    }
}

fn parse_src(skin_url: &str) -> AppResult<Url> {
    let s = skin_url.trim();
    let url = Url::parse(s).map_err(|_| {
        AppError::new(ErrorCode::InvalidInput, "skin URL must be http(s)")
            .detail("field", "skin_url")
    })?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(
            AppError::new(ErrorCode::InvalidInput, "skin URL must be http(s)")
                .detail("field", "skin_url"),
        );
    }
    Ok(url)
}

fn decode_error() -> AppError {
    AppError::new(ErrorCode::InvalidInput, "cannot decode skin PNG").detail("field", "png")
}

/// RGBA pixels of the skin; `None` when the PNG cannot be decoded.
fn decode_rgba(data: &[u8]) -> Option<Vec<u8>> {
    let img = image::load_from_memory_with_format(data, image::ImageFormat::Png).ok()?;
    Some(img.to_rgba8().into_raw())
}

/// 8x8 head: face, then the hat overlay over it where the hat is not fully
/// transparent (Minecraft's `RenderPlayer` does the same).
fn head_pixels(skin: &[u8], height: u32) -> Vec<u8> {
    let mut head = vec![0u8; (BLOCK * BLOCK * 4) as usize];
    let px = |x: u32, y: u32| -> [u8; 4] {
        if y >= height {
            // Legacy 64x32 skins have no hat layer: treat as transparent.
            return [0; 4];
        }
        let i = ((y * 64 + x) * 4) as usize;
        let Some(s) = skin.get(i..i + 4) else {
            return [0; 4];
        };
        [s[0], s[1], s[2], s[3]]
    };
    for y in 0..BLOCK {
        for x in 0..BLOCK {
            let face = px(FACE.0 + x, FACE.1 + y);
            let hat = px(HAT.0 + x, HAT.1 + y);
            let out = if hat[3] == 0 { face } else { hat };
            let i = ((y * BLOCK + x) * 4) as usize;
            head[i..i + 4].copy_from_slice(&out);
        }
    }
    head
}

/// Nearest-neighbour upscale of the 8x8 head to `HEAD_SIZE` x `HEAD_SIZE`.
fn upscale(head: &[u8]) -> Vec<u8> {
    let scale = HEAD_SIZE / BLOCK;
    let edge = HEAD_SIZE as usize;
    let mut out = vec![0u8; edge * edge * 4];
    for y in 0..edge {
        for x in 0..edge {
            let si = (((y as u32 / scale) * BLOCK + x as u32 / scale) * 4) as usize;
            let di = (y * edge + x) * 4;
            out[di..di + 4].copy_from_slice(&head[si..si + 4]);
        }
    }
    out
}

fn encode_png(rgba: &[u8]) -> AppResult<Vec<u8>> {
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, HEAD_SIZE, HEAD_SIZE);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc
            .write_header()
            .map_err(|e| AppError::internal(format!("encode head PNG: {e}")))?;
        writer
            .write_image_data(rgba)
            .map_err(|e| AppError::internal(format!("encode head PNG: {e}")))?;
    }
    Ok(out)
}

fn data_url(png: &[u8]) -> String {
    format!(
        "{DATA_URL_PREFIX}{}",
        base64::engine::general_purpose::STANDARD.encode(png)
    )
}

/// Validates and renders `skin` into a 64x64 head PNG. Pure: no network, no disk.
pub(crate) fn render_head(skin: &[u8]) -> AppResult<Vec<u8>> {
    let (_, height) = validate_skin_png(skin)?;
    let rgba = decode_rgba(skin).ok_or_else(decode_error)?;
    let head = head_pixels(&rgba, height);
    encode_png(&upscale(&head))
}

fn cache_path(cache_dir: &Path, url: &str) -> std::path::PathBuf {
    cache_dir.join(format!("{}.png", hex_sha256(url.as_bytes())))
}

fn data_url_from_file(path: &Path) -> Option<String> {
    let png = std::fs::read(path).ok()?;
    if png.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(data_url(&png))
    } else {
        None
    }
}

fn cached(memory: &HeadCache, path: &Path, url: &str) -> Option<String> {
    if let Some(v) = memory.memory_get(url) {
        return Some(v);
    }
    let v = data_url_from_file(path)?;
    memory.memory_insert(url, &v);
    Some(v)
}

/// Head data URL for an http(s) skin URL, cached in `<cache>/skins-heads`.
pub(crate) async fn head(
    downloader: &Arc<dyn Downloader>,
    cache_dir: &Path,
    memory: &HeadCache,
    skin_url: &str,
) -> AppResult<String> {
    let url = parse_src(skin_url)?;
    let key = url.as_str();
    let path = cache_path(cache_dir, key);
    if let Some(v) = cached(memory, &path, key) {
        return Ok(v);
    }
    let _g = memory.lock.lock().await;
    // Another task may have filled the cache while we waited.
    if let Some(v) = cached(memory, &path, key) {
        return Ok(v);
    }
    let body = downloader
        .get_bytes(&url, ResourceKind::Skin)
        .await
        .map_err(|e| {
            let e = if e.code == ErrorCode::Network {
                e
            } else {
                AppError::new(ErrorCode::Network, format!("download skin: {}", e.message))
            };
            e.detail("url", key)
        })?;
    if body.len() as u64 > MAX_PNG {
        return Err(
            AppError::new(ErrorCode::InvalidInput, "skin file is too large")
                .detail("field", "png")
                .detail("url", key),
        );
    }
    let png = render_head(&body)?;
    // The head itself is fine; a cache write failure is not fatal.
    if let Err(e) = write_atomic(&path, &png, false) {
        tracing::warn!(error = %e.message, "cannot cache skin head");
    }
    let value = data_url(&png);
    memory.memory_insert(key, &value);
    Ok(value)
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicUsize, Ordering};

    use async_trait::async_trait;
    use lc_http::{DownloadReq, Route};

    use super::*;
    use crate::fsutil::testutil::TempDir;

    const URL: &str = "https://skins.example.org/s/abc.png";

    /// Serves fixed bytes to every `get_bytes` call and counts the calls.
    struct BytesDownloader {
        body: Vec<u8>,
        fail: bool,
        calls: AtomicUsize,
    }

    impl BytesDownloader {
        fn ok(body: Vec<u8>) -> Arc<Self> {
            Arc::new(Self {
                body,
                fail: false,
                calls: AtomicUsize::new(0),
            })
        }

        fn failing() -> Arc<Self> {
            Arc::new(Self {
                body: Vec::new(),
                fail: true,
                calls: AtomicUsize::new(0),
            })
        }

        fn count(&self) -> usize {
            self.calls.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl Downloader for BytesDownloader {
        async fn fetch_many(
            &self,
            _reqs: Vec<DownloadReq>,
            _sink: &dyn lc_runtime::ProgressSink,
        ) -> AppResult<()> {
            Err(AppError::not_implemented("fetch_many"))
        }

        async fn get_bytes(&self, _url: &Url, _kind: ResourceKind) -> AppResult<Vec<u8>> {
            self.calls.fetch_add(1, Ordering::SeqCst);
            if self.fail {
                return Err(AppError::new(ErrorCode::Network, "connection reset"));
            }
            Ok(self.body.clone())
        }

        fn client(&self, _route: Route) -> reqwest::Client {
            reqwest::Client::new()
        }

        fn mirrors_for(&self, original: &Url, _kind: ResourceKind) -> Vec<Url> {
            vec![original.clone()]
        }

        fn set_manual_proxy(&self, _proxy: Option<lc_types::settings::ProxyCfg>) {}
    }

    fn transparent() -> [u8; 4] {
        [0, 0, 0, 0]
    }

    /// Real PNG of `64 x height` built pixel by pixel.
    fn skin_png(height: u32, paint: impl Fn(u32, u32) -> [u8; 4]) -> Vec<u8> {
        let mut px = vec![0u8; (64 * height) as usize * 4];
        for y in 0..height {
            for x in 0..64u32 {
                let i = ((y * 64 + x) * 4) as usize;
                px[i..i + 4].copy_from_slice(&paint(x, y));
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 64, height);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&px).unwrap();
        }
        out
    }

    fn in_rect(x: u32, y: u32, ox: u32, oy: u32) -> bool {
        (ox..ox + BLOCK).contains(&x) && (oy..oy + BLOCK).contains(&y)
    }

    /// Decoded RGBA of a head data URL and its size.
    fn decode_data_url(url: &str) -> (u32, u32, Vec<u8>) {
        let b64 = url.strip_prefix(DATA_URL_PREFIX).expect("png data url");
        let png = base64::engine::general_purpose::STANDARD
            .decode(b64)
            .expect("base64");
        let img = image::load_from_memory_with_format(&png, image::ImageFormat::Png).expect("png");
        let (w, h) = (img.width(), img.height());
        (w, h, img.to_rgba8().into_raw())
    }

    fn head_via(body: Vec<u8>) -> AppResult<String> {
        let dir = TempDir::new();
        let dl = BytesDownloader::ok(body);
        let cache = dir.path().join("skins-heads");
        let memory = HeadCache::default();
        futures_block(head(&(dl as Arc<dyn Downloader>), &cache, &memory, URL))
    }

    /// Minimal blocking executor so pure-cache tests stay synchronous.
    fn futures_block<T>(fut: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(fut)
    }

    #[test]
    fn renders_face_from_64x64() {
        let png = skin_png(64, |x, y| {
            if in_rect(x, y, FACE.0, FACE.1) {
                [10, 200, 30, 255]
            } else {
                transparent()
            }
        });
        let url = head_via(png).unwrap();
        assert!(url.starts_with(DATA_URL_PREFIX));
        let (w, h, px) = decode_data_url(&url);
        assert_eq!((w, h), (HEAD_SIZE, HEAD_SIZE));
        for y in [0usize, 31, 63] {
            for x in [0usize, 31, 63] {
                let i = (y * 64 + x) * 4;
                assert_eq!(&px[i..i + 4], &[10, 200, 30, 255], "pixel {x},{y}");
            }
        }
    }

    #[test]
    fn garbage_is_invalid_input() {
        let err = head_via(b"GIF89a not a skin".to_vec()).unwrap_err();
        assert_eq!(err.code, ErrorCode::InvalidInput);
        assert_eq!(render_head(b"").unwrap_err().code, ErrorCode::InvalidInput);
        // A PNG header with the wrong size is rejected by the validator.
        let mut bad = b"\x89PNG\r\n\x1a\n".to_vec();
        bad.extend_from_slice(&13u32.to_be_bytes());
        bad.extend_from_slice(b"IHDR");
        bad.extend_from_slice(&32u32.to_be_bytes());
        bad.extend_from_slice(&32u32.to_be_bytes());
        bad.extend_from_slice(&[8, 6, 0, 0, 0, 0, 0, 0, 0]);
        assert_eq!(render_head(&bad).unwrap_err().code, ErrorCode::InvalidInput);
    }

    #[test]
    fn legacy_64x32_works() {
        let png = skin_png(32, |x, y| {
            if in_rect(x, y, FACE.0, FACE.1) {
                [1, 2, 3, 255]
            } else {
                transparent()
            }
        });
        let url = head_via(png).unwrap();
        let (w, h, px) = decode_data_url(&url);
        assert_eq!((w, h), (HEAD_SIZE, HEAD_SIZE));
        assert_eq!(&px[0..4], &[1, 2, 3, 255]);
    }

    #[test]
    fn transparent_hat_keeps_face() {
        let png = skin_png(64, |x, y| {
            if in_rect(x, y, FACE.0, FACE.1) {
                [200, 10, 10, 255]
            } else {
                transparent()
            }
        });
        let (_, _, px) = decode_data_url(&head_via(png).unwrap());
        assert_eq!(&px[0..4], &[200, 10, 10, 255]);
    }

    #[test]
    fn opaque_hat_covers_face() {
        let png = skin_png(64, |x, y| {
            if in_rect(x, y, HAT.0, HAT.1) {
                [0, 0, 255, 255]
            } else if in_rect(x, y, FACE.0, FACE.1) {
                [200, 10, 10, 255]
            } else {
                transparent()
            }
        });
        let (_, _, px) = decode_data_url(&head_via(png).unwrap());
        assert_eq!(&px[0..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn second_call_hits_the_cache() {
        let dir = TempDir::new();
        let cache = dir.path().join("skins-heads");
        let dl = BytesDownloader::ok(skin_png(64, |_, _| [9, 9, 9, 255]));
        let dyn_dl = dl.clone() as Arc<dyn Downloader>;
        let memory = HeadCache::default();
        let run = || head(&dyn_dl, &cache, &memory, URL);
        let first = futures_block(run()).unwrap();
        let second = futures_block(run()).unwrap();
        assert_eq!(first, second);
        assert_eq!(dl.count(), 1);
        // The rendered head, not the skin, is cached on disk.
        let path = cache_path(&cache, URL);
        let file = std::fs::read(&path).unwrap();
        assert_eq!((HEAD_SIZE, HEAD_SIZE), {
            let img = image::load_from_memory_with_format(&file, image::ImageFormat::Png).unwrap();
            (img.width(), img.height())
        });
        // A fresh memory cache re-reads the file without a download.
        let memory = HeadCache::default();
        let third = futures_block(head(&dyn_dl, &cache, &memory, URL)).unwrap();
        assert_eq!(first, third);
        assert_eq!(dl.count(), 1);
    }

    #[test]
    fn network_and_input_errors() {
        let dir = TempDir::new();
        let cache = dir.path().join("skins-heads");
        let memory = HeadCache::default();
        let dl = BytesDownloader::failing() as Arc<dyn Downloader>;
        let err = futures_block(head(&dl, &cache, &memory, URL)).unwrap_err();
        assert_eq!(err.code, ErrorCode::Network);

        for bad in ["", "not a url", "ftp://x.org/a.png", "file:///etc/x.png"] {
            let err = futures_block(head(&dl, &cache, &memory, bad)).unwrap_err();
            assert_eq!(err.code, ErrorCode::InvalidInput, "{bad}");
        }
    }
}
