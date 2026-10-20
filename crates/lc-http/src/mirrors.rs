//! Mirror expansion for official Mojang hosts.

use url::Url;

/// BMCLAPI root (https://bmclapidoc.bangbang93.com).
pub const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";

/// Maps an official Mojang URL onto BMCLAPI, or `None` for other hosts.
pub fn bmclapi_for(original: &Url) -> Option<Url> {
    let prefix = match original.host_str()? {
        "piston-meta.mojang.com"
        | "launchermeta.mojang.com"
        | "piston-data.mojang.com"
        | "launcher.mojang.com" => "",
        "libraries.minecraft.net" => "/maven",
        "resources.download.minecraft.net" => "/assets",
        _ => return None,
    };
    let mut url = Url::parse(&format!("{BMCLAPI}{prefix}{}", original.path())).ok()?;
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
        ];
        for (from, to) in cases {
            assert_eq!(bmclapi_for(&u(from)).unwrap().as_str(), to, "{from}");
        }
        assert!(bmclapi_for(&u("https://api.modrinth.com/v2/project/x")).is_none());
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
