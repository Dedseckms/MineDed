//! Building the `LaunchPlan` (arguments, classpath, natives) and rendering it
//! into a command line. Pure functions, except natives extraction.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

use lc_accounts::{LaunchAuth, UserType};
use lc_runtime::{LaunchPlan, QuickPlay};
use lc_types::{AppError, AppResult, ErrorCode};

use crate::meta::{rules_allow, Argument, Env, LibUse, VersionJson};
use crate::prepare::Prepared;
use crate::util::io_err;

pub(crate) const LAUNCHER_NAME: &str = "MineDed";
pub(crate) const LAUNCHER_VERSION: &str = env!("CARGO_PKG_VERSION");

pub(crate) fn classpath_separator() -> &'static str {
    if cfg!(windows) {
        ";"
    } else {
        ":"
    }
}

/// Replaces `${key}` placeholders; unknown keys are left untouched.
pub(crate) fn substitute(s: &str, vars: &HashMap<&str, String>) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        match after.find('}') {
            Some(end) => {
                let key = &after[..end];
                match vars.get(key) {
                    Some(v) => out.push_str(v),
                    None => {
                        out.push_str("${");
                        out.push_str(key);
                        out.push('}');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push_str(&rest[start..]);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn collect(args: &[Argument], env: &Env) -> Vec<String> {
    let mut out = Vec::new();
    for a in args {
        match a {
            Argument::Plain(s) => out.push(s.clone()),
            Argument::Ruled { rules, value } => {
                if rules_allow(rules, env) {
                    out.extend(value.items().into_iter().map(str::to_owned));
                }
            }
        }
    }
    out
}

/// Drops `-cp ${classpath}` pairs: the classpath is rendered from `LaunchPlan::classpath`
/// so contributors can extend it.
fn strip_classpath(args: Vec<String>) -> Vec<String> {
    let mut out = Vec::with_capacity(args.len());
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        if (a == "-cp" || a == "-classpath")
            && it.peek().map(|n| n == "${classpath}").unwrap_or(false)
        {
            it.next();
            continue;
        }
        if a == "${classpath}" {
            continue;
        }
        out.push(a);
    }
    out
}

/// Raw (unsubstituted) JVM and game argument templates of a profile.
/// Modern `arguments` win; legacy `minecraftArguments` gets the default JVM set.
pub(crate) fn arg_templates(profile: &VersionJson, env: &Env) -> (Vec<String>, Vec<String>) {
    let legacy_jvm = || {
        let mut v = Vec::new();
        if env.os == "windows" {
            v.push(
                "-XX:HeapDumpPath=MojangTricksIntelDriversForPerformance_javaw.exe_minecraft.exe.heapdump"
                    .to_owned(),
            );
        }
        if env.os == "osx" {
            v.push("-XstartOnFirstThread".to_owned());
        }
        if env.arch == "x86" {
            v.push("-Xss1M".to_owned());
        }
        v.push("-Djava.library.path=${natives_directory}".to_owned());
        v.push("-Dminecraft.launcher.brand=${launcher_name}".to_owned());
        v.push("-Dminecraft.launcher.version=${launcher_version}".to_owned());
        v.push("-cp".to_owned());
        v.push("${classpath}".to_owned());
        v
    };
    match &profile.arguments {
        Some(a) if !a.game.is_empty() || !a.jvm.is_empty() => {
            let jvm = if a.jvm.is_empty() {
                legacy_jvm()
            } else {
                collect(&a.jvm, env)
            };
            let mut game = collect(&a.game, env);
            if game.is_empty() {
                if let Some(m) = &profile.minecraft_arguments {
                    game = m.split_whitespace().map(str::to_owned).collect();
                }
            }
            (strip_classpath(jvm), game)
        }
        _ => {
            let game = profile
                .minecraft_arguments
                .as_deref()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            (strip_classpath(legacy_jvm()), game)
        }
    }
}

/// 1.20+ profiles advertise `--quickPlayMultiplayer` via a feature rule.
pub(crate) fn supports_quick_play(profile: &VersionJson) -> bool {
    profile
        .arguments
        .as_ref()
        .map(|a| {
            a.game.iter().any(|arg| match arg {
                Argument::Ruled { rules, .. } => rules.iter().any(|r| {
                    r.features
                        .as_ref()
                        .map(|f| f.contains_key("is_quick_play_multiplayer"))
                        .unwrap_or(false)
                }),
                Argument::Plain(_) => false,
            })
        })
        .unwrap_or(false)
}

/// `host[:port]` → `QuickPlay` (default port 25565).
pub(crate) fn parse_server(s: &str) -> Option<QuickPlay> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    // `[::1]:25565` style IPv6.
    if let Some(rest) = s.strip_prefix('[') {
        let (host, tail) = rest.split_once(']')?;
        let port = match tail.strip_prefix(':') {
            Some(p) => p.parse().ok()?,
            None => 25565,
        };
        return Some(QuickPlay {
            host: host.to_owned(),
            port,
        });
    }
    match s.rsplit_once(':') {
        Some((host, port)) if !host.contains(':') => Some(QuickPlay {
            host: host.to_owned(),
            port: port.parse().ok()?,
        }),
        _ => Some(QuickPlay {
            host: s.to_owned(),
            port: 25565,
        }),
    }
}

pub(crate) fn user_type_str(t: UserType) -> &'static str {
    match t {
        UserType::Legacy => "legacy",
        UserType::Mojang => "mojang",
    }
}

/// Everything `build_plan` needs besides the prepared profile.
pub(crate) struct PlanInputs<'a> {
    pub prepared: &'a Prepared,
    pub env: &'a Env,
    pub libraries_dir: &'a Path,
    pub assets_root: &'a Path,
    pub game_dir: &'a Path,
    pub natives_dir: &'a Path,
    pub java: PathBuf,
    pub auth: &'a LaunchAuth,
    pub min_mb: u32,
    pub max_mb: u32,
    pub extra_jvm: &'a [String],
    pub join_server: Option<&'a str>,
}

