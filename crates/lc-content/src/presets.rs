//! Performance presets: optimization mods per loader/version + JVM flags and
//! heap size picked from the hardware (`SystemProbe`).

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::content::{ContentKind, ContentSourceKind, ContentVersionRef};
use lc_types::instance::{InstancePatch, MemoryCfg, PerfPreset};
use lc_types::system::SystemInfo;
use lc_types::version::LoaderKind;
use lc_types::{AppResult, InstanceId};

use crate::contract::PresetService;
use crate::index::ContentIndex;
use crate::resolver::{resolve, InstalledRef};
use crate::service::{target_of, ContentServices};
use crate::source::Catalog;

/// Modrinth slugs; from each group the first one with a build for the instance's
/// Minecraft version + loader is installed (so the list adapts to the version).
pub(crate) fn preset_mod_groups(loader: LoaderKind, preset: PerfPreset) -> Vec<Vec<&'static str>> {
    let mut g: Vec<Vec<&'static str>> = match loader {
        LoaderKind::Fabric | LoaderKind::Quilt => vec![
            vec!["sodium"],
            vec!["lithium"],
            vec!["ferrite-core"],
            vec!["modernfix"],
            vec!["immediatelyfast"],
            vec!["entityculling"],
        ],
        LoaderKind::NeoForge => vec![
            vec!["sodium", "embeddium"],
            vec!["lithium"],
            vec!["ferrite-core"],
            vec!["modernfix"],
            vec!["immediatelyfast"],
            vec!["entityculling"],
        ],
        LoaderKind::Forge => vec![
            vec!["embeddium", "rubidium"],
            vec!["canary"],
            vec!["ferrite-core"],
            vec!["modernfix"],
            vec!["immediatelyfast"],
            vec!["entityculling"],
        ],
        LoaderKind::Vanilla => Vec::new(),
    };
    if preset == PerfPreset::Quality {
        // Entity culling can cause visible pop-in; quality keeps the vanilla look.
        g.retain(|x| x != &vec!["entityculling"]);
    }
    g
}

/// JVM options this module manages (replaced on every apply).
fn managed(arg: &str) -> bool {
    let Some(opt) = arg.strip_prefix("-XX:") else {
        return false;
    };
    let name = opt
        .trim_start_matches(['+', '-'])
        .split('=')
        .next()
        .unwrap_or_default();
    name.starts_with("G1")
        || name.ends_with("GC")
        || matches!(
            name,
            "ParallelRefProcEnabled"
                | "MaxGCPauseMillis"
                | "UnlockExperimentalVMOptions"
                | "DisableExplicitGC"
                | "AlwaysPreTouch"
                | "InitiatingHeapOccupancyPercent"
                | "SurvivorRatio"
                | "PerfDisableSharedMem"
                | "MaxTenuringThreshold"
                | "ParallelGCThreads"
                | "ConcGCThreads"
        )
}

/// Aikar-style G1 flags tuned by preset, heap and CPU.
pub(crate) fn jvm_flags(preset: PerfPreset, info: &SystemInfo, max_mb: u32) -> Vec<String> {
    let big = max_mb >= 12 * 1024;
    let (new_min, new_max, region, reserve, ihop) = if big {
        (40, 50, 16, 15, 20)
    } else {
        (30, 40, 8, 20, 15)
    };
    let pause = match preset {
        PerfPreset::Potato => 100,
        _ => 200,
    };
    let mut f = vec![
        "-XX:+UseG1GC".to_owned(),
        "-XX:+ParallelRefProcEnabled".to_owned(),
        format!("-XX:MaxGCPauseMillis={pause}"),
        "-XX:+UnlockExperimentalVMOptions".to_owned(),
        "-XX:+DisableExplicitGC".to_owned(),
        format!("-XX:G1NewSizePercent={new_min}"),
        format!("-XX:G1MaxNewSizePercent={new_max}"),
        format!("-XX:G1HeapRegionSize={region}M"),
        format!("-XX:G1ReservePercent={reserve}"),
        "-XX:G1HeapWastePercent=5".to_owned(),
        "-XX:G1MixedGCCountTarget=4".to_owned(),
        format!("-XX:InitiatingHeapOccupancyPercent={ihop}"),
        "-XX:G1MixedGCLiveThresholdPercent=90".to_owned(),
        "-XX:G1RSetUpdatingPauseTimePercent=5".to_owned(),
        "-XX:SurvivorRatio=32".to_owned(),
        "-XX:+PerfDisableSharedMem".to_owned(),
        "-XX:MaxTenuringThreshold=1".to_owned(),
    ];
    // Few cores: keep GC threads from starving the game thread.
    if info.cpu_threads > 0 && info.cpu_threads <= 4 {
        f.push(format!("-XX:ParallelGCThreads={}", info.cpu_threads.max(2)));
        f.push("-XX:ConcGCThreads=1".to_owned());
    }
    // Pre-touching the heap only pays off when RAM is plentiful.
    if preset == PerfPreset::Quality && !info.weak_hardware && info.total_ram_mb >= 16 * 1024 {
        f.push("-XX:+AlwaysPreTouch".to_owned());
    }
    f
}

