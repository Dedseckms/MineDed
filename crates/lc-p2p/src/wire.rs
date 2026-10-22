//! Line framing for `proto.rs` messages, password proofs and peer tokens.

use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use lc_types::{AppError, AppResult, ErrorCode, RoomId};
use serde::Serialize;
use sha2::Sha256;
use tokio::io::{AsyncBufRead, AsyncBufReadExt, AsyncRead, AsyncReadExt};

use crate::proto::ControlMsg;

/// Upper bound for one control line (chat text is limited far below).
pub(crate) const MAX_LINE: usize = 64 * 1024;
/// Upper bound for the `StreamOpen` line of a game stream.
pub(crate) const MAX_OPEN_LINE: usize = 256;

/// JSON line with a trailing `\n`.
pub(crate) fn encode<T: Serialize>(msg: &T) -> AppResult<String> {
    let mut line =
        serde_json::to_string(msg).map_err(|e| AppError::internal(format!("p2p json: {e}")))?;
    line.push('\n');
    Ok(line)
}

pub(crate) fn decode(line: &str) -> AppResult<ControlMsg> {
    serde_json::from_str(line).map_err(proto_err)
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

/// Label for TLS keying material export (RFC 5705) used by password proofs;
/// the export context is the room id.
pub(crate) const EKM_LABEL: &[u8] = b"lc/1 room password";

/// `base64url(HMAC-SHA256(key = password, msg = ekm))` (`Hello.password_proof`).
/// `ekm` comes from the QUIC/TLS session, so a proof is bound to one
/// connection and useless to anyone who records it; the hub never sees it.
pub(crate) fn password_proof(password: &str, ekm: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(mac(password.as_bytes(), ekm).finalize().into_bytes())
}

/// Constant-time check of a proof produced by [`password_proof`].
pub(crate) fn verify_password_proof(password: &str, ekm: &[u8], proof: &str) -> bool {
    let Ok(raw) = URL_SAFE_NO_PAD.decode(proof) else {
        return false;
    };
    mac(password.as_bytes(), ekm).verify_slice(&raw).is_ok()
}

fn peer_token_msg(room_id: RoomId, endpoint_id: &str, uuid: &str, exp: u64) -> String {
    format!("lc-peer/2|{room_id}|{endpoint_id}|{uuid}|{exp}")
}

/// Hub `peerToken` v2 (`<exp>.<uuid|->.<mac>`, see hub.yaml joinRoom). `uuid`
/// is the undashed profile UUID of an authenticated join. Used by tests and
/// by any local hub implementation.
pub fn make_peer_token(
    host_token: &str,
    room_id: RoomId,
    endpoint_id: &str,
    uuid: Option<&str>,
    exp: u64,
) -> String {
    let uuid = uuid.unwrap_or("-");
    let m = mac(
        host_token.as_bytes(),
        peer_token_msg(room_id, endpoint_id, uuid, exp).as_bytes(),
    );
    format!(
        "{exp}.{uuid}.{}",
        URL_SAFE_NO_PAD.encode(m.finalize().into_bytes())
    )
}

/// Verifies a hub `peerToken` against the TLS-authenticated endpoint id of
/// the connecting guest. `now` is unix seconds. `Some(uuid)` when valid
/// (`uuid` is `None` for anonymous joins), `None` when invalid or expired.
pub(crate) fn verify_peer_token(
    token: &str,
    host_token: &str,
    room_id: RoomId,
    endpoint_id: &str,
    now: u64,
) -> Option<Option<String>> {
    let mut parts = token.splitn(3, '.');
    let (exp, uuid, tag) = (parts.next()?, parts.next()?, parts.next()?);
    let exp_n = exp.parse::<u64>().ok()?;
    if exp_n < now || uuid.is_empty() {
        return None;
    }
    let raw = URL_SAFE_NO_PAD.decode(tag).ok()?;
    mac(
        host_token.as_bytes(),
        peer_token_msg(room_id, endpoint_id, uuid, exp_n).as_bytes(),
    )
    .verify_slice(&raw)
    .ok()?;
    Some((uuid != "-").then(|| uuid.to_owned()))
}
fn mac(key: &[u8], msg: &[u8]) -> HmacSha256 {
    // HMAC accepts keys of any length, so this never fails; the fallback
    // (empty key) is unreachable and cannot match a real tag anyway.
    let mut m = <HmacSha256 as Mac>::new_from_slice(key)
        .unwrap_or_else(|_| <HmacSha256 as Mac>::new(&Default::default()));
    m.update(msg);
    m
}

pub(crate) fn proto_err(e: impl std::fmt::Display) -> AppError {
    AppError::new(ErrorCode::Network, format!("p2p protocol error: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hello_roundtrip() {
        let msg = ControlMsg::Hello {
            protocol: 1,
            name: "Steve".into(),
            uuid: None,
            peer_token: "tok".into(),
            password_proof: Some("ab".into()),
        };
        let line = encode(&msg).unwrap();
        assert!(line.ends_with('\n'));
        assert_eq!(decode(line.trim_end()).unwrap(), msg);
    }

    #[test]
    fn password_proof_checks() {
        let ekm = [7u8; 32];
        let p = password_proof("secret", &ekm);
        assert!(!p.contains('='));
        assert!(verify_password_proof("secret", &ekm, &p));
        assert!(!verify_password_proof("Secret", &ekm, &p));
        assert!(!verify_password_proof("secret", &[8u8; 32], &p));
        assert!(!verify_password_proof("secret", &ekm, "zz!"));
        assert!(!verify_password_proof("secret", &ekm, ""));
    }

    #[test]
    fn peer_token_checks() {
        let room = RoomId::new();
        let t = make_peer_token("host", room, "ep1", None, 1000);
        assert_eq!(verify_peer_token(&t, "host", room, "ep1", 999), Some(None));
        assert_eq!(verify_peer_token(&t, "host", room, "ep1", 1000), Some(None));
        assert_eq!(verify_peer_token(&t, "host", room, "ep1", 1001), None); // expired
        assert_eq!(verify_peer_token(&t, "host", room, "ep2", 999), None); // other endpoint
        assert_eq!(verify_peer_token(&t, "other", room, "ep1", 999), None);
        assert_eq!(
            verify_peer_token(&t, "host", RoomId::new(), "ep1", 999),
            None
        );
        let forged = t.replacen("1000", "9999", 1);
        assert_eq!(verify_peer_token(&forged, "host", room, "ep1", 999), None);
        assert_eq!(verify_peer_token("garbage", "host", room, "ep1", 0), None);
        assert_eq!(verify_peer_token("", "host", room, "ep1", 0), None);

        let u = make_peer_token("host", room, "ep1", Some("abcdef0123"), 1000);
        assert_eq!(
            verify_peer_token(&u, "host", room, "ep1", 0),
            Some(Some("abcdef0123".into()))
        );
        // Swapping the uuid breaks the MAC.
        let swapped = u.replacen("abcdef0123", "-", 1);
        assert_eq!(verify_peer_token(&swapped, "host", room, "ep1", 0), None);
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
