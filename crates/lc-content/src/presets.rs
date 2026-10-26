//! Performance presets: optimization mods per loader/version + JVM flags and
//! heap size picked from the hardware (`SystemProbe`).

use std::collections::HashMap;
use std::path::Path;

use async_trait::async_trait;
use lc_runtime::ProgressSink;
use lc_types::content::{ContentKind, ContentSourceKind, ContentVersionRef};
use lc_types::instance::{InstancePatch, MemoryCfg, PerfPreset};
use lc_types::system::SystemInfo;
use lc_types::version::LoaderKind;
use lc_types::{AppError, AppResult, ErrorCode, InstanceId};

use crate::contract::PresetService;
use crate::index::ContentIndex;
use crate::resolver::{resolve, InstalledRef};
use crate::service::{target_of, ContentServices};
use crate::source::Catalog;
use crate::util::io_err;

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

/// Which `options.txt` keys exist in a given Minecraft version.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct OptionKeys {
    /// `graphicsMode:<0|1|2>` (1.16+) instead of `fancyGraphics:<bool>`.
    pub graphics_mode: bool,
    /// `simulationDistance` (1.18+).
    pub simulation_distance: bool,
    /// `biomeBlendRadius` (1.13+).
    pub biome_blend: bool,
    /// `entityDistanceScaling` (1.16+).
    pub entity_scaling: bool,
    /// `renderClouds:"fast"` is JSON-quoted since 1.13.
    pub quoted_clouds: bool,
}

/// `1.x[.y]` → `x`; snapshots and unknown formats → `None` (treated as newest).
fn mc_minor(mc: &str) -> Option<u32> {
    let base = mc.trim().split(['-', ' ', '+']).next().unwrap_or_default();
    let mut parts = base.split('.');
    if parts.next()? != "1" {
        return None;
    }
    parts.next()?.parse().ok()
}

pub(crate) fn option_keys(mc: &str) -> OptionKeys {
    let m = mc_minor(mc).unwrap_or(u32::MAX);
    OptionKeys {
        graphics_mode: m >= 16,
        simulation_distance: m >= 18,
        biome_blend: m >= 13,
        entity_scaling: m >= 16,
        quoted_clouds: m >= 13,
    }
}

/// How a preset sets one `options.txt` key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum OptVal {
    /// Overwrite with this raw value.
    Set(String),
    /// Raise to at least this number; a higher user value is kept.
    AtLeast(i64),
}

fn set(v: impl ToString) -> OptVal {
    OptVal::Set(v.to_string())
}