/// Builds the plan before contributors run.
pub(crate) fn build_plan(i: PlanInputs<'_>) -> AppResult<LaunchPlan> {
    let profile = &i.prepared.profile;
    let main_class = profile
        .main_class
        .clone()
        .ok_or_else(|| AppError::new(ErrorCode::InvalidInput, "version has no mainClass"))?;

    let mut classpath = Vec::new();
    let mut seen = HashSet::new();
    for lib in &i.prepared.libs {
        if lib.usage == LibUse::Classpath {
            let p = lib.local(i.libraries_dir)?;
            if seen.insert(p.clone()) {
                classpath.push(p);
            }
        }
    }
    if seen.insert(i.prepared.client_jar.clone()) {
        classpath.push(i.prepared.client_jar.clone());
    }

    let lossy = |p: &Path| p.to_string_lossy().into_owned();
    let token = i.auth.access_token.expose().to_owned();
    let mut vars: HashMap<&str, String> = HashMap::new();
    vars.insert("auth_player_name", i.auth.name.clone());
    vars.insert("auth_uuid", i.auth.uuid.clone());
    vars.insert("auth_access_token", token.clone());
    vars.insert("auth_session", token);
    vars.insert("auth_xuid", "0".into());
    vars.insert("clientid", String::new());
    vars.insert("user_type", user_type_str(i.auth.user_type).into());
    vars.insert("user_properties", "{}".into());
    vars.insert("version_name", profile.id.clone());
    vars.insert(
        "version_type",
        profile.kind.clone().unwrap_or_else(|| "release".into()),
    );
    vars.insert("game_directory", lossy(i.game_dir));
    vars.insert("assets_root", lossy(i.assets_root));
    vars.insert("game_assets", lossy(&i.prepared.assets.game_assets));
    vars.insert("assets_index_name", i.prepared.assets.index_id.clone());
    vars.insert("natives_directory", lossy(i.natives_dir));
    vars.insert("library_directory", lossy(i.libraries_dir));
    vars.insert("classpath_separator", classpath_separator().into());
    vars.insert("launcher_name", LAUNCHER_NAME.into());
    vars.insert("launcher_version", LAUNCHER_VERSION.into());
    vars.insert("resolution_width", "854".into());
    vars.insert("resolution_height", "480".into());

    let (jvm_t, game_t) = arg_templates(profile, i.env);
    let mut jvm_args = vec![format!("-Xms{}M", i.min_mb), format!("-Xmx{}M", i.max_mb)];
    jvm_args.extend(jvm_t.iter().map(|a| substitute(a, &vars)));
    jvm_args.extend(i.prepared.log_args.iter().cloned());

    let mut java_agents = Vec::new();
    if let Some(a) = &i.auth.authlib {
        java_agents.push(format!("{}={}", a.jar.to_string_lossy(), a.api_root));
        if let Some(pre) = &a.prefetched {
            jvm_args.push(format!("-Dauthlibinjector.yggdrasil.prefetched={pre}"));
        }
    }
    jvm_args.extend(i.extra_jvm.iter().filter(|a| !a.trim().is_empty()).cloned());

    let game_args = game_t.iter().map(|a| substitute(a, &vars)).collect();

    Ok(LaunchPlan {
        java: i.java,
        working_dir: i.game_dir.to_path_buf(),
        main_class,
        classpath,
        jvm_args,
        java_agents,
        game_args,
        env: Default::default(),
        quick_play: i.join_server.and_then(parse_server),
    })
}

