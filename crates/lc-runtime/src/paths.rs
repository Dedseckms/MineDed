use std::path::{Path, PathBuf};

use lc_types::InstanceId;

/// On-disk layout. Every crate builds paths through this, never by hand.
///
/// ```text
/// <root>/
///   instances/<id>/           instance.json + .minecraft-like game dir
///   libraries/                shared between instances
///   assets/{indexes,objects}/ shared between instances
///   versions/<profile>/       version json (+ client jar)
///   java/<component>/         managed runtimes
///   cache/                    http cache, mirrors health, skins
///   logs/                     launcher logs
///   sidecars/mihomo/          mihomo config + state
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    root: PathBuf,
}

impl Paths {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    pub fn instances(&self) -> PathBuf {
        self.root.join("instances")
    }

    pub fn instance(&self, id: InstanceId) -> PathBuf {
        self.instances().join(id.to_string())
    }

    pub fn libraries(&self) -> PathBuf {
        self.root.join("libraries")
    }

    pub fn assets(&self) -> PathBuf {
        self.root.join("assets")
    }

    pub fn versions(&self) -> PathBuf {
        self.root.join("versions")
    }

    pub fn java(&self) -> PathBuf {
        self.root.join("java")
    }

    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }

    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }

    pub fn mihomo(&self) -> PathBuf {
        self.root.join("sidecars").join("mihomo")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instance_dir_is_under_instances() {
        let p = Paths::new("/data");
        let id = InstanceId::new();
        assert!(p.instance(id).starts_with(p.instances()));
    }
}
