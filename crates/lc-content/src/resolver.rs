//! Dependency resolution: walks required dependencies breadth-first, skips what
//! is installed, reports incompatibilities and manual (opt-out) downloads.

use std::collections::{HashSet, VecDeque};

use async_trait::async_trait;
use lc_types::content::{
    Conflict, ContentSourceKind, ContentVersion, ContentVersionRef, DependencyKind, ManualDownload,
    ResolvePlan,
};
use lc_types::{AppResult, ErrorCode};

use crate::source::Target;

/// What the resolver needs from the catalogues.
#[async_trait]
pub(crate) trait Lookup: Send + Sync {
    async fn version(
        &self,
        source: ContentSourceKind,
        version_id: &str,
    ) -> AppResult<ContentVersion>;
    async fn latest(
        &self,
        source: ContentSourceKind,
        project_id: &str,
        target: &Target,
    ) -> AppResult<Option<ContentVersion>>;
    /// Page for a manual download.
    async fn manual_url(&self, v: &ContentVersion) -> AppResult<String>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledRef {
    pub source: ContentSourceKind,
    pub project_id: String,
    pub version_id: Option<String>,
}

struct Item {
    r: ContentVersionRef,
    /// Chosen by the user (may replace an installed version).
    pick: bool,
    /// Who required it (for messages).
    parent: Option<String>,
}

pub(crate) async fn resolve(
    lookup: &dyn Lookup,
    target: &Target,
    installed: &[InstalledRef],
    picks: Vec<ContentVersionRef>,
    // Added by the launcher (shader loader): never replace an installed version.
    extra: Vec<ContentVersionRef>,
) -> AppResult<ResolvePlan> {
    let mut plan = ResolvePlan {
        install: Vec::new(),
        already_installed: Vec::new(),
        conflicts: Vec::new(),
        manual: Vec::new(),
    };
    let mut queue: VecDeque<Item> = picks
        .into_iter()
        .map(|r| Item {
            r,
            pick: true,
            parent: None,
        })
        .chain(extra.into_iter().map(|r| Item {
            r,
            pick: false,
            parent: None,
        }))
        .collect();
    let mut seen: HashSet<(ContentSourceKind, String)> = HashSet::new();
    let mut seen_versions: HashSet<String> = HashSet::new();
    // (project that declares it, incompatible project)
    let mut incompatible: Vec<(ContentSourceKind, String, String)> = Vec::new();

    while let Some(item) = queue.pop_front() {
        let src = item.r.source;
        let fetched = match &item.r.version_id {
            Some(vid) if item.r.project_id.is_empty() || item.pick => {
                if !seen_versions.insert(vid.clone()) {
                    continue;
                }
                lookup.version(src, vid).await.map(Some)
            }
            pinned => {
                if seen.contains(&(src, item.r.project_id.clone())) {
                    continue;
                }
                match lookup.latest(src, &item.r.project_id, target).await {
                    Ok(None) => match pinned {
                        // No compatible build: fall back to the pinned one.
                        Some(vid) => lookup.version(src, vid).await.map(Some),
                        None => Ok(None),
                    },
                    other => other,
                }
            }
        };
        let v = match fetched {
            Ok(Some(v)) => v,
            Ok(None) => {
                plan.conflicts.push(Conflict {
                    project_id: item.r.project_id.clone(),
                    with_project_id: item.parent.clone().unwrap_or_default(),
                    reason: format!(
                        "no version compatible with {} {:?}",
                        target.mc, target.loader
                    ),
                });
                continue;
            }
            Err(e) if item.pick => return Err(e),
            Err(e) => {
                plan.conflicts.push(Conflict {
                    project_id: item.r.project_id.clone(),
                    with_project_id: item.parent.clone().unwrap_or_default(),
                    reason: if e.code == ErrorCode::NotFound {
                        "dependency not found".into()
                    } else {
                        format!("dependency lookup failed: {}", e.message)
                    },
                });
                continue;
            }
        };
        if !seen.insert((src, v.project_id.clone())) {
            continue;
        }
        let inst = installed
            .iter()
            .find(|i| i.source == src && i.project_id == v.project_id);
        if let Some(i) = inst {
            let same = i.version_id.as_deref() == Some(v.version_id.as_str());
            if same || !item.pick {
                plan.already_installed.push(v.project_id.clone());
                continue;
            }
        }
        for d in &v.dependencies {
            match d.kind {
                DependencyKind::Required => queue.push_back(Item {
                    r: ContentVersionRef {
                        source: src,
                        project_id: d.project_id.clone(),
                        version_id: d.version_id.clone(),
                    },
                    pick: false,
                    parent: Some(v.project_id.clone()),
                }),
                DependencyKind::Incompatible if !d.project_id.is_empty() => {
                    incompatible.push((src, v.project_id.clone(), d.project_id.clone()))
                }
                _ => {}
            }
        }
        if v.manual_download {
            plan.manual.push(ManualDownload {
                project_id: v.project_id.clone(),
                file_name: v.file_name.clone(),
                url: lookup.manual_url(&v).await?,
            });
        } else {
            plan.install.push(v);
        }
    }

    for (src, from, other) in incompatible {
        let hit = installed
            .iter()
            .any(|i| i.source == src && i.project_id == other)
            || plan
                .install
                .iter()
                .any(|v| v.source == src && v.project_id == other);
        if hit {
            plan.conflicts.push(Conflict {
                project_id: from,
                with_project_id: other,
                reason: "incompatible".into(),
            });
        }
    }
    Ok(plan)
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;

    use lc_types::content::Dependency;
    use lc_types::version::LoaderKind;
    use lc_types::AppError;

    use super::*;

    /// project → versions (newest first, already compatible).
    #[derive(Default)]
    pub struct FakeLookup {
        pub projects: HashMap<String, Vec<ContentVersion>>,
    }

    pub fn ver(project: &str, version: &str, deps: &[(&str, DependencyKind)]) -> ContentVersion {
        ContentVersion {
            source: ContentSourceKind::Modrinth,
            project_id: project.into(),
            version_id: version.into(),
            name: version.into(),
            version_number: version.into(),
            mc_versions: vec!["1.21.1".into()],
            loaders: vec![LoaderKind::Fabric],
            published: String::new(),
            file_name: format!("{project}-{version}.jar"),
            size: 1,
            sha1: None,
            dependencies: deps
                .iter()
                .map(|(p, k)| Dependency {
                    project_id: (*p).into(),
                    version_id: None,
                    kind: *k,
                })
                .collect(),
            manual_download: false,
        }
    }

    impl FakeLookup {
        pub fn add(&mut self, v: ContentVersion) {
            self.projects
                .entry(v.project_id.clone())
                .or_default()
                .push(v);
        }
    }

    #[async_trait]
    impl Lookup for FakeLookup {
        async fn version(&self, _s: ContentSourceKind, id: &str) -> AppResult<ContentVersion> {
            self.projects
                .values()
                .flatten()
                .find(|v| v.version_id == id)
                .cloned()
                .ok_or_else(|| AppError::new(ErrorCode::NotFound, "nf"))
        }
        async fn latest(
            &self,
            _s: ContentSourceKind,
            p: &str,
            _t: &Target,
        ) -> AppResult<Option<ContentVersion>> {
            match self.projects.get(p) {
                Some(v) => Ok(v.first().cloned()),
                None => Err(AppError::new(ErrorCode::NotFound, "nf")),
            }
        }
        async fn manual_url(&self, v: &ContentVersion) -> AppResult<String> {
            Ok(format!(
                "https://www.curseforge.com/x/download/{}",
                v.version_id
            ))
        }
    }

    fn target() -> Target {
        Target {
            mc: "1.21.1".into(),
            loader: LoaderKind::Fabric,
        }
    }

    fn pick(p: &str) -> ContentVersionRef {
        ContentVersionRef {
            source: ContentSourceKind::Modrinth,
            project_id: p.into(),
            version_id: None,
        }
    }

    #[tokio::test]
    async fn walks_required_and_handles_cycles() {
        let mut l = FakeLookup::default();
        l.add(ver("iris", "i1", &[("sodium", DependencyKind::Required)]));
        l.add(ver(
            "sodium",
            "s1",
            &[
                ("iris", DependencyKind::Required),
                ("opt", DependencyKind::Optional),
            ],
        ));
        l.add(ver("opt", "o1", &[]));
        let plan = resolve(&l, &target(), &[], vec![pick("iris")], vec![])
            .await
            .unwrap();
        let ids: Vec<_> = plan.install.iter().map(|v| v.project_id.as_str()).collect();
        assert_eq!(ids, vec!["iris", "sodium"]);
        assert!(plan.conflicts.is_empty());
    }

    #[tokio::test]
    async fn skips_installed_and_reports_conflicts() {
        let mut l = FakeLookup::default();
        l.add(ver(
            "a",
            "a1",
            &[
                ("lib", DependencyKind::Required),
                ("optifine", DependencyKind::Incompatible),
                ("missing", DependencyKind::Required),
            ],
        ));
        l.add(ver("lib", "l2", &[]));
        let installed = vec![
            InstalledRef {
                source: ContentSourceKind::Modrinth,
                project_id: "lib".into(),
                version_id: Some("l1".into()),
            },
            InstalledRef {
                source: ContentSourceKind::Modrinth,
                project_id: "optifine".into(),
                version_id: None,
            },
        ];
        let plan = resolve(&l, &target(), &installed, vec![pick("a")], vec![])
            .await
            .unwrap();
        assert_eq!(plan.install.len(), 1);
        assert_eq!(plan.already_installed, vec!["lib"]);
        assert_eq!(plan.conflicts.len(), 2);
        assert!(plan
            .conflicts
            .iter()
            .any(|c| c.project_id == "missing" && c.with_project_id == "a"));
        assert!(plan
            .conflicts
            .iter()
            .any(|c| c.project_id == "a" && c.with_project_id == "optifine"));
    }

    #[tokio::test]
    async fn explicit_pick_replaces_installed_version() {
        let mut l = FakeLookup::default();
        l.add(ver("a", "a2", &[]));
        let installed = vec![InstalledRef {
            source: ContentSourceKind::Modrinth,
            project_id: "a".into(),
            version_id: Some("a1".into()),
        }];
        let plan = resolve(&l, &target(), &installed, vec![pick("a")], vec![])
            .await
            .unwrap();
        assert_eq!(plan.install.len(), 1);
        let plan = resolve(
            &l,
            &target(),
            &[InstalledRef {
                version_id: Some("a2".into()),
                ..installed[0].clone()
            }],
            vec![pick("a")],
            vec![],
        )
        .await
        .unwrap();
        assert!(plan.install.is_empty());
        assert_eq!(plan.already_installed, vec!["a"]);
    }

    #[tokio::test]
    async fn manual_downloads_are_not_installed() {
        let mut l = FakeLookup::default();
        let mut v = ver("cf", "77", &[("dep", DependencyKind::Required)]);
        v.manual_download = true;
        l.add(v);
        l.add(ver("dep", "d1", &[]));
        let plan = resolve(&l, &target(), &[], vec![pick("cf")], vec![])
            .await
            .unwrap();
        assert_eq!(plan.manual.len(), 1);
        assert_eq!(
            plan.manual[0].url,
            "https://www.curseforge.com/x/download/77"
        );
        // Its dependencies are still installed.
        assert_eq!(plan.install.len(), 1);
        assert_eq!(plan.install[0].project_id, "dep");
    }
}
