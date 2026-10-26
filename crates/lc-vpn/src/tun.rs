//! Admin rights for `VpnMode::Apps` (TUN) without elevating the launcher.
//!
//! - Windows: the launcher re-runs its own executable elevated (UAC, via
//!   `Start-Process -Verb RunAs` in a hidden PowerShell wrapper) with
//!   [`HELPER_ARG`]. That helper spawns mihomo and kills it when the launcher
//!   process exits or the stop flag file appears. The launcher talks to
//!   mihomo only through its loopback external-controller.
//! - Linux: mihomo is copied into our data dir and granted
//!   `cap_net_admin,cap_net_bind_service` once via `pkexec setcap`; it then
//!   runs as a normal child process.

#[cfg(windows)]
use std::ffi::OsString;
use std::path::Path;
#[cfg(windows)]
use std::path::PathBuf;

#[cfg(target_os = "linux")]
use lc_types::{AppError, AppResult, ErrorCode};

/// First CLI argument that turns the launcher executable into the TUN helper.
pub const HELPER_ARG: &str = "--lc-vpn-tun-helper";

/// Wrapper exit codes (Windows).
#[cfg(windows)]
pub(crate) const EXIT_DECLINED: i32 = 2;
#[cfg(windows)]
pub(crate) const EXIT_SPAWN_FAILED: i32 = 3;

/// Entry point for the CLI hook in the app's `main`: when the process was
/// started as the TUN helper, runs it to completion and returns its exit code.
/// Returns `None` for normal launches.
pub fn run_tun_helper_from_args() -> Option<i32> {
    let mut args = std::env::args_os().skip(1);
    if args.next()? != HELPER_ARG {
        return None;
    }
    #[cfg(windows)]
    {
        Some(helper::run(&args.collect::<Vec<_>>()))
    }
    #[cfg(not(windows))]
    {
        let _ = args;
        Some(1)
    }
}

/// Paths and parent of one elevated run.
#[cfg(windows)]
pub(crate) struct HelperArgs {
    pub mihomo: PathBuf,
    pub dir: PathBuf,
    pub config: PathBuf,
    pub log: PathBuf,
    pub stop: PathBuf,
    pub parent: u32,
}

#[cfg(windows)]
impl HelperArgs {
    fn to_args(&self) -> Vec<OsString> {
        vec![
            self.mihomo.clone().into_os_string(),
            self.dir.clone().into_os_string(),
            self.config.clone().into_os_string(),
            self.log.clone().into_os_string(),
            self.stop.clone().into_os_string(),
            self.parent.to_string().into(),
        ]
    }

    /// Parses and sanity-checks helper arguments: every file must live in
    /// `dir` and the executable must be mihomo.
    fn parse(args: &[OsString]) -> Option<Self> {
        let [mihomo, dir, config, log, stop, parent] = args else {
            return None;
        };
        let a = Self {
            mihomo: mihomo.into(),
            dir: dir.into(),
            config: config.into(),
            log: log.into(),
            stop: stop.into(),
            parent: parent.to_str()?.parse().ok()?,
        };
        let name = a.mihomo.file_name()?.to_str()?.to_ascii_lowercase();
        let inside = |p: &Path| p.parent() == Some(a.dir.as_path());
        (name == "mihomo.exe" && inside(&a.config) && inside(&a.log) && inside(&a.stop))
            .then_some(a)
    }
}

/// MSVC command-line quoting of one argument.
#[cfg(windows)]
fn quote_arg(arg: &str) -> String {
    let mut out = String::from("\"");
    let mut backslashes = 0usize;
    for c in arg.chars() {
        match c {
            '\\' => backslashes += 1,
            '"' => {
                out.push_str(&"\\".repeat(backslashes * 2 + 1));
                out.push('"');
                backslashes = 0;
            }
            _ => {
                out.push_str(&"\\".repeat(backslashes));
                out.push(c);
                backslashes = 0;
            }
        }
    }
    out.push_str(&"\\".repeat(backslashes * 2));
    out.push('"');
    out
}