/// Final argument list (after the java executable).
pub(crate) fn render_args(plan: &LaunchPlan, modern_quick_play: bool) -> Vec<String> {
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for agent in &plan.java_agents {
        let a = if agent.starts_with("-javaagent:") {
            agent.clone()
        } else {
            format!("-javaagent:{agent}")
        };
        if seen.insert(a.clone()) {
            out.push(a);
        }
    }
    out.extend(plan.jvm_args.iter().cloned());
    if !plan.classpath.is_empty() {
        out.push("-cp".into());
        out.push(
            plan.classpath
                .iter()
                .map(|p| p.to_string_lossy().into_owned())
                .collect::<Vec<_>>()
                .join(classpath_separator()),
        );
    }
    out.push(plan.main_class.clone());
    out.extend(plan.game_args.iter().cloned());
    if let Some(q) = &plan.quick_play {
        if modern_quick_play {
            out.push("--quickPlayMultiplayer".into());
            out.push(format!("{}:{}", q.host, q.port));
        } else {
            out.push("--server".into());
            out.push(q.host.clone());
            out.push("--port".into());
            out.push(q.port.to_string());
        }
    }
    out
}

/// Arguments safe for logs (access token masked).
pub(crate) fn redacted(args: &[String], secret: &str) -> String {
    let joined = args.join(" ");
    if secret.len() < 4 {
        return joined;
    }
    joined.replace(secret, "***")
}

/// Unpacks old-style natives jars into `dir` (cleared first).
pub(crate) fn extract_natives(jars: &[(PathBuf, Vec<String>)], dir: &Path) -> AppResult<()> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| io_err(dir, e))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| io_err(dir, e))?;
    for (jar, exclude) in jars {
        let file = std::fs::File::open(jar).map_err(|e| io_err(jar, e))?;
        let mut zip = zip::ZipArchive::new(file).map_err(|e| zip_err(jar, e))?;
        for idx in 0..zip.len() {
            let mut entry = zip.by_index(idx).map_err(|e| zip_err(jar, e))?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().to_owned();
            if name.starts_with("META-INF/") || exclude.iter().any(|x| name.starts_with(x.as_str()))
            {
                continue;
            }
            let Some(rel) = entry.enclosed_name() else {
                continue;
            };
            let out = dir.join(rel);
            if let Some(parent) = out.parent() {
                std::fs::create_dir_all(parent).map_err(|e| io_err(parent, e))?;
            }
            let mut f = std::fs::File::create(&out).map_err(|e| io_err(&out, e))?;
            std::io::copy(&mut entry, &mut f).map_err(|e| io_err(&out, e))?;
        }
    }
    Ok(())
}

