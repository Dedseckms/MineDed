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
use lc_p2p::{DatagramTransport, RoomService, StubRooms};
use lc_runtime::{MemorySettingsStore, Paths, SettingsStore};
use lc_types::settings::Settings;
use lc_types::version::LoaderKind;
use lc_types::AppResult;
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
    /// Real services where implemented (core, http, accounts, skins, loaders,
    /// content, settings);
    /// everything else stays a stub.
    pub fn real(
        paths: Paths,
        settings: Arc<dyn SettingsStore>,
        initial: &Settings,
        observer: Arc<dyn GameObserver>,
    ) -> AppResult<Self> {
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

        let core = Arc::new(CoreServices::new(
            paths.clone(),
            downloader.clone(),
            accounts.clone(),
            loaders.clone(),
            observer,
            Vec::new(),
        ));
        let content = Arc::new(ContentServices::new(
            paths,
            downloader.clone(),
            core.clone(),
            core.clone(),
        ));
        let rooms = Arc::new(StubRooms);

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
            vpn: Arc::new(StubVpn),
            rooms: rooms.clone(),
            datagrams: rooms,
            voice: Arc::new(StubVoice),
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
            rooms: rooms.clone(),
            datagrams: rooms,
            voice: Arc::new(StubVoice),
        }
    }
}
