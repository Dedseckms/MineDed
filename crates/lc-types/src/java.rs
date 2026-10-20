use serde::{Deserialize, Serialize};
use specta::Type;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Type)]
#[serde(rename_all = "camelCase")]
pub struct JavaRuntime {
    /// Path to `java`/`javaw` executable.
    pub path: String,
    pub major: u32,
    /// Full version string, e.g. `21.0.5`.
    pub version: String,
    pub vendor: Option<String>,
    /// `x64`, `x86`, `arm64`.
    pub arch: String,
    /// Installed and updated by the launcher (vs found on the system).
    pub managed: bool,
}
