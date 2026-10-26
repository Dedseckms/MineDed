# MineDed website

Static site for <https://mineded.dedseckms.ru>. Plain HTML/CSS/JS, no build step,
no external fonts, CDNs or trackers.

- `index.html` — page; `style.css` — styles (tokens mirror the launcher, dark/light via `prefers-color-scheme`).
- `scenes.js` — voxel backgrounds ported from `apps/desktop/src/features/appearance/backgrounds.tsx`.
- `app.js` — RU/EN dictionary (auto from `navigator.language`, toggle remembered in `localStorage`),
  background switcher, download links resolved from the latest GitHub release
  (fallback: `https://github.com/Dedseckms/MineDed/releases/latest`).
- `screenshots/<screen>-<ru|en>.webp` — real UI screenshots (1600×1000) shown in a lazy gallery with a `<dialog>` lightbox.
  Regenerate: `pnpm --filter desktop dev`, then open `/?demo=1&lang=ru&bg=forest&section=<id>`
  (`&library=catalog`, `&instance=inst-fabric&tab=mods|browse`) in a 1600×1000 window, dark scheme.
- `logo.svg` — logo and favicon (copy of `apps/desktop/public/logo.svg`).

## Serve

Any static file server works:

```sh
python -m http.server -d site 8080      # local preview
```

Caddy:

```caddy
mineded.dedseckms.ru {
	root * /srv/mineup-site
	encode zstd gzip
	header {
		Content-Security-Policy "default-src 'self'; connect-src 'self' https://api.github.com; img-src 'self' data:; style-src 'self'; script-src 'self'"
		X-Content-Type-Options nosniff
		Referrer-Policy strict-origin-when-cross-origin
	}
	@assets path *.css *.js *.svg *.webp
	header @assets Cache-Control "public, max-age=86400"
	file_server
}
```
- `og.png` — 1200×630 social preview rendered from `og.html` (command in its header comment); `robots.txt`, `sitemap.xml` — SEO (update `lastmod` on content changes).
