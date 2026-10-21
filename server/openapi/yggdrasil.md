# Yggdrasil API (Drasl)

CONTRACT (lead). Auth, profiles and textures are served by **Drasl** (self-hosted, Docker),
not by `hub`. The launcher talks to it through authlib-injector conventions.

- Spec: authlib-injector "Yggdrasil server technical specification"
  (https://github.com/yushijinhun/authlib-injector/wiki).
- Drasl: https://github.com/unmojang/drasl — verify the exact endpoint list against the
  deployed version during wave 0.

## Deployed instance (Drasl 4.0.1)

- Public: `https://skins.dedseckms.ru` (after DNS + Caddy import); on the host: `127.0.0.1:25585`.
- Passwords: scrypt (Drasl has no argon2) — accepted by lead. Registration: invite-only.
- Admin API: `/drasl/api/v3` (v2 removed in Drasl 4).

## API root and discovery

- API root: `https://<domain>/authlib-injector` — all Yggdrasil paths below are relative to it.
- Drasl sends `X-Authlib-Injector-API-Location`, so users can type just the domain (ALI discovery).
- Launch: `-javaagent:authlib-injector.jar=<api root>` plus
  `-Dauthlibinjector.yggdrasil.prefetched=<base64 of GET <api root>>`.

## Endpoints the launcher uses

| Purpose | Method + path |
|---|---|
| Metadata + signature public key | `GET /` |
| Login | `POST /authserver/authenticate` |
| Refresh | `POST /authserver/refresh` |
| Validate | `POST /authserver/validate` |
| Logout | `POST /authserver/invalidate`, `POST /authserver/signout` |
| Join server (client) | `POST /sessionserver/session/minecraft/join` |
| Has joined (server) | `GET /sessionserver/session/minecraft/hasJoined` |
| Profile + signed textures | `GET /sessionserver/session/minecraft/profile/{uuid}?unsigned=false` |
| Names → profiles | `POST /api/profiles/minecraft` |

## Required Drasl settings (server-ops)

- Fallback API servers: not needed (the launcher has no Microsoft accounts).
- Texture signing key persisted in a volume (rotating it breaks cached skins).
- Registration policy: decided by the owner (invite-only recommended).
- Skin library uploads come from `hub` via Drasl's admin API (token kept on the server only).

## Other Yggdrasil servers

The same code path supports any authlib-injector server, e.g. Ely.by
(`https://authserver.ely.by/api/authlib-injector`). TLauncher accounts are **not** supported:
no public API (see docs/PLAN.md, risks).
