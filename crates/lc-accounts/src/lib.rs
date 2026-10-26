//! Accounts and skins. Owner: accounts. `contract.rs` is owned by the lead.

pub mod authlib;
pub mod contract;
mod fsutil;
pub mod offline;
mod remote;
mod service;
mod skins;
mod stub;
mod tokens;
pub mod yggdrasil;

#[cfg(test)]
mod mock_tests;

pub use authlib::AuthlibManager;
pub use contract::{
    AccountService, AuthProvider, AuthlibInjector, LaunchAuth, Secret, SkinService, TokenStore,
    UserType,
};
pub use service::{token_key, AccountsService, ACCOUNTS_FILE};
pub use skins::{validate_skin_png, SkinsService, SKINS_INDEX};
pub use stub::{StubAccounts, StubSkins};
pub use tokens::{
    EncryptedFileTokenStore, KeyringTokenStore, MemoryTokenStore, FALLBACK_FILE, FALLBACK_KEY_FILE,
    KEYRING_SERVICE,
};
pub use yggdrasil::{parse_server_input, DEFAULT_API_ROOT};
