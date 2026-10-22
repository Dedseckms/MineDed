//! Service container. Swapping a stub for a real implementation happens only
//! here (lead), so roles never touch each other's crates.

use std::sync::Arc;

use lc_accounts::{
    AccountService, AccountsService, AuthProvider, KeyringTokenStore, SkinService, SkinsService,
    StubAccounts, StubSkins, TokenStore,
};
use lc_content::{ContentService, ContentServices, PackIo, PresetService, StubContent};
use lc_core::{
    CoreServices, GameLauncher, GameObserver, InstanceService, JavaManager, StubCore, SystemProbe,
    VersionService,
};
use lc_http::{Downloader, DownloaderConfig, HttpDownloader, StubDownloader};
use lc_loaders::{LoaderRegistry, StubLoader};
use lc_p2p::{
    CoreRoomEnv, DatagramTransport, P2pConfig, P2pRooms, RoomLogTap, RoomObserver, RoomService,
    StubRooms,
};
use lc_runtime::{MemorySettingsStore, Paths, SettingsStore};
use lc_types::settings::Settings;
use lc_types::version::LoaderKind;
use lc_types::AppResult;
use lc_voice::{CpalVoiceEngine, StubVoice, VoiceEngine, VoiceObserver};
use lc_vpn::{MihomoVpn, StubVpn, VpnController};

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
    /// Concrete handle for restore on start and kill on exit.
    pub mihomo: Option<Arc<MihomoVpn>>,
    pub rooms: Arc<dyn RoomService>,
    pub datagrams: Arc<dyn DatagramTransport>,
    // voice
    pub voice: Arc<dyn VoiceEngine>,
}

impl AppState {
    /// Real services everywhere.
    pub fn real(
        paths: Paths,
        settings: Arc<dyn SettingsStore>,
        initial: &Settings,
        observer: Arc<dyn GameObserver>,
        room_observer: Arc<dyn RoomObserver>,
        voice_observer: Arc<dyn VoiceObserver>,
    ) -> AppResult<Self> {
        let voice = Arc::new(CpalVoiceEngine::new(&paths, voice_observer));
        let downloader: Arc<dyn Downloader> = Arc::new(HttpDownloader::new(
            DownloaderConfig::from_settings(initial),
        )?);
        let tokens: Arc<dyn TokenStore> = Arc::new(KeyringTokenStore::new(&paths));
        let accounts = Arc::new(
            AccountsService::new(&paths, downloader.clone(), tokens)
                .with_settings(settings.clone()),
        );
        let skins = Arc::new(SkinsService::new(&paths, accounts.clone()));
        let loaders = Arc::new(lc_loaders::default_registry(downloader.clone()));
        let mihomo = Arc::new(MihomoVpn::new(paths.clone(), downloader.clone()));
        let room_env = Arc::new(CoreRoomEnv::new(accounts.clone()));
        let p2p = P2pRooms::new(
            P2pConfig::default(),
            downloader.clone(),
            settings.clone(),
            room_env.clone(),
            room_observer,
        );

        let core = Arc::new(CoreServices::new(
            paths.clone(),
            downloader.clone(),
            accounts.clone(),
            loaders.clone(),
            Arc::new(RoomLogTap::new(p2p.clone(), observer)),
            vec![mihomo.clone(), p2p.launch_contributor()],
        ));
        room_env.attach_instances(core.clone());
        let content = Arc::new(ContentServices::new(
            paths,
            downloader.clone(),
            core.clone(),
            core.clone(),
        ));
        let rooms = Arc::new(p2p);

        Ok(Self {
            tasks: Arc::new(TaskRegistry::default()),
            settings,
            downloader,
            versions: core.clone(),
            java: core.clone(),
            instances: core.clone(),
            launcher: core.clone(),
            system: core,
            loaders,
            content: content.clone(),
            packs: content.clone(),
            presets: content,
            accounts: accounts.clone(),
            auth: accounts,
            skins,
            vpn: mihomo.clone(),
            mihomo: Some(mihomo),
            rooms: rooms.clone(),
            datagrams: rooms,
            voice,
        })
    }

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
            mihomo: None,
            rooms: rooms.clone(),
            datagrams: rooms,
            voice: Arc::new(StubVoice),
        }
    }
}
