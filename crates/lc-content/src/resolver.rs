//! Dependency resolution: walks required dependencies breadth-first, skips what
//! is installed, honours version pins of dependencies, reports
//! incompatibilities and manual (opt-out) downloads.
//!
//! On Fabric/Quilt the chosen set is then checked against the `depends` /
//! `breaks` ranges of each jar's `fabric.mod.json`: a dependency picked as
//! "latest" is swapped for an older compatible build when one exists;
//! otherwise the clash is reported as a conflict and the new mod that would
//! break the game is left out of `install`.

use std::collections::{BTreeSet, HashMap, HashSet, VecDeque};

use async_trait::async_trait;
use lc_types::content::{
    Conflict, ContentSourceKind, ContentVersion, ContentVersionRef, DependencyKind, ManualDownload,
    ResolvePlan,
};
use lc_types::version::LoaderKind;
use lc_types::{AppResult, ErrorCode};

use crate::modmeta::{satisfies, ModMeta};
use crate::source::Target;
use crate::util::loader_compatible;

/// Older builds tried per project when looking for a compatible one.
const MAX_CANDIDATES: usize = 12;
/// Mod ids that are not mods (provided by the loader / game).
const BUILTIN_IDS: [&str; 5] = [
    "minecraft",
    "java",
    "fabricloader",
    "fabric-loader",
    "quilt_loader",
];

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

    /// Versions compatible with `target`, preferred first (releases, newest).
    async fn candidates(
        &self,
        _source: ContentSourceKind,
        _project_id: &str,
        _target: &Target,
    ) -> AppResult<Vec<ContentVersion>> {
        Ok(Vec::new())
    }

    /// `fabric.mod.json` of the version's jar (may download it into a cache);
    /// `None` when unknown. Best effort, never fails the resolution.
    async fn mod_meta(&self, _v: &ContentVersion) -> Option<ModMeta> {
        None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstalledRef {
    pub source: ContentSourceKind,
    pub project_id: String,
    pub version_id: Option<String>,
}

/// Metadata of an enabled mod jar already in the instance.
#[derive(Debug, Clone)]
pub(crate) struct DiskMeta {
    /// Catalogue project, when the file is known to the content index.
    pub owner: Option<(ContentSourceKind, String)>,
    pub meta: ModMeta,
}

struct Item {
    r: ContentVersionRef,
    /// Chosen by the user (may replace an installed version).
    pick: bool,
    /// Who required it (for messages).
    parent: Option<String>,
}

/// How a project's version was chosen; decides who may override it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum How {
    /// The user picked this exact version.
    Explicit,
    /// A dependent pinned this exact version.
    Pinned,
    /// Newest compatible build: replaceable by a pin or a compatible build.
    Latest,
    /// Already in the instance and kept.
    Installed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Install(usize),
    Manual,
    None,
}

struct Chosen {
    how: How,
    version_id: String,
    slot: Slot,
}

type Key = (ContentSourceKind, String);

fn fits(v: &ContentVersion, target: &Target) -> bool {
    let mc_ok = v.mc_versions.is_empty() || v.mc_versions.contains(&target.mc);
    let loader_ok = v.loaders.is_empty()
        || target.loader == LoaderKind::Vanilla
        || loader_compatible(&v.loaders, target.loader);
    mc_ok && loader_ok
}

fn push_deps(
    queue: &mut VecDeque<Item>,
    incompatible: &mut Vec<(Key, String)>,
    v: &ContentVersion,
) {
    for d in &v.dependencies {
        match d.kind {
            DependencyKind::Required => queue.push_back(Item {
                r: ContentVersionRef {
                    source: v.source,
                    project_id: d.project_id.clone(),
                    version_id: d.version_id.clone(),
                },
                pick: false,
                parent: Some(v.project_id.clone()),
            }),
            DependencyKind::Incompatible if !d.project_id.is_empty() => {
                incompatible.push(((v.source, v.project_id.clone()), d.project_id.clone()))
            }
            _ => {}
        }
    }
}

