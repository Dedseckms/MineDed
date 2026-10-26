# MineDed web theme for Drasl 4.0.1

Makes the skin server UI (https://skins.dedseckms.ru) look like the launcher and
the site: voxel backgrounds with drifting clouds, translucent panels, launcher
colours/radius/fonts, MineDed logo, dark/light by `prefers-color-scheme`,
animations off with `prefers-reduced-motion`.

## How Drasl loads it

Drasl reads templates (`view/*.tmpl`), static files (`public/`, served at
`/web/public`), `locales/` and `assets/` from its **DataDirectory**. In the
Nix-built image that is `/nix/store/<hash>-drasl-4.0.1/share/drasl`; the default
can be overridden with the env var `DRASL_DEFAULT_DATA_DIRECTORY` (no
`config.toml` change).

`../Dockerfile` copies the upstream data dir out of the image, checks the files
we replace against `upstream.sha256` (build fails if upstream changed them),
overlays this folder and installs the result at `/usr/share/mineded-drasl`,
then sets `DRASL_DEFAULT_DATA_DIRECTORY` to it. Baked into the image, so it
survives restarts and works with the read-only rootfs. No volume mount.

## What is overridden

| File | Change |
|---|---|
| `view/layout.tmpl` | Theme CSS/JS, colour-scheme meta, SVG favicon; Drasl's own background effect is not loaded. |
| `view/header.tmpl` | MineDed brand, "Log in" link, scene button; opens `<div class="card panel">`. Logout form unchanged. |
| `view/footer.tmpl` | Closes the card; launcher link; Drasl version/licence/source kept (GPL). |
| `view/root.tmpl` | Home page: hero ("Аккаунт MineDed"), login form (unchanged), registration and "Скачать MineDed" buttons; the whole client/server setup guide moved unchanged into a collapsed `<details>` at the bottom. |
| `view/registration.tmpl` | "Repeat password" field (no `name`, never sent; `theme.js` blocks submit on mismatch); with an invite: "Open in launcher" → `mineded://invite/<code>`. |
| `view/user.tmpl` | "Open launcher" (`mineded://open`, falls back to the download page after 1.5 s); per-player "Change skin and cape"; tokens and player UUIDs in collapsed "Advanced settings". |
| `view/player.tmpl` | Skin/cape upload up front; texture URLs, fallback player and IDs in collapsed "Advanced settings" (still inside the form). |
| `public/style.css` | Full replacement (keeps all functional classes: previews, profile pictures, honeypot, noscript). |
| `public/logo.svg` | MineDed logo (copy of `site/logo.svg`). |
| `public/scenes.js` | Copy of `site/scenes.js` — keep in sync. |
| `public/theme.js` | Paints the scene, follows colour scheme, scene button; launcher deep links (+ "Open in launcher" next to invite links, e.g. in admin), password confirmation. |

Other page templates (`admin`, …) are **not** touched. In the overridden ones
forms, field names and invite-only registration stay upstream.
Texts are upstream i18n (`locales/ru` by `Accept-Language`); our few added
strings switch on `.L.GetLanguage`.

## Deploy (server-ops)

```sh
./start.sh   # docker compose -p mineup-skin up -d --build; image tag 4.0.1-hc-theme1
```

Config note: in `config/config.toml` set `ApplicationName = "MineDed Skins"`
(unset now, so pages say "Drasl") and make sure `InstanceName = "MineDed Skins"`
(replace any remaining "MineUP Skins" on the server).

Changing any theme file: bump `?v=` in `view/layout.tmpl`/`header.tmpl` and the
image tag in `compose.yaml`.

## Drasl upgrade

1. Bump `unmojang/drasl:<ver>` in `../Dockerfile`; build. If `sha256sum -c`
   fails, diff upstream `view/{layout,header,footer,root,registration,user,player}.tmpl` and `public/style.css`
   against the previous tag and port changes into these files.
2. Regenerate `upstream.sha256` from the new tag (`sha256sum view/layout.tmpl
   view/header.tmpl view/footer.tmpl view/root.tmpl view/registration.tmpl view/user.tmpl
   view/player.tmpl public/style.css public/logo.svg`).
3. Check login, registration with an invite link, profile, skin/cape upload,
   password change and `/web/admin` at desktop and phone widths.
