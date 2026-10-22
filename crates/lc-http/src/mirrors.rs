//! Mirror expansion for official Mojang hosts.

use url::Url;

/// BMCLAPI root (https://bmclapidoc.bangbang93.com).
pub const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";

/// `(host, path prefix replaced, BMCLAPI prefix)`; see BMCLAPI docs.
const BMCLAPI_MAP: &[(&str, &str, &str)] = &[
    ("piston-meta.mojang.com", "", ""),
    ("launchermeta.mojang.com", "", ""),
    ("piston-data.mojang.com", "", ""),
    ("launcher.mojang.com", "", ""),
    ("libraries.minecraft.net", "", "/maven"),
    ("resources.download.minecraft.net", "", "/assets"),
    ("meta.fabricmc.net", "", "/fabric-meta"),
    ("maven.fabricmc.net", "", "/maven"),
    ("maven.minecraftforge.net", "", "/maven"),
    ("files.minecraftforge.net", "/maven", "/maven"),
    ("maven.neoforged.net", "/releases", "/maven"),
];

/// Maps an official Mojang / Fabric / Forge / NeoForge URL onto BMCLAPI, or
/// `None` for other hosts.
pub fn bmclapi_for(original: &Url) -> Option<Url> {
    let host = original.host_str()?;
    let path = original.path();
    let (_, strip, prefix) = BMCLAPI_MAP.iter().find(|(h, strip, _)| {
        *h == host
            && (strip.is_empty()
                || path == *strip
                || path
                    .strip_prefix(strip)
                    .is_some_and(|rest| rest.starts_with('/')))
    })?;
    let rest = &path[strip.len()..];
    let mut url = Url::parse(&format!("{BMCLAPI}{prefix}{rest}")).ok()?;
    url.set_query(original.query());
    Some(url)
}

/// Original first, mirrors after (or the reverse with `prefer_mirrors`).
pub fn candidates(original: &Url, use_bmclapi: bool, prefer_mirrors: bool) -> Vec<Url> {
    let mirrors: Vec<Url> = if use_bmclapi {
        bmclapi_for(original).into_iter().collect()
    } else {
        Vec::new()
    };
    let mut out = Vec::with_capacity(mirrors.len() + 1);
    if prefer_mirrors {
        out.extend(mirrors);
        out.push(original.clone());
    } else {
        out.push(original.clone());
        out.extend(mirrors);
    }
    out
}

/// Expands every URL and removes duplicates, keeping the first occurrence.
pub fn expand_all(urls: &[Url], use_bmclapi: bool, prefer_mirrors: bool) -> Vec<Url> {
    let mut out: Vec<Url> = Vec::new();
    for u in urls {
        for c in candidates(u, use_bmclapi, prefer_mirrors) {
            if !out.contains(&c) {
                out.push(c);
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn u(s: &str) -> Url {
        Url::parse(s).unwrap()
    }

    #[test]
    fn maps_mojang_hosts() {
        let cases = [
            (
                "https://piston-meta.mojang.com/mc/game/version_manifest_v2.json",
                "https://bmclapi2.bangbang93.com/mc/game/version_manifest_v2.json",
            ),
            (
                "https://launchermeta.mojang.com/v1/packages/abc/1.20.json",
                "https://bmclapi2.bangbang93.com/v1/packages/abc/1.20.json",
            ),
            (
                "https://piston-data.mojang.com/v1/objects/abc/client.jar",
                "https://bmclapi2.bangbang93.com/v1/objects/abc/client.jar",
            ),
            (
                "https://launcher.mojang.com/v1/objects/abc/server.jar",
                "https://bmclapi2.bangbang93.com/v1/objects/abc/server.jar",
            ),
            (
                "https://libraries.minecraft.net/com/mojang/brigadier/1.0.18/brigadier-1.0.18.jar",
                "https://bmclapi2.bangbang93.com/maven/com/mojang/brigadier/1.0.18/brigadier-1.0.18.jar",
            ),
            (
                "https://resources.download.minecraft.net/ab/abcdef?x=1",
                "https://bmclapi2.bangbang93.com/assets/ab/abcdef?x=1",
            ),
            (
                "https://meta.fabricmc.net/v2/versions/loader/1.21.1",
                "https://bmclapi2.bangbang93.com/fabric-meta/v2/versions/loader/1.21.1",
            ),
            (
                "https://maven.fabricmc.net/net/fabricmc/fabric-loader/0.16.0/a.jar",
                "https://bmclapi2.bangbang93.com/maven/net/fabricmc/fabric-loader/0.16.0/a.jar",
            ),
            (
                "https://maven.minecraftforge.net/net/minecraftforge/forge/maven-metadata.xml",
                "https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/maven-metadata.xml",
            ),
            (
                "https://files.minecraftforge.net/maven/net/minecraftforge/forge/x.jar",
                "https://bmclapi2.bangbang93.com/maven/net/minecraftforge/forge/x.jar",
            ),
            (
                "https://maven.neoforged.net/releases/net/neoforged/neoforge/21.1.1/a.jar",
                "https://bmclapi2.bangbang93.com/maven/net/neoforged/neoforge/21.1.1/a.jar",
            ),
        ];
        for (from, to) in cases {
            assert_eq!(bmclapi_for(&u(from)).unwrap().as_str(), to, "{from}");
        }
        assert!(bmclapi_for(&u("https://api.modrinth.com/v2/project/x")).is_none());
        // Prefix must match on a segment boundary.
        assert!(bmclapi_for(&u("https://maven.neoforged.net/releasesX/a")).is_none());
        assert!(bmclapi_for(&u("https://files.minecraftforge.net/other/a")).is_none());
        // Modrinth/CurseForge have no mirrors: only `Route::Auto` applies.
        for host in [
            "api.modrinth.com",
            "cdn.modrinth.com",
            "api.curseforge.com",
            "edge.forgecdn.net",
            "mediafilez.forgecdn.net",
        ] {
            let orig = u(&format!("https://{host}/a/b.jar"));
            assert_eq!(candidates(&orig, true, true), vec![orig.clone()], "{host}");
        }
    }

    #[test]
    fn order_respects_preference() {
        let orig = u("https://piston-meta.mojang.com/a.json");
        let mirror = u("https://bmclapi2.bangbang93.com/a.json");
        assert_eq!(
            candidates(&orig, true, false),
            vec![orig.clone(), mirror.clone()]
        );
        assert_eq!(
            candidates(&orig, true, true),
            vec![mirror.clone(), orig.clone()]
        );
        assert_eq!(candidates(&orig, false, true), vec![orig.clone()]);
    }

    #[test]
    fn expand_dedups() {
        let a = u("https://piston-meta.mojang.com/a.json");
        let m = u("https://bmclapi2.bangbang93.com/a.json");
        assert_eq!(expand_all(&[a.clone(), m.clone()], true, false), vec![a, m]);
    }
}