/// PowerShell single-quoted literal.
#[cfg(windows)]
fn ps_literal(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

/// Script that starts `launcher HELPER_ARG ...` elevated and waits for it.
/// Exit codes: 0 helper finished, [`EXIT_DECLINED`] UAC declined,
/// [`EXIT_SPAWN_FAILED`] anything else.
#[cfg(windows)]
pub(crate) fn wrapper_script(launcher: &Path, h: &HelperArgs) -> String {
    let mut args = vec![quote_arg(HELPER_ARG)];
    args.extend(h.to_args().iter().map(|a| quote_arg(&a.to_string_lossy())));
    format!(
        "$ErrorActionPreference = 'Stop'\n\
         try {{\n\
         \x20 $p = Start-Process -FilePath {exe} -ArgumentList {args} -Verb RunAs -WindowStyle Hidden -PassThru\n\
         }} catch {{\n\
         \x20 $e = $_.Exception\n\
         \x20 while ($e -and -not ($e -is [System.ComponentModel.Win32Exception])) {{ $e = $e.InnerException }}\n\
         \x20 if ($e -and $e.NativeErrorCode -eq 1223) {{ exit {EXIT_DECLINED} }}\n\
         \x20 exit {EXIT_SPAWN_FAILED}\n\
         }}\n\
         $p.WaitForExit()\n\
         exit 0\n",
        exe = ps_literal(&launcher.to_string_lossy()),
        args = ps_literal(&args.join(" ")),
    )
}

/// Hidden PowerShell process running [`wrapper_script`].
#[cfg(windows)]
pub(crate) fn wrapper_command(
    launcher: &Path,
    h: &HelperArgs,
) -> (PathBuf, tokio::process::Command) {
    use base64::Engine;
    let script = wrapper_script(launcher, h);
    let utf16: Vec<u8> = script.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let ps = PathBuf::from(root)
        .join("System32")
        .join("WindowsPowerShell")
        .join("v1.0")
        .join("powershell.exe");
    let mut cmd = tokio::process::Command::new(&ps);
    cmd.args([
        "-NoProfile",
        "-NonInteractive",
        "-WindowStyle",
        "Hidden",
        "-EncodedCommand",
    ])
    .arg(encoded)
    .creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    (ps, cmd)
}

/// Asks the elevated helper to stop mihomo and exit.
pub(crate) fn signal_stop(flag: &Path) {
    let _ = std::fs::write(flag, b"stop");
}

#[cfg(windows)]
mod helper {
    use std::ffi::OsString;
    use std::process::{Command, Stdio};
    use std::time::Duration;

    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};

    use super::HelperArgs;

    const POLL: Duration = Duration::from_millis(250);

    fn start_time(sys: &mut System, pid: Pid) -> Option<u64> {
        sys.refresh_processes_specifics(
            ProcessesToUpdate::Some(&[pid]),
            true,
            ProcessRefreshKind::nothing(),
        );
        sys.process(pid).map(sysinfo::Process::start_time)
    }

    /// Runs elevated: mihomo lives exactly as long as the launcher wants it.
    pub(super) fn run(args: &[OsString]) -> i32 {
        let Some(a) = HelperArgs::parse(args) else {
            return 64;
        };
        if a.stop.exists() {
            return 0;
        }
        let parent = Pid::from_u32(a.parent);
        let mut sys = System::new();
        let Some(born) = start_time(&mut sys, parent) else {
            return 0;
        };
        let Ok(log) = std::fs::File::create(&a.log) else {
            return 65;
        };
        let Ok(log2) = log.try_clone() else {
            return 65;
        };
        use std::os::windows::process::CommandExt;
        let child = Command::new(&a.mihomo)
            .arg("-d")
            .arg(&a.dir)
            .arg("-f")
            .arg(&a.config)
            .current_dir(&a.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log2))
            .creation_flags(0x0800_0000)
            .spawn();
        let Ok(mut child) = child else {
            return 66;
        };
        loop {
            if let Ok(Some(status)) = child.try_wait() {
                return status.code().unwrap_or(1);
            }
            let parent_gone = start_time(&mut sys, parent) != Some(born);
            if parent_gone || a.stop.exists() {
                let _ = child.kill();
                let _ = child.wait();
                let _ = std::fs::remove_file(&a.stop);
                return 0;
            }
            std::thread::sleep(POLL);
        }
    }
}

/// Copies mihomo to `<dir>/tun/mihomo` (once per binary change) and grants it
/// the TUN capabilities through `pkexec setcap` when missing.
#[cfg(target_os = "linux")]
pub(crate) async fn linux_prepare(src: &Path, dir: &Path) -> AppResult<std::path::PathBuf> {
    let target = dir.join("tun").join("mihomo");
    let io = |e: std::io::Error, p: &Path| {
        AppError::new(ErrorCode::Io, e.to_string()).detail("path", p.display().to_string())
    };
    let same = match (tokio::fs::read(src).await, tokio::fs::read(&target).await) {
        (Ok(a), Ok(b)) => a == b,
        (Err(e), _) => return Err(io(e, src)),
        _ => false,
    };
    if !same {
        let parent = target
            .parent()
            .ok_or_else(|| AppError::internal("tun path has no parent"))?;
        tokio::fs::create_dir_all(parent)
            .await
            .map_err(|e| io(e, parent))?;
        let tmp = target.with_extension("tmp");
        tokio::fs::copy(src, &tmp).await.map_err(|e| io(e, &tmp))?;
        {
            use std::os::unix::fs::PermissionsExt;
            tokio::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o755))
                .await
                .map_err(|e| io(e, &tmp))?;
        }
        tokio::fs::rename(&tmp, &target)
            .await
            .map_err(|e| io(e, &target))?;
    }
    if has_caps(&target).await == Some(true) {
        return Ok(target);
    }
    let manual = format!(
        "sudo setcap cap_net_admin,cap_net_bind_service+ep '{}'",
        target.display()
    );
    let denied = |msg: &str| {
        AppError::new(ErrorCode::PermissionDenied, msg)
            .detail("command", manual.clone())
            .detail("path", target.display().to_string())
    };
    let setcap = find(&["/usr/sbin/setcap", "/sbin/setcap", "/usr/bin/setcap"])
        .ok_or_else(|| denied("setcap is not installed (package libcap2-bin / libcap)"))?;
    let pkexec = find(&["/usr/bin/pkexec", "/bin/pkexec"])
        .ok_or_else(|| denied("pkexec (polkit) is not available; run the command manually"))?;
    let status = tokio::process::Command::new(pkexec)
        .arg(setcap)
        .arg("cap_net_admin,cap_net_bind_service+ep")
        .arg(&target)
        .stdin(std::process::Stdio::null())
        .status()
        .await
        .map_err(|e| denied(&format!("cannot run pkexec: {e}")))?;
    match status.code() {
        Some(0) => {}
        Some(126) => {
            return Err(
                denied("administrator authorization was declined").detail("reason", "declined")
            )
        }
        Some(127) => return Err(denied("not authorized to grant network capabilities")),
        c => {
            return Err(denied("setcap failed")
                .detail("status", c.map(|c| c.to_string()).unwrap_or_default()))
        }
    }
    if has_caps(&target).await == Some(false) {
        return Err(denied(
            "capabilities were not applied (filesystem may not support them, e.g. nosuid)",
        ));
    }
    Ok(target)
}

