//! `SkinService::fetch_remote`: skin by PNG URL, skin page URL or nickname.
//!
//! Pages are resolved with known URL patterns (minecraftskins.com, namemc.com)
//! or by scanning the HTML for `og:image` / Ely.by storage links; nicknames go
//! to Mojang first, then Ely.by's skinsystem.

use std::path::Path;
use std::time::Duration;

use lc_types::skin::{RemoteSkin, SkinModel};
use lc_types::{AppError, AppResult, ErrorCode};
use reqwest::{Client, StatusCode};
use url::Url;

use crate::fsutil::write_atomic;
use crate::skins::{hex_sha256, validate_skin_png};
use crate::yggdrasil::parse_textures;

/// Upper bound for any downloaded body (PNG or HTML page).
pub(crate) const MAX_BODY: usize = 1024 * 1024;
const TIMEOUT: Duration = Duration::from_secs(20);
const MAX_SUGGESTED: usize = 64;

/// Public endpoints used for nickname lookups (overridable in tests).
#[derive(Debug, Clone)]
pub(crate) struct RemoteEndpoints {
    /// `GET {mojang_api}/users/profiles/minecraft/{nick}`.
    pub mojang_api: String,
    /// `GET {mojang_session}/session/minecraft/profile/{uuid}`.
    pub mojang_session: String,
    /// `GET {ely_skinsystem}/textures/{nick}`.
    pub ely_skinsystem: String,
}

impl Default for RemoteEndpoints {
    fn default() -> Self {
        Self {
            mojang_api: "https://api.mojang.com".into(),
            mojang_session: "https://sessionserver.mojang.com".into(),
            ely_skinsystem: "https://skinsystem.ely.by".into(),
        }
    }
}

/// What the user typed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Source {
    Nickname(String),
    Url(Url),
}

fn invalid_source(source: &str) -> AppError {
    AppError::new(
        ErrorCode::InvalidInput,
        "expected a skin URL or a player nickname",
    )
    .detail("field", "source")
    .detail("source", source.chars().take(200).collect::<String>())
}

fn not_found(source: &str) -> AppError {
    AppError::new(ErrorCode::NotFound, "skin not found")
        .detail("source", source.chars().take(200).collect::<String>())
}

/// Minecraft-style nickname: 1-16 of `[A-Za-z0-9_]`.
pub(crate) fn is_nickname(s: &str) -> bool {
    (1..=16).contains(&s.len()) && s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
}

/// Nickname vs URL. A dotted string without a scheme is taken as an https URL.
pub(crate) fn classify(input: &str) -> AppResult<Source> {
    let s = input.trim();
    if is_nickname(s) {
        return Ok(Source::Nickname(s.to_owned()));
    }
    let with_scheme = if s.contains("://") {
        s.to_owned()
    } else if s.contains('.') && !s.contains(char::is_whitespace) {
        format!("https://{s}")
    } else {
        return Err(invalid_source(s));
    };
    let url = Url::parse(&with_scheme).map_err(|_| invalid_source(s))?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return Err(invalid_source(s));
    }
    Ok(Source::Url(url))
}

fn host_is(url: &Url, domain: &str) -> bool {
    url.host_str().is_some_and(|h| {
        let h = h.to_ascii_lowercase();
        h == domain || h.ends_with(&format!(".{domain}"))
    })
}

fn segments(url: &Url) -> Vec<&str> {
    url.path_segments()
        .map(|s| s.filter(|p| !p.is_empty()).collect())
        .unwrap_or_default()
}

/// Result of matching a URL against known sites without fetching it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum Known {
    /// Direct PNG link plus a suggested name.
    Png(Url, String),
    /// The URL designates a player (e.g. a NameMC profile).
    Nickname(String),
    /// Unknown: fetch it (PNG or HTML page).
    Fetch,
}

