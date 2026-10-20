use std::collections::BTreeMap;
use std::path::PathBuf;

use lc_types::instance::{Instance, LaunchOpts};
use lc_types::AppResult;

/// Direct-join target passed to the game.
/// 1.20+: `--quickPlayMultiplayer host:port`; older: `--server host --port port`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuickPlay {
    pub host: String,
    pub port: u16,
}

/// Fully resolved command line, built by `lc-core` and then adjusted by
/// contributors (accounts → authlib-injector, net → proxy/P2P target, ...).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LaunchPlan {
    pub java: PathBuf,
    pub working_dir: PathBuf,
    pub main_class: String,
    pub classpath: Vec<PathBuf>,
    pub jvm_args: Vec<String>,
    /// `-javaagent:` entries; rendered before other JVM args.
    pub java_agents: Vec<String>,
    pub game_args: Vec<String>,
    pub env: BTreeMap<String, String>,
    pub quick_play: Option<QuickPlay>,
}

/// Hook that lets other crates modify the launch without `lc-core` knowing
/// about them. Order of contributors is fixed by the lead in `src-tauri`:
/// accounts → mods (perf flags) → vpn → p2p.
pub trait LaunchContributor: Send + Sync {
    /// Stable name for logs.
    fn name(&self) -> &'static str;
    fn contribute(
        &self,
        instance: &Instance,
        opts: &LaunchOpts,
        plan: &mut LaunchPlan,
    ) -> AppResult<()>;
}