/// Graphics settings a preset writes into `options.txt` for version `mc`.
pub(crate) fn preset_options(preset: PerfPreset, mc: &str) -> Vec<(&'static str, OptVal)> {
    let k = option_keys(mc);
    let clouds = |v: &str| {
        if k.quoted_clouds {
            set(format!("\"{v}\""))
        } else {
            set(v)
        }
    };
    let mut o: Vec<(&'static str, OptVal)> = Vec::new();
    let fancy = preset == PerfPreset::Quality;
    if k.graphics_mode {
        o.push((
            "graphicsMode",
            if fancy { OptVal::AtLeast(1) } else { set(0) },
        ));
    } else {
        o.push(("fancyGraphics", set(fancy)));
    }
    match preset {
        PerfPreset::Potato => {
            o.push(("renderDistance", set(6)));
            if k.simulation_distance {
                o.push(("simulationDistance", set(5)));
            }
            o.push(("particles", set(2)));
            o.push(("entityShadows", set(false)));
            o.push(("renderClouds", clouds("false")));
            o.push(("mipmapLevels", set(0)));
            if k.biome_blend {
                o.push(("biomeBlendRadius", set(0)));
            }
            o.push(("maxFps", set(60)));
            o.push(("enableVsync", set(false)));
            if k.entity_scaling {
                o.push(("entityDistanceScaling", set(0.5)));
            }
        }
        PerfPreset::Balanced => {
            o.push(("renderDistance", set(10)));
            if k.simulation_distance {
                o.push(("simulationDistance", set(8)));
            }
            o.push(("particles", set(1)));
            o.push(("renderClouds", clouds("fast")));
            if k.biome_blend {
                o.push(("biomeBlendRadius", set(1)));
            }
            o.push(("mipmapLevels", set(2)));
        }
        PerfPreset::Quality => {
            o.push(("renderDistance", OptVal::AtLeast(12)));
        }
    }
    o
}

/// Rewrites `key:value` lines of `options.txt` in place, appends missing keys and
/// keeps every other line (and the file's line ending) untouched.
pub(crate) fn edit_options(text: &str, opts: &[(&str, OptVal)]) -> String {
    let nl = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let mut seen = vec![false; opts.len()];
    let mut out: Vec<String> = Vec::new();
    for line in text.lines() {
        let found = line.split_once(':').and_then(|(key, cur)| {
            opts.iter()
                .position(|(k, _)| *k == key)
                .map(|i| (i, key, cur))
        });
        match found {
            Some((i, key, cur)) => {
                seen[i] = true;
                let value = match &opts[i].1 {
                    OptVal::Set(v) => v.clone(),
                    OptVal::AtLeast(min) => match cur.trim().parse::<i64>() {
                        Ok(c) if c > *min => c.to_string(),
                        _ => min.to_string(),
                    },
                };
                out.push(format!("{key}:{value}"));
            }
            None => out.push(line.to_owned()),
        }
    }
    for (i, (key, v)) in opts.iter().enumerate() {
        if !seen[i] {
            let value = match v {
                OptVal::Set(v) => v.clone(),
                OptVal::AtLeast(min) => min.to_string(),
            };
            out.push(format!("{key}:{value}"));
        }
    }
    let mut s = out.join(nl);
    s.push_str(nl);
    s
}

/// Applies the preset's graphics settings to `<game>/options.txt`.
async fn write_options(game: &Path, preset: PerfPreset, mc: &str) -> AppResult<()> {
    let path = game.join("options.txt");
    let text = match tokio::fs::read_to_string(&path).await {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(io_err(&path, e)),
    };
    let new = edit_options(&text, &preset_options(preset, mc));
    tokio::fs::create_dir_all(game)
        .await
        .map_err(|e| io_err(game, e))?;
    tokio::fs::write(&path, new)
        .await
        .map_err(|e| io_err(&path, e))
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
        let game = inner.instances.game_dir(instance);

        // 1. Optimization mods (first compatible candidate of each group).
        let groups = preset_mod_groups(target.loader, preset);
        let mut extra = Vec::new();
        let mut slugs: HashMap<String, &'static str> = HashMap::new();
        let mut failures: Vec<String> = Vec::new();
        for group in &groups {
            sink.check_cancelled()?;
            let mut picked = false;
            for slug in group {
                match inner.modrinth.latest(slug, &target).await {
                    Ok(Some(v)) => {
                        slugs.insert(v.project_id.clone(), slug);
                        extra.push(ContentVersionRef {
                            source: ContentSourceKind::Modrinth,
                            project_id: v.project_id,
                            version_id: Some(v.version_id),
                        });
                        picked = true;
                        break;
                    }
                    Ok(None) => {}
                    Err(e) => {
                        sink.log(&format!("preset: {slug} unavailable: {}", e.message));
                        failures.push(format!("{slug}: {}", e.message));
                    }
                }
            }
            if !picked {
                sink.log(&format!(
                    "preset: no build of {} for {} {:?}",
                    group.join("/"),
                    target.mc,
                    target.loader
                ));
            }
        }
        let mut installed: Vec<&'static str> = Vec::new();
        if !extra.is_empty() {
            let idx = ContentIndex::load(&inner.instance_dir(instance)).await?;
            let have: Vec<InstalledRef> = idx
                .files
                .values()
                .map(|e| InstalledRef {
                    source: e.source,
                    project_id: e.project_id.clone(),
                    version_id: Some(e.version_id.clone()),
                })
                .collect();
            let disk = inner.disk_metas(&inst, &game, &idx).await?;
            let plan = resolve(inner.as_ref(), &target, &have, &disk, Vec::new(), extra).await?;
            for c in &plan.conflicts {
                sink.log(&format!(
                    "preset: {} conflicts with {}: {}",
                    c.project_id, c.with_project_id, c.reason
                ));
            }
            inner
                .install_versions(instance, &plan.install, sink)
                .await?;
            let present = plan
                .install
                .iter()
                .filter(|v| !v.manual_download)
                .map(|v| &v.project_id)
                .chain(plan.already_installed.iter());
            for pid in present {
                if let Some(slug) = slugs.get(pid) {
                    if !installed.contains(slug) {
                        installed.push(slug);
                    }
                }
            }
        }

        // 2. Graphics settings (every loader, including vanilla).
        sink.check_cancelled()?;
        write_options(&game, preset, &inst.mc).await?;

        // 3. JVM flags + heap from the hardware.
        let info = inner.system.info().await?;
        let mods = inner
            .scan(&game)
            .await?
            .iter()
            .filter(|f| f.kind == ContentKind::Mod && f.enabled)
            .count();
        let memory = memory_for(preset, &info, mods);
        let max_mb = memory.max_mb;
        let jvm_args = merge_flags(&inst.jvm_args, jvm_flags(preset, &info, max_mb));

        // A modded instance without a single optimization mod means the preset
        // did not really apply: say so instead of reporting success.
        let mods_failed = !groups.is_empty() && installed.is_empty();
        inner
            .instances
            .update(
                instance,
                InstancePatch {
                    memory: Some(memory),
                    jvm_args: Some(jvm_args),
                    preset: (!mods_failed).then_some(preset),
                    ..Default::default()
                },
            )
            .await?;
        let installed_txt = if installed.is_empty() {
            "none".to_owned()
        } else {
            installed.join(", ")
        };
        sink.log(&format!(
            "preset {preset:?}: installed: {installed_txt}; memory: {max_mb} MB; options.txt updated"
        ));
        if mods_failed {
            let reason = if failures.is_empty() {
                format!(
                    "no optimization mod has a build for {} {:?}",
                    target.mc, target.loader
                )
            } else {
                failures.join("; ")
            };
            sink.log(&format!("preset: no optimization mods installed: {reason}"));
            return Err(AppError::new(
                ErrorCode::NotFound,
                "no optimization mods could be installed",
            )
            .detail("mc", &target.mc)
            .detail("loader", format!("{:?}", target.loader))
            .detail("reason", reason));
        }
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
    fn option_keys_by_version() {
        let old = option_keys("1.12.2");
        assert!(!old.graphics_mode && !old.simulation_distance && !old.quoted_clouds);
        let mid = option_keys("1.16.5");
        assert!(mid.graphics_mode && !mid.simulation_distance && mid.biome_blend);
        assert!(option_keys("1.18").simulation_distance);
        assert!(option_keys("1.21.1").simulation_distance);
        assert!(option_keys("1.19-pre1").graphics_mode);
        // Snapshots: newest key set.
        assert!(option_keys("24w14a").simulation_distance);

        let o = preset_options(PerfPreset::Potato, "1.12.2");
        assert!(o.contains(&("fancyGraphics", OptVal::Set("false".into()))));
        assert!(o.contains(&("renderClouds", OptVal::Set("false".into()))));
        assert!(!o
            .iter()
            .any(|(k, _)| *k == "graphicsMode" || *k == "simulationDistance"));
        let o = preset_options(PerfPreset::Potato, "1.17.1");
        assert!(o.contains(&("graphicsMode", OptVal::Set("0".into()))));
        assert!(!o.iter().any(|(k, _)| *k == "simulationDistance"));
        let o = preset_options(PerfPreset::Balanced, "1.21.1");
        assert!(o.contains(&("simulationDistance", OptVal::Set("8".into()))));
        assert!(o.contains(&("renderClouds", OptVal::Set("\"fast\"".into()))));
        let o = preset_options(PerfPreset::Quality, "1.21.1");
        assert_eq!(
            o,
            vec![
                ("graphicsMode", OptVal::AtLeast(1)),
                ("renderDistance", OptVal::AtLeast(12)),
            ]
        );
    }

    #[test]
    fn options_edit_in_place() {
        let text = "version:3955\r\nrenderDistance:16\r\nlang:ru_ru\r\ngraphicsMode:2\r\nkey_key.jump:key.keyboard.space\r\n";
        let out = edit_options(text, &preset_options(PerfPreset::Potato, "1.21.1"));
        assert!(
            out.starts_with("version:3955\r\nrenderDistance:6\r\nlang:ru_ru\r\ngraphicsMode:0\r\n")
        );
        assert!(out.contains("key_key.jump:key.keyboard.space\r\n"));
        assert!(out.contains("simulationDistance:5\r\n"));
        assert!(out.contains("renderClouds:\"false\"\r\n"));
        assert!(out.contains("entityDistanceScaling:0.5\r\n"));
        assert!(out.ends_with("\r\n"));
        assert!(!out.replace("\r\n", "").contains('\n'));
        assert_eq!(out.matches("renderDistance:").count(), 1);
        // Idempotent.
        let again = edit_options(&out, &preset_options(PerfPreset::Potato, "1.21.1"));
        assert_eq!(again, out);

        // Quality never lowers higher user values but raises low ones.
        let q = edit_options(text, &preset_options(PerfPreset::Quality, "1.21.1"));
        assert!(q.contains("renderDistance:16") && q.contains("graphicsMode:2"));
        let q = edit_options(
            "renderDistance:4\ngraphicsMode:0\n",
            &preset_options(PerfPreset::Quality, "1.21.1"),
        );
        assert_eq!(q, "renderDistance:12\ngraphicsMode:1\n");

        // Missing file → only preset keys.
        let fresh = edit_options("", &preset_options(PerfPreset::Balanced, "1.20.1"));
        assert!(fresh.starts_with("graphicsMode:0\nrenderDistance:10\n"));
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