/// Known URL patterns:
/// - `minecraftskins.com/skin/<id>/<slug>/` в†’ `/skin/download/<id>`;
/// - `namemc.com/skin/<hash>` в†’ `s.namemc.com/i/<hash>.png`;
/// - `namemc.com/profile/<nick>[.N]` в†’ nickname.
pub(crate) fn match_known(url: &Url) -> Known {
    let segs = segments(url);
    if host_is(url, "minecraftskins.com") {
        if let ["skin", id, rest @ ..] = segs.as_slice() {
            if *id != "download" && id.bytes().all(|b| b.is_ascii_digit()) {
                let name = rest.first().copied().unwrap_or("skin");
                if let Ok(u) = Url::parse(&format!(
                    "https://www.minecraftskins.com/skin/download/{id}"
                )) {
                    return Known::Png(u, name.to_owned());
                }
            }
        }
    }
    if host_is(url, "namemc.com") && url.host_str() != Some("s.namemc.com") {
        match segs.as_slice() {
            ["skin", hash, ..] if hash.bytes().all(|b| b.is_ascii_hexdigit()) => {
                if let Ok(u) = Url::parse(&format!("https://s.namemc.com/i/{hash}.png")) {
                    return Known::Png(u, "skin".to_owned());
                }
            }
            ["profile", who, ..] => {
                // `Notch.1` в†’ `Notch`; a UUID-style id is not a nickname.
                let nick = who.split('.').next().unwrap_or_default();
                if is_nickname(nick) {
                    return Known::Nickname(nick.to_owned());
                }
            }
            _ => {}
        }
    }
    Known::Fetch
}

/// Value of `attr="..."` (or single quotes) inside one HTML tag.
fn attr<'a>(tag: &'a str, name: &str) -> Option<&'a str> {
    let lower = tag.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find(name) {
        let start = from + pos;
        from = start + name.len();
        let before_ok = start == 0 || lower.as_bytes()[start - 1].is_ascii_whitespace();
        let rest = lower[from..].trim_start();
        if !before_ok || !rest.starts_with('=') {
            continue;
        }
        let after_eq = from + (lower[from..].len() - rest.len()) + 1;
        let tail = &tag[after_eq..];
        let trimmed = tail.trim_start();
        let q = trimmed.chars().next()?;
        if q != '"' && q != '\'' {
            return None;
        }
        let body = &trimmed[1..];
        return body.find(q).map(|end| &body[..end]);
    }
    None
}

fn unescape(s: &str) -> String {
    s.replace("&amp;", "&")
        .replace("&#x2F;", "/")
        .replace("&#47;", "/")
}

/// Candidate PNG URLs from an HTML page: Ely.by storage links first, then
/// `og:image` / `twitter:image`.
pub(crate) fn page_candidates(html: &str, base: &Url) -> Vec<Url> {
    let mut out: Vec<Url> = Vec::new();
    let mut push = |raw: &str| {
        if let Ok(u) = base.join(&unescape(raw.trim())) {
            if matches!(u.scheme(), "http" | "https") && !out.contains(&u) {
                out.push(u);
            }
        }
    };
    // Ely.by pages link the raw skin as `.../storage/skins/<hash>.png`.
    let lower = html.to_ascii_lowercase();
    let mut from = 0;
    while let Some(pos) = lower[from..].find("/storage/skins/") {
        let mid = from + pos;
        from = mid + 1;
        let start = lower[..mid]
            .rfind(|c: char| c == '"' || c == '\'' || c.is_whitespace() || c == '(')
            .map_or(0, |i| i + 1);
        let Some(end_rel) = lower[mid..].find(".png") else {
            break;
        };
        let end = mid + end_rel + 4;
        let cand = &html[start..end];
        if !cand.contains(['"', '\'', '<', '>']) && !cand.contains(char::is_whitespace) {
            push(cand);
        }
    }
    let mut from = 0;
    while let Some(pos) = lower[from..].find("<meta") {
        let start = from + pos;
        let end = lower[start..].find('>').map_or(lower.len(), |e| start + e);
        from = end.max(start + 1);
        let tag = &html[start..end];
        let key = attr(tag, "property")
            .or_else(|| attr(tag, "name"))
            .map(str::to_ascii_lowercase);
        if matches!(
            key.as_deref(),
            Some("og:image" | "og:image:url" | "og:image:secure_url" | "twitter:image")
        ) {
            if let Some(c) = attr(tag, "content") {
                push(c);
            }
        }
    }
    out
}

