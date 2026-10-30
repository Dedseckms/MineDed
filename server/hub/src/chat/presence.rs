//! Public launcher counter and the admin presence list.
//! A launcher counts while its heartbeat is younger than [`PRESENCE_TTL`]; a
//! chat WebSocket counts too, so the number is "launchers on air", not "humans".
use super::{internal, store::Store, Chat};
use crate::error::ApiError;
use crate::rfc3339;
use lc_types::social::{AdminPresence, PresenceClient, PresenceSummary};
use std::time::{Duration, Instant, SystemTime};

/// Two missed ticks of the 60 s launcher heartbeat.
pub(super) const PRESENCE_TTL: Duration = Duration::from_secs(120);
/// Bound on remembered client ids: a flood must not grow memory without limit.
const MAX_CLIENTS: usize = 50_000;
/// The admin list is a diagnostic view, not an export.
const MAX_LISTED: usize = 500;

/// Self-reported launcher kind from `POST /presence/beat`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    Offline,
    Yggdrasil,
    Anonymous,
}
impl Kind {
    /// Wire value of `POST /presence/beat`; unknown values are rejected.
    fn parse(raw: &str) -> Result<Self, ApiError> {
        match raw {
            "offline" => Ok(Kind::Offline),
            "yggdrasil" => Ok(Kind::Yggdrasil),
            "" | "anonymous" => Ok(Kind::Anonymous),
            _ => Err(super::invalid("invalid client kind")),
        }
    }
    /// Kind reported in the admin list; an offline launcher is a guest client.
    fn label(self) -> &'static str {
        match self {
            Kind::Offline => "guest",
            Kind::Yggdrasil => "yggdrasil",
            Kind::Anonymous => "anonymous",
        }
    }
}

pub(super) struct Beat {
    kind: Kind,
    since: SystemTime,
    seen: Instant,
}

impl Chat {
    /// Records a heartbeat; a repeated `client_id` only extends itself.
    pub fn beat(&self, client_id: &str, kind: &str) -> Result<(), ApiError> {
        self.beat_at(client_id, Kind::parse(kind)?, Instant::now())
    }
    pub(super) fn beat_at(&self, client_id: &str, kind: Kind, at: Instant) -> Result<(), ApiError> {
        let mut beats = self.beats.lock().map_err(|_| internal())?;
        if let Some(beat) = beats.get_mut(client_id) {
            beat.kind = kind;
            beat.seen = at;
            return Ok(());
        }
        if beats.len() >= MAX_CLIENTS {
            beats.retain(|_, beat| at.saturating_duration_since(beat.seen) < PRESENCE_TTL);
            if beats.len() >= MAX_CLIENTS {
                return Err(ApiError::rate_limited(PRESENCE_TTL.as_secs()));
            }
        }
        beats.insert(
            client_id.to_owned(),
            Beat {
                kind,
                since: SystemTime::now(),
                seen: at,
            },
        );
        Ok(())
    }
    /// Unique beating launchers plus launchers connected to the chat socket.
    pub fn online(&self) -> PresenceSummary {
        self.online_at(Instant::now())
    }
    pub(super) fn online_at(&self, at: Instant) -> PresenceSummary {
        let beating = self
            .beats
            .lock()
            .map(|beats| {
                beats
                    .values()
                    .filter(|beat| at.saturating_duration_since(beat.seen) < PRESENCE_TTL)
                    .count()
            })
            .unwrap_or_default();
        let sockets = self
            .presence
            .lock()
            .map(|presence| {
                presence
                    .values()
                    .filter(|session| session.sockets > 0)
                    .count()
            })
            .unwrap_or_default();
        PresenceSummary {
            online: u32::try_from(beating.saturating_add(sockets)).unwrap_or(u32::MAX),
        }
    }
    /// Drops expired heartbeats; called from the request and reaper sweeps.
    pub(super) fn sweep_beats(&self) {
        let now = Instant::now();
        if let Ok(mut beats) = self.beats.lock() {
            beats.retain(|_, beat| now.saturating_duration_since(beat.seen) < PRESENCE_TTL);
        }
    }
}

/// Admin view: beating launchers stay anonymous, socket clients show their name.
pub(super) fn admin_presence(chat: &Chat, db: &Store) -> Result<AdminPresence, ApiError> {
    let now = Instant::now();
    let mut clients = Vec::new();
    let beats = chat.beats.lock().map_err(|_| internal())?;
    for beat in beats.values() {
        if now.saturating_duration_since(beat.seen) >= PRESENCE_TTL {
            continue;
        }
        clients.push(PresenceClient {
            label: "anonymous".into(),
            kind: beat.kind.label().into(),
            since: rfc3339::format(beat.since),
        });
    }
    drop(beats);
    let sessions: Vec<(String, SystemTime)> = chat
        .presence
        .lock()
        .map_err(|_| internal())?
        .iter()
        .filter(|(_, session)| session.sockets > 0)
        .map(|(id, session)| (id.clone(), session.since))
        .collect();
    for (id, since) in sessions {
        // A stale socket without a profile cannot be named; keep it anonymous.
        let (label, guest) = db
            .label(&id)?
            .unwrap_or_else(|| ("anonymous".to_owned(), false));
        clients.push(PresenceClient {
            label,
            kind: if guest { "guest" } else { "yggdrasil" }.into(),
            since: rfc3339::format(since),
        });
    }
    clients.truncate(MAX_LISTED);
    Ok(AdminPresence {
        online: chat.online().online,
        clients,
    })
}
