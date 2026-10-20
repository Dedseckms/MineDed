//! Service container. Swapping a stub for a real implementation happens only
//! here (lead), so roles never touch each other's crates.

use std::sync::Arc;

use lc_accounts::{AccountService, AuthProvider, SkinService, StubAccounts, StubSkins};
use lc_content::{ContentService, PackIo, PresetService, StubContent};
use lc_core::{GameLauncher, InstanceService, JavaManager, StubCore, SystemProbe, VersionService};
use lc_http::{Downloader, StubDownloader};
use lc_loaders::{LoaderRegistry, StubLoader};
use lc_p2p::{DatagramTransport, RoomService, StubRooms};
use lc_runtime::{MemorySettingsStore, SettingsStore};
use lc_types::version::LoaderKind;
use lc_voice::{StubVoice, VoiceEngine};
use lc_vpn::{StubVpn, VpnController};

use crate::tasks::TaskRegistry;

pub struct AppState {
    pub tasks: Arc<TaskRegistry>,
    pub settings: Arc<dyn SettingsStore>,
    pub downloader: Arc<dyn Downloader>,
    // core
    pub versions: Arc<dyn VersionService>,
    pub java: Arc<dyn JavaManager>,
    pub instances: Arc<dyn InstanceService>,
    pub launcher: Arc<dyn GameLauncher>,
    pub system: Arc<dyn SystemProbe>,
    // mods
    pub loaders: Arc<LoaderRegistry>,
    pub content: Arc<dyn ContentService>,
    pub packs: Arc<dyn PackIo>,
    pub presets: Arc<dyn PresetService>,
    // accounts
    pub accounts: Arc<dyn AccountService>,
    pub auth: Arc<dyn AuthProvider>,
    pub skins: Arc<dyn SkinService>,
    // net
    pub vpn: Arc<dyn VpnController>,
    pub rooms: Arc<dyn RoomService>,
    pub datagrams: Arc<dyn DatagramTransport>,
    // voice
    pub voice: Arc<dyn VoiceEngine>,
}

impl AppState {
    /// Every service is a stub that returns `notImplemented`.
    pub fn stubs() -> Self {
        let core = Arc::new(StubCore);
        let accounts = Arc::new(StubAccounts);
        let content = Arc::new(StubContent);
        let rooms = Arc::new(StubRooms);

        let mut loaders = LoaderRegistry::default();
        for kind in [
            LoaderKind::Fabric,
            LoaderKind::Quilt,
            LoaderKind::Forge,
            LoaderKind::NeoForge,
        ] {
            loaders.register(Arc::new(StubLoader(kind)));
        }

        Self {
            tasks: Arc::new(TaskRegistry::default()),
            settings: Arc::new(MemorySettingsStore::default()),
            downloader: Arc::new(StubDownloader),
            versions: core.clone(),
            java: core.clone(),
            instances: core.clone(),
            launcher: core.clone(),
            system: core,
            loaders: Arc::new(loaders),
            content: content.clone(),
            packs: content.clone(),
            presets: content,
            accounts: accounts.clone(),
            auth: accounts,
            skins: Arc::new(StubSkins),
            vpn: Arc::new(StubVpn),
            rooms: rooms.clone(),
            datagrams: rooms,
            voice: Arc::new(StubVoice),
        }
    }
}
