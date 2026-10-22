//! Launch integration: quick play into the room for guests, LAN port pickup
//! from the host's game log.

use std::sync::Arc;

use lc_core::GameObserver;
use lc_runtime::{LaunchContributor, LaunchPlan, QuickPlay};
use lc_types::events::{InstanceLogPayload, InstanceStatePayload};
use lc_types::instance::{Instance, LaunchOpts};
use lc_types::room::RoomRole;
use lc_types::AppResult;

use crate::rooms::{Inner, P2pRooms};

/// Guest: when launched for the current room (`LaunchOpts::room`), joins the
/// tunnel's local address directly. Host: nothing to add (authlib-injector
/// comes from the accounts contributor for `yggdrasil` rooms).
pub struct RoomLaunchContributor {
    inner: Arc<Inner>,
}

impl P2pRooms {
    pub fn launch_contributor(&self) -> Arc<dyn LaunchContributor> {
        Arc::new(RoomLaunchContributor {
            inner: self.inner().clone(),
        })
    }
}

impl LaunchContributor for RoomLaunchContributor {
    fn name(&self) -> &'static str {
        "p2p"
    }

    fn contribute(
        &self,
        _instance: &Instance,
        opts: &LaunchOpts,
        plan: &mut LaunchPlan,
    ) -> AppResult<()> {
        let Some(room) = opts.room else {
            return Ok(());
        };
        if opts.join_server.is_some() || plan.quick_play.is_some() {
            return Ok(());
        }
        let Some(s) = self.inner.current() else {
            return Ok(());
        };
        if s.room_id != room || s.role != RoomRole::Guest {
            return Ok(());
        }
        if let Some(addr) = s.local_addr {
            plan.quick_play = Some(QuickPlay {
                host: addr.ip().to_string(),
                port: addr.port(),
            });
        }
        Ok(())
    }
}

/// `GameObserver` decorator that also feeds game log lines to the rooms
/// service (LAN port detection), then forwards everything to `inner`.
pub struct RoomLogTap {
    rooms: P2pRooms,
    inner: Arc<dyn GameObserver>,
}

impl RoomLogTap {
    pub fn new(rooms: P2pRooms, inner: Arc<dyn GameObserver>) -> Self {
        Self { rooms, inner }
    }
}

impl GameObserver for RoomLogTap {
    fn on_state(&self, payload: InstanceStatePayload) {
        self.inner.on_state(payload);
    }

    fn on_log(&self, payload: InstanceLogPayload) {
        for line in &payload.lines {
            self.rooms.feed_game_log(payload.id, line);
        }
        self.inner.on_log(payload);
    }
}
