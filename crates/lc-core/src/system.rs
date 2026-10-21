//! Hardware probe and memory recommendation.

use std::path::Path;

use lc_types::instance::PerfPreset;
use lc_types::system::{MemoryRecommendation, SystemInfo};
use lc_types::AppResult;

use crate::java::command;
use crate::util::join;

const MB: u64 = 1024 * 1024;
/// Mod count from which an instance counts as a large modpack.
pub(crate) const LARGE_MODPACK: usize = 80;

/// `clamp(total × 0.4, 2 GB, 8 GB)`, at least 4 GB for large modpacks (PLAN.md),
/// never more than total − 1.5 GB. Rounded down to 256 MB.
pub(crate) fn recommend(
    total_mb: u64,
    free_mb: u64,
    mod_count: usize,
    preset: Option<PerfPreset>,
) -> MemoryRecommendation {
    let base = ((total_mb as f64) * 0.4) as u64;
    let mut max = base.clamp(2048, 8192);
    let large = mod_count >= LARGE_MODPACK;
    if large {
        max = max.max(4096);
    }
    let ceiling = total_mb.saturating_sub(1536).max(1024);
    max = max.min(ceiling);
    max = (max / 256 * 256).max(1024);
    let min = if max >= 4096 { 1024 } else { 512 };

    let reason_key = if free_mb < max && preset != Some(PerfPreset::Potato) {
        "memory.reason.lowFreeRam"
    } else if large {
        "memory.reason.largeModpack"
    } else if total_mb < 4096 {
        "memory.reason.lowTotalRam"
    } else {
        "memory.reason.default"
    };
    MemoryRecommendation {
        min_mb: min as u32,
        max_mb: max as u32,
        reason_key: reason_key.to_owned(),
    }
}

/// Few threads, little RAM or an integrated GPU.
pub(crate) fn weak_hardware(cpu_threads: u32, total_ram_mb: u64, gpu: Option<&str>) -> bool {
    let igpu = gpu.map(is_integrated_gpu).unwrap_or(false);
    cpu_threads <= 4 || total_ram_mb < 6 * 1024 || igpu
}

fn is_integrated_gpu(name: &str) -> bool {
    let n = name.to_lowercase();
    let intel_igpu = n.contains("intel")
        && (n.contains("hd graphics") || n.contains("uhd") || n.contains("iris"));
    let amd_apu = n.contains("radeon")
        && (n.contains("vega")
            || n.trim_end().ends_with("radeon(tm) graphics")
            || n.contains("radeon graphics"));
    intel_igpu || amd_apu || n.contains("llvmpipe") || n.contains("microsoft basic")
}

/// Count of enabled mods (`*.jar`) in `<game_dir>/mods`.
pub(crate) async fn count_mods(game_dir: &Path) -> usize {
    let mut n = 0;
    if let Ok(mut rd) = tokio::fs::read_dir(game_dir.join("mods")).await {
        while let Ok(Some(e)) = rd.next_entry().await {
            if e.file_name()
                .to_string_lossy()
                .to_lowercase()
                .ends_with(".jar")
            {
                n += 1;
            }
        }
    }
    n
}

/// `(total, free)` RAM in MB.
pub(crate) async fn ram_mb() -> AppResult<(u64, u64)> {
    join(
        tokio::task::spawn_blocking(|| {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            Ok((sys.total_memory() / MB, sys.available_memory() / MB))
        })
        .await,
    )
}

pub(crate) async fn info() -> AppResult<SystemInfo> {
    let gpu = gpu_name().await;
    join(
        tokio::task::spawn_blocking(move || {
            let mut sys = sysinfo::System::new();
            sys.refresh_memory();
            sys.refresh_cpu_all();
            let cpu_threads = sys.cpus().len().max(1) as u32;
            let cpu_model = sys
                .cpus()
                .first()
                .map(|c| c.brand().trim().to_owned())
                .unwrap_or_default();
            let total_ram_mb = sys.total_memory() / MB;
            let free_ram_mb = sys.available_memory() / MB;
            let os = if cfg!(windows) { "windows" } else { "linux" };
            let wayland = !cfg!(windows)
                && (std::env::var_os("WAYLAND_DISPLAY").is_some()
                    || std::env::var("XDG_SESSION_TYPE")
                        .map(|s| s.eq_ignore_ascii_case("wayland"))
                        .unwrap_or(false));
            Ok(SystemInfo {
                os: os.to_owned(),
                os_version: sysinfo::System::long_os_version()
                    .or_else(sysinfo::System::os_version)
                    .unwrap_or_default(),
                arch: crate::java::current_arch(),
                cpu_model,
                cpu_threads,
                total_ram_mb,
                free_ram_mb,
                weak_hardware: weak_hardware(cpu_threads, total_ram_mb, gpu.as_deref()),
                gpu,
                wayland,
            })
        })
        .await,
    )
}