#[cfg(target_os = "linux")]
fn find(candidates: &[&str]) -> Option<std::path::PathBuf> {
    candidates
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}

/// `Some(true)` when `getcap` lists `cap_net_admin`; `None` without getcap.
#[cfg(target_os = "linux")]
async fn has_caps(path: &Path) -> Option<bool> {
    let getcap = find(&["/usr/sbin/getcap", "/sbin/getcap", "/usr/bin/getcap"])?;
    let out = tokio::process::Command::new(getcap)
        .arg(path)
        .output()
        .await
        .ok()?;
    Some(String::from_utf8_lossy(&out.stdout).contains("cap_net_admin"))
}

#[cfg(test)]
mod tests {
    #[test]
    fn normal_launch_is_not_the_helper() {
        assert_eq!(super::run_tun_helper_from_args(), None);
    }

    #[cfg(windows)]
    #[test]
    fn wrapper_quotes_and_helper_args_roundtrip() {
        use super::*;
        assert_eq!(quote_arg(r"C:\a b\x.exe"), r#""C:\a b\x.exe""#);
        assert_eq!(quote_arg(r"C:\dir\"), r#""C:\dir\\""#);
        assert_eq!(quote_arg(r#"a"b"#), r#""a\"b""#);
        assert_eq!(ps_literal("O'Neil"), "'O''Neil'");

        let dir = PathBuf::from(r"C:\Users\O'Neil\AppData\Roaming\lc\mihomo");
        let h = HelperArgs {
            mihomo: dir.join("bin").join("mihomo.exe"),
            dir: dir.clone(),
            config: dir.join("config.json"),
            log: dir.join("mihomo.log"),
            stop: dir.join("tun.stop"),
            parent: 4242,
        };
        let script = wrapper_script(Path::new(r"C:\Program Files\L\launcher.exe"), &h);
        assert!(script.contains("-Verb RunAs"));
        assert!(script.contains("'C:\\Program Files\\L\\launcher.exe'"));
        assert!(script.contains("O''Neil"));
        assert!(script.contains("exit 2"));
        assert!(script.contains(HELPER_ARG));

        let parsed = HelperArgs::parse(&h.to_args()).unwrap();
        assert_eq!(parsed.parent, 4242);
        assert_eq!(parsed.config, h.config);
        // Files outside the dir or a foreign executable are refused.
        let mut evil = h.to_args();
        evil[2] = r"C:\Windows\System32\config.json".into();
        assert!(HelperArgs::parse(&evil).is_none());
        let mut evil = h.to_args();
        evil[0] = r"C:\x\cmd.exe".into();
        assert!(HelperArgs::parse(&evil).is_none());
        assert!(HelperArgs::parse(&h.to_args()[..5]).is_none());
        // Already-stopped run exits without touching anything.
        let tmp = std::env::temp_dir().join(format!("lc-vpn-tun-{}", std::process::id()));
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(tmp.join("tun.stop"), b"stop").unwrap();
        let h2 = HelperArgs {
            mihomo: tmp.join("mihomo.exe"),
            dir: tmp.clone(),
            config: tmp.join("config.json"),
            log: tmp.join("mihomo.log"),
            stop: tmp.join("tun.stop"),
            parent: std::process::id(),
        };
        assert_eq!(helper::run(&h2.to_args()), 0);
        assert!(!tmp.join("mihomo.log").exists());
        let _ = std::fs::remove_dir_all(tmp);
    }
}
