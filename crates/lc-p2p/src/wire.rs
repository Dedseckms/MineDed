//! Line framing for `proto.rs` messages plus optional extension fields.
//!
//! Extension fields are extra keys in the same JSON object. Peers that do not
//! know them ignore them (`ControlMsg` does not deny unknown fields), so the
//! wire stays compatible with `proto.rs`. They are pending a CONTRACT REQUEST
//! to become regular fields:
//! - `password_proof` on `hello`: HMAC proof of the room password;
//! - `from` / `from_name` on `chat`: original sender when the host forwards.

use hmac::{Hmac, Mac};
use lc_types::{AppError, AppResult, ErrorCode};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};

use crate::proto::ControlMsg;

/// Upper bound for one control line (chat text is limited far below).
pub(crate) const MAX_LINE: usize = 64 * 1024;
/// Upper bound for the `StreamOpen` line of a game stream.
pub(crate) const MAX_OPEN_LINE: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Ext {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub password_proof: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_name: Option<String>,
}

/// JSON line (with trailing `\n`) for a message plus extension fields.
pub(crate) fn encode(msg: &ControlMsg, ext: &Ext) -> AppResult<String> {
    let mut v = serde_json::to_value(msg).map_err(json_err)?;
    if let (Some(obj), serde_json::Value::Object(extra)) = (
        v.as_object_mut(),
        serde_json::to_value(ext).map_err(json_err)?,
    ) {
        for (k, val) in extra {
            obj.insert(k, val);
        }
    }
    let mut line = serde_json::to_string(&v).map_err(json_err)?;
    line.push('\n');
    Ok(line)
}

pub(crate) fn encode_plain<T: Serialize>(msg: &T) -> AppResult<String> {
    let mut line = serde_json::to_string(msg).map_err(json_err)?;
    line.push('\n');
    Ok(line)
}

pub(crate) fn decode(line: &str) -> AppResult<(ControlMsg, Ext)> {
    let msg: ControlMsg = serde_json::from_str(line).map_err(proto_err)?;
    let ext: Ext = serde_json::from_str(line).map_err(proto_err)?;
    Ok((msg, ext))
}

/// Reads one `\n`-terminated line of at most `max` bytes. `Ok(None)` on a
/// clean end of stream before any byte.
pub(crate) async fn read_line<R: AsyncBufRead + Unpin>(
    r: &mut R,
    max: usize,
) -> AppResult<Option<String>> {
    let mut buf = Vec::new();
    loop {
        let chunk = r.fill_buf().await?;
        if chunk.is_empty() {
            if buf.is_empty() {
                return Ok(None);
            }
            return Err(proto_err("stream ended mid-line"));
        }
        if let Some(pos) = chunk.iter().position(|b| *b == b'\n') {
            buf.extend_from_slice(&chunk[..pos]);
            r.consume(pos + 1);
            break;
        }
        let n = chunk.len();
        buf.extend_from_slice(chunk);
        r.consume(n);
        if buf.len() > max {
            return Err(proto_err("line too long"));
        }
    }
    if buf.len() > max {
        return Err(proto_err("line too long"));
    }
    String::from_utf8(buf).map(Some).map_err(proto_err)
}

/// Unbuffered variant: reads byte by byte so nothing after the line is
/// consumed (the rest of a game stream is raw TCP payload).
pub(crate) async fn read_line_unbuffered<R: AsyncRead + Unpin>(
    r: &mut R,
    max: usize,
) -> AppResult<String> {
    let mut buf = Vec::new();
    loop {
        let b = r.read_u8().await?;
        if b == b'\n' {
            break;
        }
        buf.push(b);
        if buf.len() > max {
            return Err(proto_err("line too long"));
        }
    }
    String::from_utf8(buf).map_err(proto_err)
}

type HmacSha256 = Hmac<Sha256>;

/// Label for TLS keying material export (RFC 5705) used by password proofs.
pub(crate) const EKM_LABEL: &[u8] = b"lc/1 room password";

/// `hex(HMAC-SHA256(key = password, msg = ekm))`. `ekm` is exported from the
/// QUIC/TLS session, so a proof is bound to one connection and useless to
/// anyone who records it; the hub never sees either side.
pub(crate) fn password_proof(password: &str, ekm: &[u8]) -> String {
    hex::encode(mac(password, ekm).finalize().into_bytes())
}

/// Constant-time check of a proof produced by [`password_proof`].
pub(crate) fn verify_password_proof(password: &str, ekm: &[u8], proof: &str) -> bool {
    let Ok(raw) = hex::decode(proof) else {
        return false;
    };
    mac(password, ekm).verify_slice(&raw).is_ok()
}

fn mac(password: &str, ekm: &[u8]) -> HmacSha256 {
    // HMAC accepts keys of any length, so this never fails; fall back to an
    // empty-key MAC that cannot match a real proof just in case.
    let mut m = <HmacSha256 as Mac>::new_from_slice(password.as_bytes())
        .unwrap_or_else(|_| <HmacSha256 as Mac>::new(&Default::default()));
    m.update(ekm);
    m
}

fn json_err(e: serde_json::Error) -> AppError {
    AppError::internal(format!("p2p json: {e}"))
}

pub(crate) fn proto_err(e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Network, format!("p2p protocol error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extension_fields_roundtrip_and_are_ignored_by_plain_decoder() {
        let msg = ControlMsg::Hello {
            protocol: 1,
            name: "Steve".into(),
            uuid: None,
            peer_token: "tok".into(),
        };
        let ext = Ext {
            password_proof: Some("ab".into()),
            ..Ext::default()
        };
        let line = encode(&msg, &ext).unwrap();
        assert!(line.ends_with('\n'));
        let (back, back_ext) = decode(line.trim_end()).unwrap();
        assert_eq!(back, msg);
        assert_eq!(back_ext, ext);
        // A peer that only knows proto.rs still parses the message.
        let plain: ControlMsg = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(plain, msg);
    }

    #[test]
    fn password_proof_checks() {
        let ekm = [7u8; 32];
        let p = password_proof("secret", &ekm);
        assert!(verify_password_proof("secret", &ekm, &p));
        assert!(!verify_password_proof("Secret", &ekm, &p));
        assert!(!verify_password_proof("secret", &[8u8; 32], &p));
        assert!(!verify_password_proof("secret", &ekm, "zz"));
        assert!(!verify_password_proof("secret", &ekm, ""));
    }

    #[tokio::test]
    async fn line_reader_limits() {
        let data = b"{\"t\":\"bye\"}\nrest".to_vec();
        let mut r = tokio::io::BufReader::new(&data[..]);
        assert_eq!(
            read_line(&mut r, 100).await.unwrap().as_deref(),
            Some("{\"t\":\"bye\"}")
        );
        assert!(read_line(&mut r, 100).await.is_err()); // ends mid-line
        let long = vec![b'a'; 300];
        let mut r = tokio::io::BufReader::new(&long[..]);
        assert!(read_line(&mut r, 100).await.is_err());
        let mut empty = tokio::io::BufReader::new(&b""[..]);
        assert_eq!(read_line(&mut empty, 100).await.unwrap(), None);

        let mut raw = &b"{\"t\":\"game\"}\nPAYLOAD"[..];
        assert_eq!(
            read_line_unbuffered(&mut raw, MAX_OPEN_LINE).await.unwrap(),
            "{\"t\":\"game\"}"
        );
        assert_eq!(raw, b"PAYLOAD");
    }
}