pub(crate) async fn resolve(
    lookup: &dyn Lookup,
    target: &Target,
    installed: &[InstalledRef],
    disk: &[DiskMeta],
    picks: Vec<ContentVersionRef>,
    // Added by the launcher (shader loader, presets): never replace an installed version.
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
    let mut chosen: HashMap<Key, Chosen> = HashMap::new();
    let mut seen_versions: HashSet<String> = HashSet::new();
    // (project that declares it, incompatible project)
    let mut incompatible: Vec<(Key, String)> = Vec::new();

    while let Some(item) = queue.pop_front() {
        let src = item.r.source;
        let pid = item.r.project_id.clone();
        let known = !pid.is_empty();
        let prev = chosen.get(&(src, pid.clone()));

        let (how, fetched) = match &item.r.version_id {
            Some(vid) => {
                if prev.is_some_and(|c| c.version_id == *vid || c.how == How::Installed) {
                    continue;
                }
                if !known && !seen_versions.insert(vid.clone()) {
                    continue;
                }
                let how = if item.pick {
                    How::Explicit
                } else {
                    How::Pinned
                };
                match lookup.version(src, vid).await {
                    // A user pick is taken as is; a pin only if it runs here.
                    Ok(v) if item.pick || fits(&v, target) => (how, Ok(Some(v))),
                    pinned if known && !item.pick => {
                        if prev.is_some() {
                            continue;
                        }
                        // Unusable pin: newest compatible build, else the pin itself.
                        let r = match lookup.latest(src, &pid, target).await {
                            Ok(None) => pinned.map(Some),
                            other => other,
                        };
                        (How::Latest, r)
                    }
                    other => (how, other.map(Some)),
                }
            }
            None => {
                if prev.is_some() {
                    continue;
                }
                (How::Latest, lookup.latest(src, &pid, target).await)
            }
        };
        let v = match fetched {
            Ok(Some(v)) => v,
            Ok(None) => {
                plan.conflicts.push(Conflict {
                    project_id: pid,
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
                    project_id: pid,
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
        let key: Key = (src, v.project_id.clone());

        if let Some(c) = chosen.get_mut(&key) {
            if c.version_id == v.version_id || c.how == How::Installed || how == How::Latest {
                continue;
            }
            match (c.how, c.slot) {
                // A pin (or exact pick) beats "latest".
                (How::Latest, Slot::Install(i)) if !v.manual_download => {
                    c.how = how;
                    c.version_id = v.version_id.clone();
                    push_deps(&mut queue, &mut incompatible, &v);
                    plan.install[i] = v;
                }
                _ => plan.conflicts.push(Conflict {
                    project_id: v.project_id.clone(),
                    with_project_id: item.parent.clone().unwrap_or_default(),
                    reason: format!(
                        "requires version {} but {} is selected",
                        v.version_number, c.version_id
                    ),
                }),
            }
            continue;
        }

        let inst = installed
            .iter()
            .find(|i| i.source == src && i.project_id == v.project_id);
        if let Some(i) = inst {
            let same = i.version_id.as_deref() == Some(v.version_id.as_str());
            if same || !item.pick {
                plan.already_installed.push(v.project_id.clone());
                chosen.insert(
                    key,
                    Chosen {
                        how: How::Installed,
                        version_id: i.version_id.clone().unwrap_or_default(),
                        slot: Slot::None,
                    },
                );
                continue;
            }
        }
        push_deps(&mut queue, &mut incompatible, &v);
        let slot = if v.manual_download {
            plan.manual.push(ManualDownload {
                project_id: v.project_id.clone(),
                file_name: v.file_name.clone(),
                url: lookup.manual_url(&v).await?,
            });
            Slot::Manual
        } else {
            plan.install.push(v.clone());
            Slot::Install(plan.install.len() - 1)
        };
        chosen.insert(
            key,
            Chosen {
                how,
                version_id: v.version_id,
                slot,
            },
        );
    }

    if matches!(target.loader, LoaderKind::Fabric | LoaderKind::Quilt) {
        check_mod_meta(lookup, target, &mut plan, &chosen, disk).await;
    }

    for ((src, from), other) in incompatible {
        let hit = installed
            .iter()
            .any(|i| i.source == src && i.project_id == other)
            || plan
                .install
                .iter()
                .any(|v| v.source == src && v.project_id == other);
        let from_live = plan
            .install
            .iter()
            .any(|v| v.source == src && v.project_id == from)
            || plan.manual.iter().any(|m| m.project_id == from);
        if hit && from_live {
            plan.conflicts.push(Conflict {
                project_id: from,
                with_project_id: other,
                reason: "incompatible".into(),
            });
        }
    }
    Ok(plan)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Who {
    Plan(usize),
    Disk(usize),
}

struct Violation {
    dependent: Who,
    offender: Who,
    reason: String,
}

/// Every `depends` / `breaks` clash between known mods of the final set.
fn violations(metas: &[Option<ModMeta>], disk: &[&DiskMeta]) -> Vec<Violation> {
    let all: Vec<(Who, &ModMeta)> = disk
        .iter()
        .enumerate()
        .map(|(i, d)| (Who::Disk(i), &d.meta))
        .chain(
            metas
                .iter()
                .enumerate()
                .filter_map(|(i, m)| m.as_ref().map(|m| (Who::Plan(i), m))),
        )
        .collect();
    let mut providers: HashMap<&str, (Who, &str)> = HashMap::new();
    for (who, m) in &all {
        providers.insert(m.id.as_str(), (*who, m.version.as_str()));
        for p in &m.provides {
            providers.insert(p.as_str(), (*who, m.version.as_str()));
        }
    }
    let mut out = Vec::new();
    for (who, m) in &all {
        for (dep, preds) in &m.depends {
            // The game/loader versions are already matched by the catalogue.
            if BUILTIN_IDS.contains(&dep.as_str()) {
                continue;
            }
            if let Some((ow, ver)) = providers.get(dep.as_str()) {
                if ow != who && satisfies(preds, ver) == Some(false) {
                    out.push(Violation {
                        dependent: *who,
                        offender: *ow,
                        reason: format!(
                            "{} {} requires {dep} {}, found {ver}",
                            m.id,
                            m.version,
                            preds.join(" or ")
                        ),
                    });
                }
            }
        }
        for (dep, preds) in &m.breaks {
            if BUILTIN_IDS.contains(&dep.as_str()) {
                continue;
            }
            if let Some((ow, ver)) = providers.get(dep.as_str()) {
                if ow != who && satisfies(preds, ver) == Some(true) {
                    out.push(Violation {
                        dependent: *who,
                        offender: *ow,
                        reason: format!("{} {} is incompatible with {dep} {ver}", m.id, m.version),
                    });
                }
            }
        }
    }
    out
}

fn involves(v: &Violation, i: usize) -> bool {
    v.dependent == Who::Plan(i) || v.offender == Who::Plan(i)
}

async fn check_mod_meta(
    lookup: &dyn Lookup,
    target: &Target,
    plan: &mut ResolvePlan,
    chosen: &HashMap<Key, Chosen>,
    disk: &[DiskMeta],
) {
    let mut metas = Vec::with_capacity(plan.install.len());
    for v in &plan.install {
        metas.push(lookup.mod_meta(v).await);
    }
    // Disk jars of projects being replaced do not count.
    let replaced: HashSet<Key> = plan
        .install
        .iter()
        .map(|v| (v.source, v.project_id.clone()))
        .collect();
    let disk: Vec<&DiskMeta> = disk
        .iter()
        .filter(|d| d.owner.as_ref().is_none_or(|o| !replaced.contains(o)))
        .collect();
    let replaceable = |v: &ContentVersion| {
        chosen
            .get(&(v.source, v.project_id.clone()))
            .is_some_and(|c| c.how == How::Latest)
    };

    // Swap "latest" picks for compatible builds: the offender first, then the dependent.
    let mut tried: HashSet<usize> = HashSet::new();
    loop {
        let found = violations(&metas, &disk);
        let next = found.iter().find_map(|x| {
            [x.offender, x.dependent].into_iter().find_map(|w| match w {
                Who::Plan(i) if !tried.contains(&i) && replaceable(&plan.install[i]) => Some(i),
                _ => None,
            })
        });
        let Some(i) = next else { break };
        tried.insert(i);
        let cur = plan.install[i].clone();
        let cands = lookup
            .candidates(cur.source, &cur.project_id, target)
            .await
            .unwrap_or_default();
        for c in cands
            .into_iter()
            .filter(|c| c.version_id != cur.version_id && !c.manual_download)
            .take(MAX_CANDIDATES)
        {
            let Some(m) = lookup.mod_meta(&c).await else {
                continue;
            };
            let mut trial = metas.clone();
            trial[i] = Some(m);
            if !violations(&trial, &disk).iter().any(|x| involves(x, i)) {
                metas = trial;
                plan.install[i] = c;
                break;
            }
        }
    }

    // What is left: report, and keep the new mod that would break the game out.
    let name = |w: Who, install: &[ContentVersion]| match w {
        Who::Plan(i) => install[i].project_id.clone(),
        Who::Disk(i) => disk[i]
            .owner
            .as_ref()
            .map(|o| o.1.clone())
            .unwrap_or_else(|| disk[i].meta.id.clone()),
    };
    let explicit = |i: usize| {
        let v = &plan.install[i];
        chosen
            .get(&(v.source, v.project_id.clone()))
            .is_some_and(|c| c.how == How::Explicit)
    };
    let mut drop = BTreeSet::new();
    for x in violations(&metas, &disk) {
        plan.conflicts.push(Conflict {
            project_id: name(x.dependent, &plan.install),
            with_project_id: name(x.offender, &plan.install),
            reason: x.reason.clone(),
        });
        let in_plan: Vec<usize> = [x.dependent, x.offender]
            .into_iter()
            .filter_map(|w| match w {
                Who::Plan(i) => Some(i),
                Who::Disk(_) => None,
            })
            .collect();
        if in_plan.iter().any(|i| drop.contains(i)) {
            continue;
        }
        // Prefer leaving out what the user did not pick explicitly; tie → dependent.
        if let Some(i) = in_plan
            .iter()
            .copied()
            .find(|i| !explicit(*i))
            .or_else(|| in_plan.first().copied())
        {
            drop.insert(i);
        }
    }
    for i in drop.into_iter().rev() {
        plan.install.remove(i);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use std::collections::HashMap;

    use lc_types::content::Dependency;
    use lc_types::version::LoaderKind;
    use lc_types::AppError;

    use super::*;
    use crate::modmeta::parse_fabric_mod_json;

    /// project → versions (newest first, already compatible).
    #[derive(Default)]
    pub struct FakeLookup {
        pub projects: HashMap<String, Vec<ContentVersion>>,
        /// version id → `fabric.mod.json`.
        pub metas: HashMap<String, ModMeta>,
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
        async fn candidates(
            &self,
            _s: ContentSourceKind,
            p: &str,
            _t: &Target,
        ) -> AppResult<Vec<ContentVersion>> {
            Ok(self.projects.get(p).cloned().unwrap_or_default())
        }
        async fn mod_meta(&self, v: &ContentVersion) -> Option<ModMeta> {
            self.metas.get(&v.version_id).cloned()
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
        let plan = resolve(&l, &target(), &[], &[], vec![pick("iris")], vec![])
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
        let plan = resolve(&l, &target(), &installed, &[], vec![pick("a")], vec![])
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
        let plan = resolve(&l, &target(), &installed, &[], vec![pick("a")], vec![])
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
            &[],
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
        let plan = resolve(&l, &target(), &[], &[], vec![pick("cf")], vec![])
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

    // --- Iris 1.8.8 + Sodium on 1.21.1 (qa: "Incompatible mods found") ---

    const IRIS: &[u8] = include_bytes!("../tests/fixtures/fabric/iris-1.8.8.fabric.mod.json");
    const SODIUM6: &[u8] = include_bytes!("../tests/fixtures/fabric/sodium-0.6.13.fabric.mod.json");
    const SODIUM8: &[u8] = include_bytes!("../tests/fixtures/fabric/sodium-0.8.13.fabric.mod.json");

    /// Sodium 0.8.13 (newest) and 0.6.13; Iris 1.8.8 requiring Sodium,
    /// optionally pinned to 0.6.13.
    fn iris_sodium(pin: bool, metas: bool) -> FakeLookup {
        let mut l = FakeLookup::default();
        l.add(ver("sodium", "s0813", &[]));
        l.add(ver("sodium", "s0613", &[]));
        let mut iris = ver("iris", "i188", &[("sodium", DependencyKind::Required)]);
        if pin {
            iris.dependencies[0].version_id = Some("s0613".into());
        }
        l.add(iris);
        if metas {
            for (id, json) in [("i188", IRIS), ("s0613", SODIUM6), ("s0813", SODIUM8)] {
                l.metas
                    .insert(id.into(), parse_fabric_mod_json(json).unwrap());
            }
        }
        l
    }

    fn ids(plan: &ResolvePlan) -> Vec<(String, String)> {
        let mut v: Vec<_> = plan
            .install
            .iter()
            .map(|v| (v.project_id.clone(), v.version_id.clone()))
            .collect();
        v.sort();
        v
    }

    #[tokio::test]
    async fn dependency_pin_beats_latest_and_user_pick() {
        let l = iris_sodium(true, false);
        // The user picks Sodium (no version) before Iris: the pin still wins.
        let plan = resolve(
            &l,
            &target(),
            &[],
            &[],
            vec![pick("sodium"), pick("iris")],
            vec![],
        )
        .await
        .unwrap();
        assert_eq!(
            ids(&plan),
            vec![
                ("iris".into(), "i188".into()),
                ("sodium".into(), "s0613".into())
            ]
        );
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
        // Pin alone.
        let plan = resolve(&l, &target(), &[], &[], vec![pick("iris")], vec![])
            .await
            .unwrap();
        assert!(plan.install.iter().any(|v| v.version_id == "s0613"));
    }

    #[tokio::test]
    async fn fabric_ranges_select_compatible_dependency() {
        let l = iris_sodium(false, true);
        let plan = resolve(&l, &target(), &[], &[], vec![pick("iris")], vec![])
            .await
            .unwrap();
        assert_eq!(
            ids(&plan),
            vec![
                ("iris".into(), "i188".into()),
                ("sodium".into(), "s0613".into())
            ]
        );
        assert!(plan.conflicts.is_empty(), "{:?}", plan.conflicts);
    }

    #[tokio::test]
    async fn explicit_incompatible_version_is_reported_not_installed() {
        let l = iris_sodium(false, true);
        let exact = ContentVersionRef {
            version_id: Some("s0813".into()),
            ..pick("sodium")
        };
        let plan = resolve(&l, &target(), &[], &[], vec![exact, pick("iris")], vec![])
            .await
            .unwrap();
        assert_eq!(ids(&plan), vec![("sodium".into(), "s0813".into())]);
        assert!(plan
            .conflicts
            .iter()
            .any(|c| c.project_id == "iris" && c.with_project_id == "sodium"));
    }

    #[tokio::test]
    async fn installed_incompatible_dependency_blocks_new_mod() {
        let l = iris_sodium(false, true);
        let installed = vec![InstalledRef {
            source: ContentSourceKind::Modrinth,
            project_id: "sodium".into(),
            version_id: Some("s0813".into()),
        }];
        let disk = vec![DiskMeta {
            owner: Some((ContentSourceKind::Modrinth, "sodium".into())),
            meta: parse_fabric_mod_json(SODIUM8).unwrap(),
        }];
        let plan = resolve(&l, &target(), &installed, &disk, vec![pick("iris")], vec![])
            .await
            .unwrap();
        assert!(plan.install.is_empty(), "{:?}", ids(&plan));
        assert!(plan
            .conflicts
            .iter()
            .any(|c| c.with_project_id == "sodium" && c.reason.contains("sodium")));
    }

    #[tokio::test]
    async fn unusable_pin_falls_back_to_latest() {
        let mut l = FakeLookup::default();
        let mut old = ver("lib", "l-old", &[]);
        old.mc_versions = vec!["1.20.1".into()];
        l.add(ver("lib", "l-new", &[]));
        l.add(old);
        let mut a = ver("a", "a1", &[("lib", DependencyKind::Required)]);
        a.dependencies[0].version_id = Some("l-old".into());
        l.add(a);
        let plan = resolve(&l, &target(), &[], &[], vec![pick("a")], vec![])
            .await
            .unwrap();
        assert!(plan.install.iter().any(|v| v.version_id == "l-new"));
    }
}
