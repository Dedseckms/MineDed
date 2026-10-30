use super::{internal, invalid, Caller, Chat};
use crate::{error::ApiError, AppState};
use axum::body::Body;
use axum::extract::{Extension, Multipart, Path, Query, State};
use axum::http::{header, HeaderValue, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use lc_types::social::SocialAttachment;
use rusqlite::{params, OptionalExtension};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::io::{Cursor, Write};
use std::sync::Arc;

pub(super) const MAX_UPLOAD: usize = 10 * 1024 * 1024;
fn oversized() -> ApiError {
    ApiError::new(
        StatusCode::PAYLOAD_TOO_LARGE,
        "invalidInput",
        "file exceeds 10 MiB",
    )
}
fn hash(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
pub(super) fn persist(dir: &std::path::Path, bytes: &[u8]) -> Result<String, ApiError> {
    let digest = hash(bytes);
    let path = dir.join(&digest);
    if path.exists() {
        if hash(&std::fs::read(&path)?) != digest {
            return Err(internal());
        }
        return Ok(digest);
    }
    let temp = dir.join(format!(".upload-{}", uuid::Uuid::new_v4()));
    let mut file = std::fs::OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&temp)?;
    let result = (|| {
        file.write_all(bytes)?;
        file.sync_all()?;
        std::fs::rename(&temp, &path)?;
        #[cfg(unix)]
        std::fs::File::open(dir)?.sync_all()?;
        Ok::<_, ApiError>(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temp);
    }
    result?;
    Ok(digest)
}
struct Encoded {
    bytes: Vec<u8>,
    thumb: Option<Vec<u8>>,
    width: Option<u32>,
    height: Option<u32>,
}
fn encode(bytes: Vec<u8>) -> Result<Encoded, ApiError> {
    if image::guess_format(&bytes).is_err() {
        return Ok(Encoded {
            bytes,
            thumb: None,
            width: None,
            height: None,
        });
    }
    let mut reader = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| invalid("invalid image"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let (width, height) = image::ImageReader::new(Cursor::new(&bytes))
        .with_guessed_format()
        .map_err(|_| invalid("invalid image"))?
        .into_dimensions()
        .map_err(|_| invalid("invalid image"))?;
    if u64::from(width) * u64::from(height) > 8_000_000 || width > 4096 || height > 4096 {
        return Err(invalid("image dimensions exceed limit"));
    }
    let image = reader
        .decode()
        .map_err(|_| invalid("invalid or oversized image"))?;
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::WebP)
        .map_err(|_| invalid("image conversion failed"))?;
    if encoded.get_ref().len() > MAX_UPLOAD {
        return Err(oversized());
    }
    let mut side = 320;
    let thumb = loop {
        let mut thumb = Cursor::new(Vec::new());
        image
            .thumbnail(side, side)
            .write_to(&mut thumb, image::ImageFormat::WebP)
            .map_err(|_| invalid("thumbnail conversion failed"))?;
        if thumb.get_ref().len() <= 256 * 1024 {
            break thumb;
        }
        side = side * 3 / 4;
        if side < 32 {
            return Err(invalid("thumbnail is too large"));
        }
    };
    Ok(Encoded {
        bytes: encoded.into_inner(),
        thumb: Some(thumb.into_inner()),
        width: Some(width),
        height: Some(height),
    })
}
/// Re-encodes an avatar to a 256×256 WebP (cropped to a square, bounded decode).
pub(super) fn avatar(bytes: &[u8]) -> Result<Vec<u8>, ApiError> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|_| invalid("invalid image"))?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(4096);
    limits.max_image_height = Some(4096);
    limits.max_alloc = Some(64 * 1024 * 1024);
    reader.limits(limits);
    let image = reader
        .decode()
        .map_err(|_| invalid("invalid or oversized image"))?;
    let image = image.resize_to_fill(256, 256, image::imageops::FilterType::Lanczos3);
    let mut encoded = Cursor::new(Vec::new());
    image
        .write_to(&mut encoded, image::ImageFormat::WebP)
        .map_err(|_| invalid("image conversion failed"))?;
    if encoded.get_ref().len() > super::MAX_AVATAR {
        return Err(super::avatar_too_large());
    }
    Ok(encoded.into_inner())
}