/// Replaces managed flags in `current`, keeping the user's own arguments.
pub(crate) fn merge_flags(current: &[String], flags: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = current.iter().filter(|a| !managed(a)).cloned().collect();
    out.extend(flags);
    out
}

/// Heap size from total RAM, preset and mod count.
pub(crate) fn memory_for(preset: PerfPreset, info: &SystemInfo, mods: usize) -> MemoryCfg {
    let total = info.total_ram_mb.min(u32::MAX as u64) as u32;
    let reserve = if total <= 4096 {
        1536
    } else if total <= 8192 {
        2560
    } else {
        4096
    };
    let cap = total.saturating_sub(reserve).max(1024);
    let mut want: u32 = match preset {
        PerfPreset::Potato => 2048,
        PerfPreset::Balanced => 4096,
        PerfPreset::Quality => 6144,
    };
    if mods > 200 {
        want += 2048;
    } else if mods > 100 {
        want += 1024;
    }
    let max = (want.min(cap).max(1024) / 256) * 256;
    let min = if info.weak_hardware || preset == PerfPreset::Potato {
        512
    } else {
        (max / 2).min(2048)
    };
    MemoryCfg {
        min_mb: min,
        max_mb: max,
        auto: false,
    }
}

#[async_trait]
impl PresetService for ContentServices {
    async fn apply(
        &self,
        instance: InstanceId,
        preset: PerfPreset,
        sink: &dyn ProgressSink,
    ) -> AppResult<()> {
        let inner = &self.inner;
        let inst = inner.instances.get(instance).await?;
        let target = target_of(&inst);

        // 1. Optimization mods (first compatible candidate of each group).
        let mut extra = Vec::new();
        for group in preset_mod_groups(target.loader, preset) {
            sink.check_cancelled()?;
            for slug in group {
                match inner.modrinth.latest(slug, &target).await {
                    Ok(Some(v)) => {
                        extra.push(ContentVersionRef {
                            source: ContentSourceKind::Modrinth,
                            project_id: v.project_id,
                            version_id: Some(v.version_id),
                        });
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => sink.log(&format!("preset: {slug} unavailable: {}", e.message)),
                }
            }
        }
        if !extra.is_empty() {
            let idx = ContentIndex::load(&inner.instance_dir(instance)).await?;
            let installed: Vec<InstalledRef> = idx
                .files
                .values()
                .map(|e| InstalledRef {
                    source: e.source,
                    project_id: e.project_id.clone(),
                    version_id: Some(e.version_id.clone()),
                })
                .collect();
            let plan = resolve(inner.as_ref(), &target, &installed, Vec::new(), extra).await?;
            for c in &plan.conflicts {
                sink.log(&format!(
                    "preset: {} conflicts with {}: {}",
                    c.project_id, c.with_project_id, c.reason
                ));
            }
            inner
                .install_versions(instance, &plan.install, sink)
                .await?;
        }

        // 2. JVM flags + heap from the hardware.
        let info = inner.system.info().await?;
        let game = inner.instances.game_dir(instance);
        let mods = inner
            .scan(&game)
            .await?
            .iter()
            .filter(|f| f.kind == ContentKind::Mod && f.enabled)
            .count();
        let memory = memory_for(preset, &info, mods);
        let jvm_args = merge_flags(&inst.jvm_args, jvm_flags(preset, &info, memory.max_mb));
        inner
            .instances
            .update(
                instance,
                InstancePatch {
                    memory: Some(memory),
                    jvm_args: Some(jvm_args),
                    ..Default::default()
                },
            )
            .await?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(ram: u64, threads: u32, weak: bool) -> SystemInfo {
        SystemInfo {
            os: "windows".into(),
            os_version: "11".into(),
            arch: "x86_64".into(),
            cpu_model: "cpu".into(),
            cpu_threads: threads,
            total_ram_mb: ram,
            free_ram_mb: ram / 2,
            gpu: None,
            weak_hardware: weak,
            wayland: false,
        }
    }

    #[test]
    fn mod_groups_per_loader() {
        let fabric = preset_mod_groups(LoaderKind::Fabric, PerfPreset::Potato);
        let flat: Vec<&str> = fabric.iter().flatten().copied().collect();
        for m in [
            "sodium",
            "lithium",
            "ferrite-core",
            "modernfix",
            "entityculling",
            "immediatelyfast",
        ] {
            assert!(flat.contains(&m), "{m}");
        }
        assert_eq!(
            preset_mod_groups(LoaderKind::Forge, PerfPreset::Balanced)[0],
            vec!["embeddium", "rubidium"]
        );
        assert_eq!(
            preset_mod_groups(LoaderKind::NeoForge, PerfPreset::Balanced)[0],
            vec!["sodium", "embeddium"]
        );
        assert!(!preset_mod_groups(LoaderKind::Fabric, PerfPreset::Quality)
            .iter()
            .flatten()
            .any(|m| *m == "entityculling"));
        assert!(preset_mod_groups(LoaderKind::Vanilla, PerfPreset::Potato).is_empty());
    }

    #[test]
    fn memory_respects_hardware() {
        let m = memory_for(PerfPreset::Quality, &info(4096, 4, true), 0);
        assert_eq!(m.max_mb, 2560);
        assert_eq!(m.min_mb, 512);
        assert!(!m.auto);
        let m = memory_for(PerfPreset::Balanced, &info(16384, 12, false), 150);
        assert_eq!(m.max_mb, 5120);
        assert_eq!(m.min_mb, 2048);
        let m = memory_for(PerfPreset::Potato, &info(2048, 2, true), 0);
        assert_eq!(m.max_mb, 1024);
    }

    #[test]
    fn flags_replace_only_managed() {
        let cur = vec![
            "-XX:+UseZGC".to_owned(),
            "-Dfoo=bar".to_owned(),
            "-XX:MaxGCPauseMillis=50".to_owned(),
        ];
        let out = merge_flags(
            &cur,
            jvm_flags(PerfPreset::Potato, &info(4096, 4, true), 2048),
        );
        assert!(out.contains(&"-Dfoo=bar".to_owned()));
        assert!(!out.iter().any(|a| a == "-XX:+UseZGC"));
        assert_eq!(
            out.iter()
                .filter(|a| a.starts_with("-XX:MaxGCPauseMillis"))
                .count(),
            1
        );
        assert!(out.contains(&"-XX:ConcGCThreads=1".to_owned()));
        // Idempotent.
        let again = merge_flags(
            &out,
            jvm_flags(PerfPreset::Potato, &info(4096, 4, true), 2048),
        );
        assert_eq!(again, out);
        let big = jvm_flags(PerfPreset::Quality, &info(32768, 16, false), 12288);
        assert!(big.contains(&"-XX:G1HeapRegionSize=16M".to_owned()));
        assert!(big.contains(&"-XX:+AlwaysPreTouch".to_owned()));
    }
}