/// `true` when a 64x64 skin with alpha has the 4th arm column empty (slim arms
/// are 3 px wide). 64x32 legacy skins are always classic.
pub(crate) fn guess_model(png: &[u8]) -> SkinModel {
    match rgba(png) {
        Some((64, 64, px)) => {
            let alpha = |x: usize, y: usize| px[(y * 64 + x) * 4 + 3];
            // Right arm: back face tail x 54..56, y 20..32; slim leaves it empty.
            let empty = (20..32).all(|y| (54..56).all(|x| alpha(x, y) == 0))
                && (16..20).all(|y| (50..52).all(|x| alpha(x, y) == 0));
            if empty {
                SkinModel::Slim
            } else {
                SkinModel::Classic
            }
        }
        _ => SkinModel::Classic,
    }
}

/// Decodes to 8-bit RGBA. `None` on any decoding problem.
fn rgba(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    let mut dec = png::Decoder::new(std::io::Cursor::new(data));
    dec.set_transformations(png::Transformations::normalize_to_color8());
    let mut reader = dec.read_info().ok()?;
    let mut buf = vec![0; reader.output_buffer_size()?];
    let info = reader.next_frame(&mut buf).ok()?;
    let (w, h) = (info.width, info.height);
    let n = (w * h) as usize;
    let out = match info.color_type {
        png::ColorType::Rgba => buf.get(..n * 4)?.to_vec(),
        png::ColorType::GrayscaleAlpha => buf
            .get(..n * 2)?
            .as_chunks::<2>()
            .0
            .iter()
            .flat_map(|&[g, a]| [g, g, g, a])
            .collect(),
        // No alpha channel: everything is opaque.
        png::ColorType::Rgb => buf
            .get(..n * 3)?
            .as_chunks::<3>()
            .0
            .iter()
            .flat_map(|&[r, g, b]| [r, g, b, 255])
            .collect(),
        png::ColorType::Grayscale => buf.get(..n)?.iter().flat_map(|&g| [g, g, g, 255]).collect(),
        png::ColorType::Indexed => return None,
    };
    Some((w, h, out))
}

/// File-name-ish suggestion: last path segment without `.png`, sanitized.
pub(crate) fn name_from_url(url: &Url) -> String {
    let last = segments(url).last().copied().unwrap_or_default();
    let stem = last
        .strip_suffix(".png")
        .or_else(|| last.strip_suffix(".PNG"))
        .unwrap_or(last);
    sanitize_name(stem)
}

pub(crate) fn sanitize_name(s: &str) -> String {
    let decoded = s.replace("%20", " ");
    let clean: String = decoded
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '-' | '_' | '.') {
                c
            } else {
                '_'
            }
        })
        .take(MAX_SUGGESTED)
        .collect();
    let clean = clean.trim().to_owned();
    if clean.is_empty() {
        "skin".to_owned()
    } else {
        clean
    }
}

