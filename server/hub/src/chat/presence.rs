//! Public launcher counter and the admin presence list.
//! A launcher counts while its heartbeat is younger than [`PRESENCE_TTL`]; one
//! launcher reports one client id, so the counter is "launchers on air", not "humans".
use super::{internal, store::Store, Chat};
use crate::error::ApiError;
use crate::rfc3339;
use lc_types::social::{AdminPresence, PresenceClient, PresenceSummary};
use std::time::{Duration, Instant};

/// Two missed ticks of the 60 s launcher heartbeat.
pub(super) const PRESENCE_TTL: Duration = Duration::from_secs(120);
/// Bound on remembered client ids: a flood must not grow memory without limit.
const MAX_CLIENTS: usize = 50_000;
/// The admin list is a diagnostic view, not an export.
const MAX_LISTED: usize = 500;

/// Self-reported launcher kind from `POST /presence/beat`.
/// It carries no identity, so it only validates the wire value: the admin list
/// names authenticated chat sessions instead.
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
}

pub(super) struct Beat {
    seen: Instant,
}

impl Chat {
    /// Records a heartbeat; a repeated `client_id` only extends itself.
    pub fn beat(&self, client_id: &str, kind: &str) -> Result<(), ApiError> {
        Kind::parse(kind)?;
        self.beat_at(client_id, Instant::now())
    }
    pub(super) fn beat_at(&self, client_id: &str, at: Instant) -> Result<(), ApiError> {
        let mut beats = self.beats.lock().map_err(|_| internal())?;
        if let Some(beat) = beats.get_mut(client_id) {
            beat.seen = at;
            return Ok(());
        }
        if beats.len() >= MAX_CLIENTS {
            beats.retain(|_, beat| at.saturating_duration_since(beat.seen) < PRESENCE_TTL);
            if beats.len() >= MAX_CLIENTS {
                return Err(ApiError::rate_limited(PRESENCE_TTL.as_secs()));
            }
        }
        beats.insert(client_id.to_owned(), Beat { seen: at });
        Ok(())
    }
    /// Unique beating launchers. Chat sockets are not added on top: every
    /// launcher that keeps a socket open also beats under the same process, so
    /// a second count would report it twice.
    pub fn online(&self) -> PresenceSummary {
        self.online_at(Instant::now())
    }
    pub(super) fn online_at(&self, at: Instant) -> PresenceSummary {
        let online = self
            .beats
            .lock()
            .map(|beats| {
                beats
                    .values()
                    .filter(|beat| at.saturating_duration_since(beat.seen) < PRESENCE_TTL)
                    .count()
            })
            .unwrap_or_default();
        PresenceSummary {
            online: u32::try_from(online).unwrap_or(u32::MAX),
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

/// Admin view: authenticated chat sessions with their names only. A heartbeat
/// carries no identity, so anonymous launchers raise `online` without becoming
/// a client entry; an empty list with a non-zero counter is expected.
pub(super) fn admin_presence(chat: &Chat, db: &Store) -> Result<AdminPresence, ApiError> {
    let sessions: Vec<(String, std::time::SystemTime)> = chat
        .presence
        .lock()
        .map_err(|_| internal())?
        .iter()
        .filter(|(_, session)| session.sockets > 0)
        .map(|(id, session)| (id.clone(), session.since))
        .collect();
    let mut clients = Vec::new();
    for (id, since) in sessions {
        // A socket without a profile row cannot be named; nothing to show.
        let Some((label, guest)) = db.label(&id)? else {
            continue;
        };
        clients.push(PresenceClient {
            label,
            kind: if guest { "guest" } else { "yggdrasil" }.into(),
            since: rfc3339::format(since),
        });
    }
    // Stable order for the admin panel: oldest session first, then by name.
    clients.sort_by(|a, b| a.since.cmp(&b.since).then_with(|| a.label.cmp(&b.label)));
    clients.truncate(MAX_LISTED);
    Ok(AdminPresence {
        online: chat.online().online,
        clients,
    })
}