/// Best-effort GPU name: registry on Windows, `lspci` on Linux.
async fn gpu_name() -> Option<String> {
    let (program, args): (&str, &[&str]) = if cfg!(windows) {
        (
            "reg",
            &[
                "query",
                r"HKLM\SYSTEM\CurrentControlSet\Control\Class\{4d36e968-e325-11ce-bfc1-08002be10318}",
                "/s",
                "/v",
                "DriverDesc",
            ],
        )
    } else {
        ("lspci", &[])
    };
    let mut cmd = command(Path::new(program));
    cmd.args(args)
        .stdin(std::process::Stdio::null())
        .kill_on_drop(true);
    let out = tokio::time::timeout(std::time::Duration::from_secs(5), cmd.output())
        .await
        .ok()?
        .ok()?;
    let text = String::from_utf8_lossy(&out.stdout);
    parse_gpu(&text, cfg!(windows))
}

/// Picks a discrete GPU when several are listed.
pub(crate) fn parse_gpu(text: &str, windows: bool) -> Option<String> {
    let names: Vec<String> = if windows {
        text.lines()
            .filter_map(|l| {
                let l = l.trim();
                l.strip_prefix("DriverDesc")
                    .and_then(|r| r.trim().strip_prefix("REG_SZ"))
                    .map(|v| v.trim().to_owned())
            })
            .filter(|v| !v.is_empty())
            .collect()
    } else {
        text.lines()
            .filter(|l| l.contains("VGA compatible controller") || l.contains("3D controller"))
            .filter_map(|l| l.split_once(": ").map(|(_, rest)| rest))
            .map(|s| s.trim().to_owned())
            .collect()
    };
    names
        .iter()
        .find(|n| !is_integrated_gpu(n))
        .or_else(|| names.first())
        .cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_formula() {
        // 16 GB → 6.4 GB → rounded to 6400/256*256 = 6400.
        let r = recommend(16384, 10000, 0, None);
        assert_eq!(r.max_mb, 6553 / 256 * 256);
        assert_eq!(r.reason_key, "memory.reason.default");
        // 32 GB → capped at 8 GB.
        assert_eq!(recommend(32768, 20000, 0, None).max_mb, 8192);
        // 4 GB → floor 2 GB.
        let r = recommend(4096, 3000, 0, None);
        assert_eq!(r.max_mb, 2048);
        assert_eq!(r.min_mb, 512);
        // Large modpack on 8 GB → at least 4 GB.
        let r = recommend(8192, 6000, 150, None);
        assert_eq!(r.max_mb, 4096);
        assert_eq!(r.min_mb, 1024);
        assert_eq!(r.reason_key, "memory.reason.largeModpack");
        // Tiny machine: never above total - 1.5 GB.
        let r = recommend(3072, 3000, 0, None);
        assert_eq!(r.max_mb, 1536);
        assert_eq!(r.reason_key, "memory.reason.lowTotalRam");
        // Low free RAM → suggest potato.
        assert_eq!(
            recommend(16384, 1000, 0, None).reason_key,
            "memory.reason.lowFreeRam"
        );
        assert_ne!(
            recommend(16384, 1000, 0, Some(PerfPreset::Potato)).reason_key,
            "memory.reason.lowFreeRam"
        );
    }

    #[test]
    fn weak_hardware_heuristic() {
        assert!(weak_hardware(4, 16384, None));
        assert!(weak_hardware(8, 4096, None));
        assert!(weak_hardware(8, 16384, Some("Intel(R) UHD Graphics 620")));
        assert!(!weak_hardware(12, 16384, Some("NVIDIA GeForce RTX 3060")));
        assert!(!weak_hardware(
            12,
            16384,
            Some("Intel(R) Arc(TM) A770 Graphics")
        ));
    }

    #[test]
    fn gpu_parsing() {
        let reg = "HKEY_LOCAL_MACHINE\\...\\0000\r\n    DriverDesc    REG_SZ    Intel(R) UHD Graphics 630\r\n\r\nHKEY_LOCAL_MACHINE\\...\\0001\r\n    DriverDesc    REG_SZ    NVIDIA GeForce GTX 1650\r\n";
        assert_eq!(
            parse_gpu(reg, true).as_deref(),
            Some("NVIDIA GeForce GTX 1650")
        );
        let lspci = "00:02.0 VGA compatible controller: Intel Corporation HD Graphics 620 (rev 02)\n00:1f.3 Audio device: Intel Corporation Sunrise Point-LP HD Audio\n";
        assert_eq!(
            parse_gpu(lspci, false).as_deref(),
            Some("Intel Corporation HD Graphics 620 (rev 02)")
        );
        assert_eq!(parse_gpu("", false), None);
    }
}