/// GET with a body cap. `Ok(None)` for 404 / 204.
async fn get_capped(client: &Client, url: &Url) -> AppResult<Option<(Vec<u8>, String)>> {
    let net = |m: String| AppError::new(ErrorCode::Network, m).detail("url", url.as_str());
    let mut resp = client
        .get(url.clone())
        .timeout(TIMEOUT)
        .send()
        .await
        .map_err(|e| net(format!("request failed: {e}")))?;
    let status = resp.status();
    if status == StatusCode::NOT_FOUND || status == StatusCode::NO_CONTENT {
        return Ok(None);
    }
    if !status.is_success() {
        return Err(net(format!("HTTP {status}")).detail("status", status.as_u16().to_string()));
    }
    if resp.content_length().is_some_and(|l| l > MAX_BODY as u64) {
        return Err(too_large(url));
    }
    let ctype = resp
        .headers()
        .get(reqwest::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or_default()
        .to_ascii_lowercase();
    let mut body = Vec::new();
    while let Some(chunk) = resp
        .chunk()
        .await
        .map_err(|e| net(format!("read body: {e}")))?
    {
        if body.len() + chunk.len() > MAX_BODY {
            return Err(too_large(url));
        }
        body.extend_from_slice(&chunk);
    }
    Ok(Some((body, ctype)))
}

fn too_large(url: &Url) -> AppError {
    AppError::new(ErrorCode::InvalidInput, "skin file is too large")
        .detail("field", "png")
        .detail("url", url.as_str())
}

fn is_png(data: &[u8]) -> bool {
    data.starts_with(b"\x89PNG\r\n\x1a\n")
}

/// Skin PNG plus whatever the source said about the model.
struct Found {
    png: Vec<u8>,
    model: Option<SkinModel>,
    name: String,
}

async fn fetch_png(client: &Client, url: &Url, name: String) -> AppResult<Option<Found>> {
    let Some((data, _)) = get_capped(client, url).await? else {
        return Ok(None);
    };
    if !is_png(&data) {
        return Err(AppError::new(ErrorCode::InvalidInput, "not a PNG file")
            .detail("field", "png")
            .detail("url", url.as_str()));
    }
    Ok(Some(Found {
        png: data,
        model: None,
        name,
    }))
}

async fn from_url(client: &Client, ep: &RemoteEndpoints, url: &Url) -> AppResult<Option<Found>> {
    match match_known(url) {
        Known::Png(png, name) => return fetch_png(client, &png, sanitize_name(&name)).await,
        Known::Nickname(n) => return from_nickname(client, ep, &n).await,
        Known::Fetch => {}
    }
    let Some((data, ctype)) = get_capped(client, url).await? else {
        return Ok(None);
    };
    if is_png(&data) {
        return Ok(Some(Found {
            png: data,
            model: None,
            name: name_from_url(url),
        }));
    }
    if !ctype.is_empty() && !ctype.contains("html") {
        return Err(AppError::new(ErrorCode::InvalidInput, "not a PNG file")
            .detail("field", "png")
            .detail("url", url.as_str()));
    }
    let html = String::from_utf8_lossy(&data);
    let mut last_err = None;
    for cand in page_candidates(&html, url).into_iter().take(4) {
        match fetch_png(client, &cand, name_from_url(url)).await {
            Ok(Some(f)) if validate_skin_png(&f.png).is_ok() => return Ok(Some(f)),
            Ok(Some(f)) => last_err = validate_skin_png(&f.png).err(),
            Ok(None) => {}
            Err(e) => last_err = Some(e),
        }
    }
    match last_err {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

async fn from_nickname(
    client: &Client,
    ep: &RemoteEndpoints,
    nick: &str,
) -> AppResult<Option<Found>> {
    match mojang(client, ep, nick).await {
        Ok(Some(f)) => return Ok(Some(f)),
        Ok(None) => {}
        // Mojang may be blocked or rate-limited: still try Ely.by.
        Err(e) => tracing::warn!(error = %e.message, "mojang skin lookup failed"),
    }
    ely(client, ep, nick).await
}

fn parse_url(raw: &str) -> AppResult<Url> {
    Url::parse(raw).map_err(|_| {
        AppError::new(ErrorCode::Network, "unexpected server response: bad URL").detail("url", raw)
    })
}

fn bad_json(url: &Url) -> AppError {
    AppError::new(ErrorCode::Network, "unexpected server response").detail("url", url.as_str())
}

async fn mojang(client: &Client, ep: &RemoteEndpoints, nick: &str) -> AppResult<Option<Found>> {
    let base = ep.mojang_api.trim_end_matches('/');
    let url = parse_url(&format!("{base}/users/profiles/minecraft/{nick}"))?;
    let Some((body, _)) = get_capped(client, &url).await? else {
        return Ok(None);
    };
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| bad_json(&url))?;
    let (Some(id), name) = (
        v.get("id").and_then(|x| x.as_str()),
        v.get("name").and_then(|x| x.as_str()).unwrap_or(nick),
    ) else {
        return Ok(None);
    };
    if !id.bytes().all(|b| b.is_ascii_hexdigit() || b == b'-') {
        return Err(bad_json(&url));
    }
    let session = ep.mojang_session.trim_end_matches('/');
    let url = parse_url(&format!(
        "{session}/session/minecraft/profile/{}",
        id.replace('-', "")
    ))?;
    let Some((body, _)) = get_capped(client, &url).await? else {
        return Ok(None);
    };
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| bad_json(&url))?;
    let tex = parse_textures(&v)?;
    let Some(skin) = tex.skin_url else {
        return Ok(None);
    };
    let model = Some(if tex.slim {
        SkinModel::Slim
    } else {
        SkinModel::Classic
    });
    Ok(fetch_png(client, &parse_url(&skin)?, sanitize_name(name))
        .await?
        .map(|f| Found { model, ..f }))
}

async fn ely(client: &Client, ep: &RemoteEndpoints, nick: &str) -> AppResult<Option<Found>> {
    let base = ep.ely_skinsystem.trim_end_matches('/');
    let url = parse_url(&format!("{base}/textures/{nick}"))?;
    let Some((body, _)) = get_capped(client, &url).await? else {
        return Ok(None);
    };
    if body.iter().all(u8::is_ascii_whitespace) {
        return Ok(None);
    }
    let v: serde_json::Value = serde_json::from_slice(&body).map_err(|_| bad_json(&url))?;
    let skin = v.get("SKIN");
    let Some(raw) = skin.and_then(|s| s.get("url")).and_then(|u| u.as_str()) else {
        return Ok(None);
    };
    let slim = skin
        .and_then(|s| s.get("metadata"))
        .and_then(|m| m.get("model"))
        .and_then(|m| m.as_str())
        == Some("slim");
    let model = Some(if slim {
        SkinModel::Slim
    } else {
        SkinModel::Classic
    });
    Ok(fetch_png(client, &parse_url(raw)?, sanitize_name(nick))
        .await?
        .map(|f| Found { model, ..f }))
}

/// Resolves `source`, validates the PNG and caches it as `<cache>/<sha256>.png`.
pub(crate) async fn fetch_remote(
    client: &Client,
    ep: &RemoteEndpoints,
    cache: &Path,
    source: &str,
) -> AppResult<RemoteSkin> {
    let found = match classify(source)? {
        Source::Nickname(n) => from_nickname(client, ep, &n).await?,
        Source::Url(u) => from_url(client, ep, &u).await?,
    };
    let Some(found) = found else {
        return Err(not_found(source.trim()));
    };
    validate_skin_png(&found.png)?;
    let model = found.model.unwrap_or_else(|| guess_model(&found.png));
    let path = cache.join(format!("{}.png", hex_sha256(&found.png)));
    if !path.exists() {
        write_atomic(&path, &found.png, false)?;
    }
    Ok(RemoteSkin {
        local_path: path.display().to_string(),
        model,
        suggested_name: found.name,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    /// Real 64x64 RGBA PNG; `slim` clears the 4th right-arm column.
    pub(crate) fn skin_png(slim: bool) -> Vec<u8> {
        let mut px = vec![255u8; 64 * 64 * 4];
        if slim {
            for y in 16..32 {
                for x in 50..56 {
                    let in_top = y < 20 && (50..52).contains(&x);
                    let in_side = y >= 20 && (54..56).contains(&x);
                    if in_top || in_side {
                        px[(y * 64 + x) * 4 + 3] = 0;
                    }
                }
            }
        }
        let mut out = Vec::new();
        {
            let mut enc = png::Encoder::new(&mut out, 64, 64);
            enc.set_color(png::ColorType::Rgba);
            enc.set_depth(png::BitDepth::Eight);
            let mut w = enc.write_header().unwrap();
            w.write_image_data(&px).unwrap();
        }
        out
    }

    #[test]
    fn nickname_vs_url() {
        assert_eq!(
            classify(" Notch ").unwrap(),
            Source::Nickname("Notch".into())
        );
        assert_eq!(classify("a_b_9").unwrap(), Source::Nickname("a_b_9".into()));
        assert!(matches!(
            classify("ely.by/skins/123").unwrap(),
            Source::Url(u) if u.as_str() == "https://ely.by/skins/123"
        ));
        assert!(matches!(
            classify("http://x.org/a.png").unwrap(),
            Source::Url(u) if u.scheme() == "http"
        ));
        for bad in [
            "",
            "way_too_long_nickname_here",
            "two words",
            "ftp://x.org/a",
            "ab$",
        ] {
            assert_eq!(
                classify(bad).unwrap_err().code,
                ErrorCode::InvalidInput,
                "{bad}"
            );
        }
    }

    #[test]
    fn known_patterns() {
        assert_eq!(
            match_known(&u("https://www.minecraftskins.com/skin/21458012/cool-guy/")),
            Known::Png(
                u("https://www.minecraftskins.com/skin/download/21458012"),
                "cool-guy".into()
            )
        );
        assert_eq!(
            match_known(&u("https://namemc.com/skin/8a0f9c1b2d3e4f50")),
            Known::Png(
                u("https://s.namemc.com/i/8a0f9c1b2d3e4f50.png"),
                "skin".into()
            )
        );
        assert_eq!(
            match_known(&u("https://ru.namemc.com/profile/Notch.1")),
            Known::Nickname("Notch".into())
        );
        assert_eq!(match_known(&u("https://ely.by/skins/123")), Known::Fetch);
        assert_eq!(
            match_known(&u("https://s.namemc.com/i/abc.png")),
            Known::Fetch
        );
        assert_eq!(
            match_known(&u("https://evilminecraftskins.com/skin/1/x")),
            Known::Fetch
        );
    }

    #[test]
    fn page_resolution() {
        let html = r#"<html><head>
            <meta name="twitter:image" content="https://cdn.x/tw.png">
            <meta content='/img/og.png?a=1&amp;b=2' property="og:image" />
            </head><body><img src="https://ely.by/storage/skins/abc123.png"></body></html>"#;
        let got = page_candidates(html, &u("https://ely.by/skins/5"));
        assert_eq!(
            got,
            vec![
                u("https://ely.by/storage/skins/abc123.png"),
                u("https://cdn.x/tw.png"),
                u("https://ely.by/img/og.png?a=1&b=2"),
            ]
        );
        assert!(page_candidates("<p>nothing</p>", &u("https://a.b/")).is_empty());
    }

    #[test]
    fn model_guess_from_pixels() {
        assert_eq!(guess_model(&skin_png(false)), SkinModel::Classic);
        assert_eq!(guess_model(&skin_png(true)), SkinModel::Slim);
        assert_eq!(guess_model(b"garbage"), SkinModel::Classic);
    }

    #[test]
    fn names() {
        assert_eq!(
            name_from_url(&u("https://x.org/a/My%20Skin.png")),
            "My Skin"
        );
        assert_eq!(name_from_url(&u("https://x.org/")), "skin");
        assert_eq!(sanitize_name("a/b<c>"), "a_b_c_");
        assert_eq!(sanitize_name(&"x".repeat(100)).len(), 64);
    }
}