pub(super) async fn upload(
    State(s): State<AppState>,
    Extension(Caller(me)): Extension<Caller>,
    multipart: Result<Multipart, axum::extract::multipart::MultipartRejection>,
) -> Result<Json<SocialAttachment>, ApiError> {
    let chat = s.chat.ok_or_else(internal)?;
    chat.uploads.check(&me).map_err(ApiError::rate_limited)?;
    let _permit = chat
        .upload_jobs
        .clone()
        .try_acquire_owned()
        .map_err(|_| ApiError::rate_limited(5))?;
    chat.run(|db, chat| {
        if chat.migration.lock().map_err(|_| internal())?.is_some() {
            return Err(ApiError::rate_limited(5));
        }
        if db.health(chat)?.attachments_paused {
            return Err(ApiError::new(
                StatusCode::INSUFFICIENT_STORAGE,
                "io",
                "attachments temporarily paused",
            ));
        }
        Ok(())
    })
    .await?;
    let mut multipart = multipart.map_err(|_| invalid("multipart form required"))?;
    let field = multipart
        .next_field()
        .await
        .map_err(|_| oversized())?
        .ok_or_else(|| invalid("file required"))?;
    if field.name() != Some("file") {
        return Err(invalid("file field required"));
    }
    let name = field
        .file_name()
        .unwrap_or("file")
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | '"' | ';'))
        .take(150)
        .collect::<String>();
    let bytes = field.bytes().await.map_err(|_| oversized())?;
    if bytes.len() > MAX_UPLOAD {
        return Err(oversized());
    }
    if bytes.is_empty() {
        return Err(invalid("file is empty"));
    }
    if multipart
        .next_field()
        .await
        .map_err(|_| oversized())?
        .is_some()
    {
        return Err(invalid("only one file is allowed"));
    }
    chat.run(move |db, chat| {
        if chat.migration.lock().map_err(|_| internal())?.is_some() {
            return Err(ApiError::rate_limited(5));
        }
        // Recheck under the mutation lock; queued uploads cannot bypass a full disk.
        if db.health(chat)?.attachments_paused {
            return Err(ApiError::new(
                StatusCode::INSUFFICIENT_STORAGE,
                "io",
                "attachments temporarily paused",
            ));
        }
        let encoded = encode(bytes.to_vec())?;
        let digest = persist(&db.files, &encoded.bytes)?;
        let thumb = encoded
            .thumb
            .as_ref()
            .map(|b| persist(&db.files, b))
            .transpose()?;
        let id = uuid::Uuid::new_v4().simple().to_string();
        let attachment = SocialAttachment {
            id: id.clone(),
            file_name: if encoded.thumb.is_some() {
                format!("{}.webp", name.trim_end_matches('.'))
            } else {
                name
            },
            mime: if encoded.thumb.is_some() {
                "image/webp"
            } else {
                "application/octet-stream"
            }
            .into(),
            size: encoded.bytes.len() as u64,
            url: format!("/api/v1/chat/files/{id}"),
            thumb_url: thumb
                .as_ref()
                .map(|_| format!("/api/v1/chat/files/{id}?thumb=1")),
            width: encoded.width,
            height: encoded.height,
        };
        db.db.execute(
            "INSERT INTO attachments(id,owner,metadata,hash,thumb) VALUES(?1,?2,?3,?4,?5)",
            params![id, me, serde_json::to_string(&attachment)?, digest, thumb],
        )?;
        Ok(Json(attachment))
    })
    .await
}
#[derive(Deserialize)]
pub(super) struct DownloadQuery {
    thumb: Option<String>,
}
pub(super) async fn download(
    State(s): State<AppState>,
    Extension(Caller(me)): Extension<Caller>,
    Path(id): Path<String>,
    Query(q): Query<DownloadQuery>,
) -> Result<Response, ApiError> {
    let chat = s.chat.ok_or_else(internal)?;
    let (bytes, mime, name) =
        download_bytes(chat, &me, &id, q.thumb.as_deref() == Some("1")).await?;
    let mut response = Body::from(bytes).into_response();
    response.headers_mut().insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(&mime).map_err(|_| internal())?,
    );
    response.headers_mut().insert(
        "x-content-type-options",
        HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        header::CONTENT_DISPOSITION,
        HeaderValue::from_str(&format!(
            "attachment; filename=\"{}\"",
            name.chars().filter(char::is_ascii).collect::<String>()
        ))
        .map_err(|_| internal())?,
    );
    Ok(response)
}
pub(super) async fn download_bytes(
    chat: Arc<Chat>,
    me: &str,
    id: &str,
    thumb: bool,
) -> Result<(Vec<u8>, String, String), ApiError> {
    let me = me.to_owned();
    let id = id.to_owned();
    chat.run(move|db,_|{
        let row:Option<(String,String,Option<String>)>=db.db.query_row("SELECT a.metadata,a.hash,a.thumb FROM attachments a WHERE a.id=?1 AND EXISTS(SELECT 1 FROM message_files f JOIN messages m ON m.id=f.message JOIN channels c ON c.id=m.channel WHERE f.file=a.id AND m.deleted=0 AND (c.id='global' OR c.a=?2 OR c.b=?2))",params![id,me],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?))).optional()?;
        let(metadata,hash,thumbnail)=row.ok_or_else(||ApiError::not_found("attachment not found"))?;
        let attachment:SocialAttachment=serde_json::from_str(&metadata)?;
        let hash=if thumb{thumbnail.ok_or_else(||ApiError::not_found("no thumbnail"))?}else{hash};
        let path=db.files.join(hash);if std::fs::metadata(&path)?.len()>MAX_UPLOAD as u64{return Err(internal());}
        let bytes=std::fs::read(path)?;
        Ok((bytes,attachment.mime,attachment.file_name))
    }).await
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    #[allow(clippy::unwrap_used)]
    fn bounded_image_conversion_and_file_passthrough() {
        let image = image::DynamicImage::new_rgb8(640, 480);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        let output = encode(png.into_inner()).unwrap();
        assert_eq!(output.width, Some(640));
        assert_eq!(output.height, Some(480));
        assert_eq!(
            image::guess_format(&output.bytes).unwrap(),
            image::ImageFormat::WebP
        );
        let thumb = output.thumb.unwrap();
        assert!(thumb.len() <= 256 * 1024);
        let thumb = image::load_from_memory(&thumb).unwrap();
        assert!(thumb.width() <= 320 && thumb.height() <= 320);
        let image = image::DynamicImage::new_rgb8(4097, 1);
        let mut png = Cursor::new(Vec::new());
        image.write_to(&mut png, image::ImageFormat::Png).unwrap();
        assert!(encode(png.into_inner()).is_err());
        let file = encode(b"arbitrary file".to_vec()).unwrap();
        assert_eq!(file.bytes, b"arbitrary file");
        assert!(file.thumb.is_none());
    }
}
