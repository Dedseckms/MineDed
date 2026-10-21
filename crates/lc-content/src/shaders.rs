//! Shader loader selection. Shader packs need Iris (Fabric/Quilt, NeoForge on
//! newer versions) or Oculus (+ Embeddium/Rubidium) on Forge. OptiFine has no
//! API and restrictive terms, so it is only ever offered as a manual download.

use lc_types::content::ManualDownload;
use lc_types::version::LoaderKind;

pub(crate) const OPTIFINE_URL: &str = "https://optifine.net/downloads";

/// Groups of Modrinth slugs: from each group the first one with a compatible
/// version is installed. The first group is the shader loader itself; when it
/// has no compatible candidate, OptiFine is suggested instead.
pub(crate) fn shader_loader_groups(loader: LoaderKind) -> Vec<Vec<&'static str>> {
    match loader {
        LoaderKind::Fabric | LoaderKind::Quilt => vec![vec!["iris"]],
        LoaderKind::NeoForge => vec![vec!["iris", "oculus"]],
        // Oculus needs a Sodium port; it declares it, but older builds do not.
        LoaderKind::Forge => vec![vec!["oculus"], vec!["embeddium", "rubidium"]],
        LoaderKind::Vanilla => Vec::new(),
    }
}

/// File names that mean a shader loader is already present.
pub(crate) fn has_shader_loader(file_names: &[String]) -> bool {
    file_names.iter().any(|f| {
        let f = f.to_ascii_lowercase();
        f.starts_with("iris") || f.starts_with("oculus") || f.contains("optifine")
    })
}

pub(crate) fn optifine_manual() -> ManualDownload {
    ManualDownload {
        project_id: "optifine".into(),
        file_name: "OptiFine.jar".into(),
        url: OPTIFINE_URL.into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_loader_per_platform() {
        assert_eq!(shader_loader_groups(LoaderKind::Fabric), vec![vec!["iris"]]);
        assert_eq!(shader_loader_groups(LoaderKind::Quilt), vec![vec!["iris"]]);
        assert_eq!(
            shader_loader_groups(LoaderKind::NeoForge),
            vec![vec!["iris", "oculus"]]
        );
        assert_eq!(
            shader_loader_groups(LoaderKind::Forge),
            vec![vec!["oculus"], vec!["embeddium", "rubidium"]]
        );
        assert!(shader_loader_groups(LoaderKind::Vanilla).is_empty());
        assert_eq!(optifine_manual().url, OPTIFINE_URL);
    }

    #[test]
    fn detects_existing_loader() {
        assert!(has_shader_loader(&["iris-1.8.jar".into()]));
        assert!(has_shader_loader(&["OptiFine_1.20.1_HD_U_I6.jar".into()]));
        assert!(!has_shader_loader(&["sodium.jar".into()]));
    }
}
