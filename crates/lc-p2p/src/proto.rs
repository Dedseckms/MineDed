//! CONTRACT (lead): wire protocol between launchers inside a room.
//!
//! One iroh/QUIC connection per peer pair, ALPN [`ALPN`]:
//! - first bi-stream: control, newline-delimited JSON [`ControlMsg`];
//! - each further bi-stream: one game TCP connection. The opener writes a
//!   single [`StreamOpen`] JSON line, then raw bytes in both directions;
//! - QUIC datagrams: voice, [`VoiceHeader`] + Opus frame (20 ms).

use serde::{Deserialize, Serialize};

pub const ALPN: &[u8] = b"lc/1";
pub const PROTOCOL_VERSION: u32 = 1;
/// Above this many peers the host forwards voice instead of full mesh.
pub const MESH_LIMIT: usize = 6;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum ControlMsg {
    Hello {
        protocol: u32,
        name: String,
        uuid: Option<String>,
        /// Hub-issued token proving room membership; the host verifies it with
        /// its `hostToken` (format in `server/openapi/hub.yaml`, joinRoom).
        peer_token: String,
        /// Base64url HMAC-SHA256(key = password, msg = TLS-exported keying
        /// material, context = room id). `None` for rooms without a password.
        #[serde(default)]
        password_proof: Option<String>,
    },
    Welcome {
        peers: Vec<String>,
        /// Display names, same order as `peers`.
        #[serde(default)]
        peer_names: Vec<String>,
    },
    Ping {
        nonce: u64,
    },
    Pong {
        nonce: u64,
    },
    Chat {
        id: String,
        text: String,
        sent_at: String,
        /// Original sender when the host forwards a guest's message.
        #[serde(default)]
        from: Option<String>,
        #[serde(default)]
        from_name: Option<String>,
    },
    VoiceState {
        speaking: bool,
        muted: bool,
    },
    Kick {
        reason: String,
    },
    Bye,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "t", rename_all = "snake_case")]
pub enum StreamOpen {
    /// Guest → host: proxy to the host's LAN game port.
    Game,
}

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ProtoError {
    #[error("datagram too short")]
    TooShort,
    #[error("unknown datagram version {0}")]
    Version(u8),
}

/// 8-byte voice datagram header, big endian:
/// `ver:u8 | speaker:u8 | seq:u16 | timestamp_ms:u32`.
/// `speaker` is the peer index assigned by the host in `Welcome` order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoiceHeader {
    pub speaker: u8,
    pub seq: u16,
    pub timestamp_ms: u32,
}

impl VoiceHeader {
    pub const LEN: usize = 8;
    pub const VERSION: u8 = 1;

    pub fn encode(&self, opus: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(Self::LEN + opus.len());
        out.push(Self::VERSION);
        out.push(self.speaker);
        out.extend_from_slice(&self.seq.to_be_bytes());
        out.extend_from_slice(&self.timestamp_ms.to_be_bytes());
        out.extend_from_slice(opus);
        out
    }

    pub fn decode(buf: &[u8]) -> Result<(Self, &[u8]), ProtoError> {
        if buf.len() < Self::LEN {
            return Err(ProtoError::TooShort);
        }
        if buf[0] != Self::VERSION {
            return Err(ProtoError::Version(buf[0]));
        }
        let header = Self {
            speaker: buf[1],
            seq: u16::from_be_bytes([buf[2], buf[3]]),
            timestamp_ms: u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]),
        };
        Ok((header, &buf[Self::LEN..]))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn voice_header_roundtrip() {
        let h = VoiceHeader {
            speaker: 3,
            seq: 513,
            timestamp_ms: 123_456,
        };
        let buf = h.encode(&[9, 9]);
        let (back, payload) = VoiceHeader::decode(&buf).unwrap();
        assert_eq!(back, h);
        assert_eq!(payload, &[9, 9]);
        assert_eq!(VoiceHeader::decode(&buf[..3]), Err(ProtoError::TooShort));
    }

    #[test]
    fn control_msg_json() {
        let s = serde_json::to_string(&ControlMsg::Ping { nonce: 1 }).unwrap();
        assert_eq!(s, r#"{"t":"ping","nonce":1}"#);
    }
}