fn zip_err(path: &Path, e: zip::result::ZipError) -> AppError {
    AppError::new(ErrorCode::Io, format!("bad natives jar: {e}"))
        .detail("path", path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meta::{resolve_libraries, Features};
    use crate::prepare::AssetsInfo;
    use lc_accounts::{AuthlibInjector, Secret};

    fn env(os: &'static str) -> Env {
        Env {
            os,
            arch: "x86_64",
            os_version: "10.0".into(),
            features: Features::default(),
        }
    }

    fn fixture(name: &str) -> VersionJson {
        let p = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures")
            .join(name);
        serde_json::from_slice(&std::fs::read(p).unwrap()).unwrap()
    }

    fn auth(authlib: bool) -> LaunchAuth {
        LaunchAuth {
            name: "Steve".into(),
            uuid: "0123456789abcdef0123456789abcdef".into(),
            access_token: Secret::new("tok-secret-123"),
            user_type: if authlib {
                UserType::Mojang
            } else {
                UserType::Legacy
            },
            authlib: authlib.then(|| AuthlibInjector {
                jar: PathBuf::from("/data/authlib-injector.jar"),
                api_root: url::Url::parse("https://mc.example.ru/api/yggdrasil/").unwrap(),
                prefetched: Some("eyJ9".into()),
            }),
        }
    }

    fn prepared(profile: VersionJson, e: &Env) -> Prepared {
        let libs = resolve_libraries(&profile.libraries, e).unwrap();
        Prepared {
            client_jar: PathBuf::from("/root/versions")
                .join(&profile.id)
                .join(format!("{}.jar", profile.id)),
            assets: AssetsInfo {
                index_id: profile.asset_index.as_ref().unwrap().id.clone(),
                game_assets: PathBuf::from("/root/assets"),
            },
            log_args: vec!["-Dlog4j.configurationFile=/root/assets/log_configs/client.xml".into()],
            libs,
            profile,
        }
    }

    fn plan_for(name: &str, os: &'static str, authlib: bool, join: Option<&str>) -> LaunchPlan {
        let e = env(os);
        let p = prepared(fixture(name), &e);
        let a = auth(authlib);
        build_plan(PlanInputs {
            prepared: &p,
            env: &e,
            libraries_dir: Path::new("/root/libraries"),
            assets_root: Path::new("/root/assets"),
            game_dir: Path::new("/root/instances/x/minecraft"),
            natives_dir: Path::new("/root/instances/x/natives"),
            java: PathBuf::from("/java/bin/java"),
            auth: &a,
            min_mb: 512,
            max_mb: 4096,
            extra_jvm: &["-XX:+UseG1GC".to_string()],
            join_server: join,
        })
        .unwrap()
    }

    fn after<'a>(args: &'a [String], flag: &str) -> &'a str {
        let i = args.iter().position(|a| a == flag).unwrap();
        &args[i + 1]
    }

    #[test]
    fn substitution() {
        let mut v = HashMap::new();
        v.insert("a", "1".to_string());
        assert_eq!(substitute("x${a}y${b}z${", &v), "x1y${b}z${");
    }

    #[test]
    fn modern_arguments_1_21_1() {
        let plan = plan_for("1.21.1.json", "windows", false, None);
        assert_eq!(plan.main_class, "net.minecraft.client.main.Main");
        assert_eq!(&plan.jvm_args[..2], &["-Xms512M", "-Xmx4096M"]);
        assert!(plan
            .jvm_args
            .contains(&"-Djava.library.path=/root/instances/x/natives".to_string()));
        assert!(plan
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-XX:HeapDumpPath=")));
        assert!(!plan.jvm_args.iter().any(|a| a == "-XstartOnFirstThread"));
        assert!(!plan.jvm_args.iter().any(|a| a == "-Xss1M"));
        assert!(!plan.jvm_args.iter().any(|a| a == "-cp" || a.contains("${")));
        assert!(plan
            .jvm_args
            .contains(&"-Dminecraft.launcher.brand=MineDed".to_string()));
        assert_eq!(plan.jvm_args.last().unwrap(), "-XX:+UseG1GC");

        let g = &plan.game_args;
        assert_eq!(after(g, "--username"), "Steve");
        assert_eq!(after(g, "--version"), "1.21.1");
        assert_eq!(after(g, "--assetIndex"), "17");
        assert_eq!(after(g, "--accessToken"), "tok-secret-123");
        assert_eq!(after(g, "--userType"), "legacy");
        assert_eq!(after(g, "--xuid"), "0");
        assert_eq!(after(g, "--clientId"), "");
        assert_eq!(after(g, "--versionType"), "release");
        assert!(!g.iter().any(|a| a == "--demo" || a == "--width"));
        assert!(!g.iter().any(|a| a.starts_with("--quickPlay")));
        assert!(!g.iter().any(|a| a.contains("${")));

        // Classpath: libs + client jar last, natives-windows on classpath.
        assert!(plan
            .classpath
            .last()
            .unwrap()
            .ends_with(Path::new("1.21.1").join("1.21.1.jar")));
        assert!(plan.classpath.iter().any(|p| p
            .to_string_lossy()
            .ends_with("lwjgl-3.3.3-natives-windows.jar")));
        assert!(supports_quick_play(
            &prepared(fixture("1.21.1.json"), &env("windows")).profile
        ));
    }

    #[test]
    fn legacy_arguments_1_12_2() {
        let plan = plan_for("1.12.2.json", "linux", true, Some("play.example.ru:25570"));
        let g = &plan.game_args;
        assert_eq!(after(g, "--username"), "Steve");
        assert_eq!(after(g, "--version"), "1.12.2");
        assert_eq!(after(g, "--assetIndex"), "1.12");
        assert_eq!(after(g, "--gameDir"), "/root/instances/x/minecraft");
        assert_eq!(after(g, "--userType"), "mojang");
        assert_eq!(after(g, "--uuid"), "0123456789abcdef0123456789abcdef");
        assert!(plan
            .jvm_args
            .contains(&"-Djava.library.path=/root/instances/x/natives".to_string()));
        assert!(!plan
            .jvm_args
            .iter()
            .any(|a| a.starts_with("-XX:HeapDumpPath")));
        assert!(plan
            .jvm_args
            .contains(&"-Dauthlibinjector.yggdrasil.prefetched=eyJ9".to_string()));
        // Natives jars are not on the classpath.
        assert!(!plan
            .classpath
            .iter()
            .any(|p| p.to_string_lossy().contains("natives-")));
        assert!(plan.classpath.iter().any(|p| p
            .to_string_lossy()
            .contains("lwjgl-2.9.4-nightly-20150209.jar")));

        let modern = supports_quick_play(&prepared(fixture("1.12.2.json"), &env("linux")).profile);
        assert!(!modern);
        let args = render_args(&plan, modern);
        assert!(args[0].starts_with("-javaagent:/data/authlib-injector.jar=https://mc.example.ru/"));
        let cp = after(&args, "-cp");
        assert!(cp.ends_with("1.12.2.jar"));
        let main = args
            .iter()
            .position(|a| a == "net.minecraft.client.main.Main")
            .unwrap();
        assert_eq!(args[main - 2], "-cp");
        assert_eq!(after(&args, "--server"), "play.example.ru");
        assert_eq!(after(&args, "--port"), "25570");
        assert!(!redacted(&args, "tok-secret-123").contains("tok-secret-123"));
    }

    #[test]
    fn quick_play_modern_and_agents_dedup() {
        let mut plan = plan_for("1.21.1.json", "linux", true, Some("host"));
        plan.java_agents.push(
            "-javaagent:/data/authlib-injector.jar=https://mc.example.ru/api/yggdrasil/".into(),
        );
        let args = render_args(&plan, true);
        assert_eq!(
            args.iter().filter(|a| a.starts_with("-javaagent:")).count(),
            1
        );
        assert_eq!(after(&args, "--quickPlayMultiplayer"), "host:25565");
        assert!(!args.iter().any(|a| a == "--server"));
    }

    #[test]
    fn server_parsing() {
        assert_eq!(
            parse_server("a.b:1234"),
            Some(QuickPlay {
                host: "a.b".into(),
                port: 1234
            })
        );
        assert_eq!(parse_server("a.b").unwrap().port, 25565);
        assert_eq!(parse_server("[::1]:7").unwrap().host, "::1");
        assert_eq!(parse_server("a:notaport"), None);
        assert_eq!(parse_server(" "), None);
    }

    #[test]
    fn natives_extraction() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let jar = dir.path().join("natives.jar");
        {
            let f = std::fs::File::create(&jar).unwrap();
            let mut z = zip::ZipWriter::new(f);
            let opts = zip::write::SimpleFileOptions::default();
            z.start_file("lwjgl64.dll", opts).unwrap();
            z.write_all(b"dll").unwrap();
            z.start_file("META-INF/MANIFEST.MF", opts).unwrap();
            z.write_all(b"m").unwrap();
            z.start_file("skip/me.txt", opts).unwrap();
            z.write_all(b"x").unwrap();
            z.start_file("../evil.dll", opts).unwrap();
            z.write_all(b"e").unwrap();
            z.finish().unwrap();
        }
        let out = dir.path().join("out");
        std::fs::create_dir_all(&out).unwrap();
        std::fs::write(out.join("stale.dll"), b"old").unwrap();
        extract_natives(&[(jar, vec!["skip/".into()])], &out).unwrap();
        assert_eq!(std::fs::read(out.join("lwjgl64.dll")).unwrap(), b"dll");
        assert!(!out.join("META-INF").exists());
        assert!(!out.join("skip").exists());
        assert!(!out.join("stale.dll").exists());
        assert!(!dir.path().join("evil.dll").exists());
    }
}
