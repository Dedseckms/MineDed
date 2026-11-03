# Relay stack (iroh-relay + hub + update mirror) — /opt/relay

Журнал деплоя и эксплуатации. Секретов здесь нет (у стека их нет вовсе: hub проверяет
токены через Drasl, relay открыт всем с лимитами). Копия в репозитории: `server/relay/`.

## Что развёрнуто

| | |
|---|---|
| Compose-проект | `mineup-relay` (`compose.yaml`, опционально `compose.qad.yaml`) |
| hub | `mineup/hub:0.1.0` = busybox 1.37.0-musl + статический бинарь `hub` (musl, собирается на машине разработчика, хост Rust не компилирует). Контейнер `mineup-relay-hub` |
| iroh-relay | `n0computer/iroh-relay:v1.4.1` (alpine). Контейнер `mineup-relay-iroh` |
| Сети | `mineup-relay_relay_net` bridge `172.30.43.0/24` (`RELAY_SUBNET`); hub также в `mineup-skin_skin_net` (наша сеть skinserver) — только чтобы достучаться до Drasl `http://mineup-skin-drasl:25585` |
| Порты | `127.0.0.1:18080` → hub 8080 (`HUB_PORT`); `127.0.0.1:13340` → relay 3340 (`RELAY_HTTP_PORT`); `127.0.0.1:18090` → updates-web 8080 (`UPDATES_PORT`); при `QAD=1`: `0.0.0.0:7842/udp` (`QAD_PORT`) |
| Volume | mineup-relay_updates_data — только зеркало релизов (перекачивается, бэкап не нужен). hub и relay без состояния (комнаты в памяти hub) |
| updates | mineup/updates-mirror:0.1.0 (alpine 3.20.3 + curl + jq, updates/mirror.sh), usybox:1.37.0-musl httpd. Контейнеры mineup-relay-updates-mirror, mineup-relay-updates-web |
| Сайт | `mineded.dedseckms.ru` → `mineup-relay-site` (busybox httpd, `./site` ro, `127.0.0.1:18091` = `SITE_PORT`, 32 MiB / 0.2 CPU), `caddy/site.caddy` с CSP/HSTS/nosniff/X-Frame-Options |
| Домен | `relay.dedseckms.ru`: `/api/v1/*` → hub, `/updates/*` → updates-web, остальное → iroh-relay (`caddy/relay.caddy`) |

Изоляция и лимиты: оба контейнера `user 65534`, `cap_drop: [ALL]`, `no-new-privileges`,
`read_only`, без `privileged`/`cap_add`/`network_mode: host`. hub: 64 MiB, 0.2 CPU; relay:
128 MiB, 0.4 CPU; `cpu_shares 256`, `pids_limit 64`, логи `json-file` 10m × 3. Healthcheck:
`wget /api/v1/health` и `wget /healthz` раз в 30 с.

## hub (rooms API, `server/openapi/hub.yaml`)

- `POST /api/v1/rooms` — Bearer обязателен, проверка `POST /authserver/validate` в Drasl
  (+ `GET /minecraftservices/minecraft/profile` для id/имени), кэш 60 с.
- `POST /api/v1/rooms/{code}/join` — публичный для `authMode=open`, Bearer для `yggdrasil`.
  Считаются только живые участники (WS открыт или ≤ 60 с после join без WS); `peerToken` =
  `<exp>.<uuid|->.<HMAC-SHA256(hostToken, "lc-peer/2|…")>`, живёт 10 мин; uuid — только из
  проверенного Bearer.
- `POST /api/v1/rooms` без Bearer — только `authMode=open` (офлайн-хост): ≤ 2 активных
  комнаты и ≤ 10 созданий/ч на IP, закрытие — по `hostToken`.
- `DELETE /api/v1/rooms/{roomId}` — Bearer = Yggdrasil-токен владельца или `hostToken`.
- `GET /api/v1/rooms/{roomId}/ws?token=` — presence + fallback-чат; закрытие WS, `leave` или
  `kick` от хоста освобождают место.
- TTL комнаты 6 ч (`HUB_ROOM_TTL_SECS`), продлевается, пока хост подключён к WS;
  лимиты: 200 комнат всего, 3 на пользователя.
- Rate limit по IP (из `X-Real-IP`, его перезаписывает Caddy): create 6/мин (burst 5),
  join 20/мин (burst 10), ws 60/мин (burst 20), delete 30/мин; чат 60 сообщений/мин на сокет.
  429 с кодом `rateLimited` + `Retry-After`. `/mirrors`, `/skins*` пока 501.

## Зеркало обновлений (updates)

- `updates-mirror` раз в `UPDATES_INTERVAL_SECS` (1800 с) берёт
  `api.github.com/repos/$GITHUB_REPO/releases/latest` (без токена), качает ассеты в том
  (проверка размера и `digest` sha256, если GitHub его отдаёт), затем публикует
  `latest.json` с URL платформ, переписанными на `https://relay.dedseckms.ru/updates/<file>`
  (если файл не скачан — `latest.json` не обновляется), и удаляет файлы старых релизов.
  Пустой `GITHUB_REPO` или 404 = idle. Лимит релиза `UPDATES_MAX_TOTAL_MB` (1024).
- `status.json` (`repo`, `state` idle/ok/error, `tag`, `checkedAt`) обновляется каждый
  проход; healthcheck mirror — его свежесть (< 70 мин), web — `wget /status.json`.
- `updates-web`: busybox httpd, том смонтирован `ro`; Caddy пропускает только GET/HEAD,
  `latest.json`/`status.json` с `Cache-Control: no-cache`.
- Лимиты: mirror 64 MiB / 0.2 CPU, web 32 MiB / 0.2 CPU, `pids_limit 32`, логи 10m × 3.
- Принудительная синхронизация: `sudo docker compose -p mineup-relay restart updates-mirror`.
- После изменения `caddy/relay.caddy`: `sudo caddy validate --config /etc/caddy/Caddyfile`,
  `sudo systemctl reload caddy` (только reload).

## iroh-relay

- Plain HTTP за Caddy; клиенты: `https://relay.dedseckms.ru` (WebSocket `/relay`).
- `access = everyone`, лимиты: 5 новых соединений/с (burst 20), 2 MiB/с на клиента (burst 4 MiB).
- Метрики выключены (порт 9090 занят на хосте и не нужен).
- QAD (QUIC address discovery, нужен для hole punching) — UDP **7842**. Требует TLS-серт
  `relay.dedseckms.ru`: используем серт хостового Caddy (bind mount ro каталога
  `/var/lib/caddy/.local/share/caddy/certificates/acme-v02.api.letsencrypt.org-directory/relay.dedseckms.ru`,
  контейнер под uid/gid caddy 996:988 без capabilities, `cert_mode = Reloading` — подхватывает
  продление). Флаг `--dev` здесь означает только «relay по HTTP» (TLS для HTTP в Caddy).

## Запросы владельцу

Все выполнены к 2026-10-24 (п. 2 и 4 — с разрешения владельца).

1. DNS: A-запись `relay.dedseckms.ru` → 62.182.138.63.
2. Caddy: в `/etc/caddy/Caddyfile` рядом с импортом skinserver добавить
   `import /opt/relay/caddy/*.caddy`, затем
   `sudo cp /etc/caddy/Caddyfile /etc/caddy/Caddyfile.bak-<дата>`,
   `sudo caddy validate --config /etc/caddy/Caddyfile`, `sudo systemctl reload caddy`.
3. Роутер: проброс **UDP 7842** → 192.168.31.83 (QAD). TCP ничего нового не нужно (443 уже).
4. После 1–3 и выпуска серта Caddy: в `/opt/relay/.env` `QAD=1`, `./start.sh`.

## Эксплуатация

```bash
cd /opt/relay
./start.sh                                   # проверка портов/сети/серта + up -d --build
./check.sh                                   # health, relay, 404/401, соседи
TEST_USER=mu_tester ./check.sh               # + создать/войти/закрыть тестовую комнату
sudo docker compose -p mineup-relay ps
sudo docker compose -p mineup-relay logs --tail 100 hub iroh-relay
sudo docker compose -p mineup-relay restart hub
```

Деплой новой версии с машины разработчика: `powershell -File server/relay/deploy.ps1`
(собирает musl-бинарь, загружает, `install.sh` архивирует прежние файлы в
`backups/relay-<UTC>.tar.gz`, затем `start.sh` + `check.sh`).

## Откат

```bash
cd /opt/relay
sudo docker compose -p mineup-relay down                # снять стек (сети проекта удалятся, тома нет)
tar -xzf backups/relay-<дата>.tar.gz                    # вернуть прежние файлы
./start.sh
```

Полное удаление стека — только `down`; образы `mineup/hub`, `n0computer/iroh-relay` удалять
с разрешения владельца.

## mihomo

TUN в mihomo нет (проверено 2026-10-21), трафик контейнеров не перехватывается. Если владелец
включит TUN с `auto-route`, добавить `route-exclude-address: [172.30.43.0/24]`.

## Журнал

### 2026-10-22 — подготовка (server-ops)
1. Read-only разведка: свободны tcp 18080, 13340, udp 7842; `/opt/relay` пуст, принадлежит
   dedseckms; Caddy 2.6.2, импорт `/opt/skinserver/caddy/*.caddy` уже есть; DNS
   `relay.dedseckms.ru` ещё нет; Drasl отвечает на `127.0.0.1:25585` (validate 403 на чужой
   токен, profile 401 — маршруты подтверждены).
2. Файлы загружены в `/opt/relay` (без sudo). Контейнеры **не запускались**: sudo для
   `docker compose` удалённо недоступен — запуск выполняет владелец (`./start.sh`).
   Создан `.env` из `.env.example`, архив загрузки — `backups/upload-20261022-031952.tar`.
3. Smoke-тест бинаря без Docker: `timeout 15 ./hub/hub` на `127.0.0.1:18081` (процесс
   завершился сам, порт закрыт): health 200; create с чужим токеном → 401 `authExpired`
   (запрос реально ушёл в Drasl); join `AAAA-AAAA` → 404; ws с плохим токеном → 401.
   После: mihomo/caddy active, Drasl 200.

### 2026-10-24 — запуск, Caddy, QAD, зеркало обновлений (server-ops)
Владелец: DNS `relay.dedseckms.ru` готов, проброс UDP 7842 → 192.168.31.83 сделан; импорт
в Caddyfile и reload Caddy разрешены оператору.
1. `./start.sh` (QAD=0): поднялись `mineup-relay-hub`, `mineup-relay-iroh` (healthy).
2. Caddy: бэкап `/etc/caddy/Caddyfile.bak-mineup-relay-20261024-100230`, в конец добавлена
   строка `import /opt/relay/caddy/*.caddy`, `caddy validate` OK, `systemctl reload caddy`.
   Серт выпущен; `https://relay.dedseckms.ru/api/v1/health`, `/ping`, `/healthz` → 200.
3. `.env`: `QAD=1` (бэкап `backups/env-*.bak`), `./start.sh`: `0.0.0.0:7842/udp`
   слушает, в логе relay `QUIC server listening on 0.0.0.0:7842`, `Reloaded the certificate`.
4. Зеркало обновлений: загрузка `backups/upload-20261024-100813.tar` (прежние файлы —
   `backups/relay-20261024-100813.tar.gz`), в `.env` добавлены `GITHUB_REPO=Dedseckms/MineDed`,
   `UPDATES_*`; `./start.sh` создал том `mineup-relay_updates_data` и контейнеры
   `updates-mirror`/`updates-web` (healthy). Caddy: бэкап
   `Caddyfile.bak-mineup-updates-*`, validate, reload. Репозиторий пока отдаёт 404 (релиза нет
   или репо приватный) → `state: idle`. Функциональный тест `mirror.sh` на `jqlang/jq` в
   одноразовом контейнере (tmpfs, `--rm`): 30 ассетов скачаны с проверкой digest, `state: ok`,
   переписывание URL в `latest.json` проверено.
5. Публично: `/updates/status.json` 200, POST 405, несуществующий файл 404.
   После: mihomo/caddy active, Drasl 200, чужие контейнеры не тронуты.

Откат зеркала: `sudo docker compose -p mineup-relay stop updates-mirror updates-web` и убрать
блок `handle_path /updates/*` из `caddy/relay.caddy` + validate/reload. Откат Caddy целиком:
`sudo cp /etc/caddy/Caddyfile.bak-mineup-relay-20261024-100230 /etc/caddy/Caddyfile` + reload.

### 2026-10-24 — hub 1.0.0
deploy.ps1 не загрузил бинарь (`hub/hub` оставался от 2026-10-22). Загружен musl-бинарь 1.0.0
(sha256 `d9777f94…d58a`), прежний сохранён в `backups/hub-prev-20261024-154153`, `./start.sh`.
Health `{"status":"ok","version":"1.0.0"}`; анонимный open-room create → 201, DELETE по hostToken → 204.
mihomo/caddy active, Drasl 200. Тег образа остаётся `mineup/hub:0.1.0` (`HUB_VERSION` в .env не задан).
Откат: вернуть `backups/hub-prev-*` в `hub/hub` и `./start.sh`.

### 2026-10-25 — сайт mineded.dedseckms.ru (server-ops, с разрешения владельца)
Файлы `site/` (main 578b5ca, без README) → `/opt/relay/site`; загрузка
`backups/upload-20261024-220240.tar`, прежние файлы `backups/relay-20261024-220240.tar.gz`,
`.env` (+`SITE_PORT=18091`) → `backups/env-site.bak`. `./start.sh` поднял `mineup-relay-site`
(healthy). Caddy: бэкап `Caddyfile.bak-mineup-site-*`, новый `caddy/site.caddy` подхвачен
существующим импортом, validate OK, reload. `https://mineded.dedseckms.ru/` → 200 text/html,
заголовки безопасности на месте, README.md → 404. hub/updates/Drasl 200, mihomo/caddy active.
Откат: `sudo docker compose -p mineup-relay stop site`, переименовать `caddy/site.caddy` в
`site.caddy.off` + validate/reload.

### 2026-10-26 — moved to VPS 31.59.192.133 (server-ops)
Full log: /opt/DEPLOY.md (repo: server/deploy/vps/DEPLOY.md), section Phase 2.

## 2026-10-26: MineDed VPN route
relay.caddy now has \import /opt/vpn/caddy/vpn.caddy\ inside the relay.dedseckms.ru block (s-ui panel + subscriptions on secret paths). Backup: /opt/vpn/backups/relay.caddy.20261026-085642. Details: /opt/vpn/DEPLOY.md.

### 2026-10-26 — hub /me/vpn (server-ops)
hub (musl, sha256 `be745282…2929`) with `/me/vpn` GET/POST: one s-ui client per profile
(remark `mc-<uuid>`, random name = subscription secret, 256 GiB, inbounds 1+2).
New `compose.vpn.yaml` (start.sh adds it when `HUB_VPN_API_URL` is set): hub joins the external
network `mineded-vpn_default` and calls `http://mineded-vpn:2095/<panel>/apiv2`; s-ui ports unchanged
(127.0.0.1 only). `.env` += `HUB_VPN_API_URL`, `HUB_VPN_API_TOKEN`, `HUB_VPN_SUB_BASE`,
`HUB_VPN_QUOTA_GB` (from /opt/vpn/.secrets). Backups: `backups/env-vpn-20261026-094048.bak`,
`backups/hub-prev-20261026-094048`, `backups/start.sh.20261026-094048`.
Checks: health 200 (local + public, 1.0.7), POST /me/vpn without auth 401; E2E with the Drasl test
user: GET 404 -> POST 200 (quota 274877906944, enabled, resetAt 2026-10-31T21:00:00Z) -> POST again
same URL -> subscription 200 (hysteria2 + vless) -> test client deleted in s-ui (GET 404, sub 400),
token invalidated. All containers healthy.
Rollback: restore the three backups (`.env`, `hub/hub`, `start.sh`), `./start.sh`
(without `HUB_VPN_API_URL` the vpn override is not applied and `/me/vpn` returns 503).

## 20261026-125415: site synced from main (excl. README.md, og.html); backup backups/site-20261026-125415.tgz

## 20261026-193147: site synced from main; backup backups/site-20261026-193147.tgz

### 2026-10-27 - join/invite page (server-ops)
Added site/join.html, site/join.js; caddy/site.caddy now rewrites /join/* and /invite/* to /join.html. Backup: /opt/relay/backups/site.caddy.20261027-join. validate+reload OK; /join/X 200, / 200, mineup 301. Rollback: cp backup back to caddy/site.caddy + systemctl reload caddy.

## 2026-10-27 security headers
- Added HSTS, nosniff, X-Frame-Options DENY, Referrer-Policy, -Server to the site block. Backup: *.caddy.bak-20261027-060540. Rollback: restore backup, caddy validate, systemctl reload caddy.

## 2026-10-30 - public static site without VPN promotion

- Scope: existing `/opt/relay/site` bind mount only. No hub/chat deployment,
  container restart, daemon operation, network change, or home-server access.
- Preflight: canonical path `/opt/relay/site`; existing site mount verified;
  no Git repository in relay/site. Site, hub and Drasl checks passed before changes.
- Source: Git archive of `site/` from repository commit
  `7025e9a908ef947930b6f2d30108233c76eeaf7b` (site content change `719bacb`).
  README and unused VPN screenshots excluded. Existing search verification files retained.
- Backup: `/opt/relay/backups/site-before-20261030-033602.tar.gz`.
- Git snapshots: `/opt/relay/backups/site-snapshot-20261030-033602.git`
  (outside the public web root): before `bbd7738`, deployed `a77f99c`; clean worktree.
- Uploaded archive: `/opt/relay/backups/site-upload-20261030-033602.tar`;
  SHA-256 `2620f18d12829cd6936a172042cef57fb1dc15b492f1f51d235d7c0a9c5b51d9` verified.
- Checks: no VPN text in served HTML/JS sources; public root, install, privacy,
  share-image HTML, app.js?v=10, og.png and play screenshot all HTTP 200.
  Public index/app.js SHA-256 match deployed files. Hub health 200/status ok
  (existing version 1.0.7); Drasl authlib-injector metadata HTTP 200.
- Rollback if authorized: from `/opt/relay`, extract the pre-change tar archive
  over `site/` using `tar -xzf backups/site-before-20261030-033602.tar.gz
  --no-same-owner --no-same-permissions`, then repeat public/health checks.
  No service restart is necessary. New unreferenced screenshot files may remain.

## 2026-10-30 — hub 1.3.0 on the VPS

- Image `mineup/hub:1.3.0` built on the VPS from repository commit `1c9ffe3`
  (merged to `main` as `5d54e42`, tag `v1.3.0`). Static musl binary built in
  `rust:1.99.1-bookworm` (`/opt/relay/hub-1.3/build-hub.sh`).
- Backups: `/opt/relay/backups/20261030-hub13/` (env, compose, DEPLOY.md) and
  `/opt/relay/backups/hub-dir-20261030-hub13.tar.gz`; previous image
  `mineup/hub:0.1.0` kept for rollback; previous hub directory left as
  `/opt/relay/hub.bak-20261030`.
- Changed: `hub/hub` + `hub/Dockerfile` replaced; `compose.yaml` now passes
  `CURSEFORGE_API_KEY` and chat storage; `HUB_VERSION=1.3.0` and the optional
  catalogue and Drasl admin credentials live in `.env` (mode 600, never printed).
- Restart: `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml
  -f compose.vpn.yaml up -d hub` — only the hub container was recreated.
- Checks: `/api/v1/health` 200 `{"status":"ok","version":"1.3.0"}`;
  `/api/v1/chat/me` and `/api/v1/chat/channels` 401 without a token;
  `/api/v1/catalog/curseforge/mods/search` 200; `authlib-injector` 200;
  `/api/v1/rooms` 405 for GET (rooms are POST); hub logs clean.
- Rollback: set `HUB_VERSION=0.1.0` in `.env` (or restore the backup) and run the
  same `up -d hub`; the old image is local, no rebuild needed.

## 2026-11-03 — hub 1.5.31 (server-ops)

- Source: `git archive` of repository `main` at `17cd2c7` (workspace version 1.5.31), uploaded to
  `/opt/relay/hub-1.3/ws-1.5.31.tgz` (sha256 `47ca7fd0…72af`), extracted over
  `/opt/relay/hub-1.3/ws`. `server/hub` code unchanged in this batch, only the version number.
- Build: `./build-hub.sh` (throwaway `rust:1.99.1-bookworm`), 2m38s on 1 vCPU. The earlier 1.5.3
  attempt (`build14.log`) failed on `--locked`: `Cargo.lock` was stale; the 1.5.31 archive carries
  a matching lock file.
- Binary: `/opt/relay/hub-1.3/hub` sha256 `5f55966f…7a80` → copied to `/opt/relay/hub/hub`.
- Backups: `/opt/relay/backups/hub-1.5.31-20261103-110608/` (previous binary `hub-prev`
  sha256 `0243049a…9f12` + Dockerfile); previous image kept as `mineup/hub:prev-20261103-110608`.
- Image: `mineup/hub:1.3.0` rebuilt; the tag is unchanged by design (`HUB_VERSION` in `.env`
  untouched), the reported version comes from the binary.
- Smoke test: throwaway container on `127.0.0.1:18081` answered
  `{"status":"ok","version":"1.5.31"}`, then removed.
- Restart: `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml -f compose.vpn.yaml
  up -d --force-recreate hub` — only the hub container was recreated (restart count 0).
- Checks: public `/api/v1/health` `{"status":"ok","version":"1.5.31"}` (repeated three times);
  `/api/v1/presence` 200 `{"online":1}`; `/api/v1/chat/me` 401 without a token; `GET /api/v1/rooms`
  405 (rooms are POST); Drasl `127.0.0.1:25585/authlib-injector` 200; hub container healthy; caddy
  active; drasl, iroh-relay, updates, site, tg-bot and s-ui containers untouched (7–8 days uptime).
- Rollback: `cp /opt/relay/backups/hub-1.5.31-20261103-110608/hub-prev /opt/relay/hub/hub` then
  `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml -f compose.vpn.yaml
  up -d --build hub`, or `docker tag mineup/hub:prev-20261103-110608 mineup/hub:1.3.0` followed by
  `up -d hub` (no rebuild).

## 2026-11-03 — hub 1.5.33 (server-ops)

First release where `server/hub` code changed: approving an account request creates the Drasl
account, the request password is stored sealed (`users.request_secret`, XChaCha20-Poly1305, key
in the `settings` row `request-key`), new `POST /chat/admin/reject/{uuid}`, `mutedUntil`/`banned`
in `/chat/admin/accounts`, `login` in `/chat/admin/requests`. `chacha20poly1305` is new in
`Cargo.lock`.

- Backup first, because of the additive schema migration: `/opt/relay/backups/hub-1.5.33-20261103-104713/`
  (mode 700): `chat.sqlite3` from the SQLite online backup API (`PRAGMA integrity_check` = ok,
  17 users, 1 open request), `files.tar.gz` (42 entries), previous binary `hub-prev`
  (sha256 `5f55966f…7a80`), `Dockerfile`, `image-prev-id.txt`. Previous image kept as
  `mineup/hub:prev-20261103-104713`; previous binary also as `/opt/relay/hub/hub.prev-1.5.31`.
- Source: `git archive` of `main` at `b8d2af6` (1.5.33) -> `ws-1.5.33.tgz`
  (sha256 `70549a68…ad12`), extracted over `/opt/relay/hub-1.3/ws`; only five stale frontend
  files remained, none in the hub crate. `Cargo.lock` carries `chacha20poly1305`, so `--locked`
  passes.
- Build: `./build-hub.sh` (throwaway `rust:1.99.1-bookworm`), 2m54s, log `build-1.5.33.log`.
  Binary `/opt/relay/hub-1.3/hub` sha256 `e8b8dc1c…836c`, static-pie musl -> `/opt/relay/hub/hub`.
- Image `mineup/hub:1.3.0` rebuilt (`HUB_VERSION` untouched by design; the version comes from the
  binary), new id `sha256:233c0bad…4bde7`.
- Smoke test before the switch, on a throwaway volume holding a copy of the backup DB: health
  `{"status":"ok","version":"1.5.33"}`, `/presence` 200, `/chat/me` and `/chat/admin/requests` 401,
  no panic. A plain `busybox` container is NOT a valid smoke host for this binary: without the
  image's `/etc/ssl/cert.pem` the reqwest client cannot be built and startup fails with
  "could not configure catalogue broker".
- Restart: `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml -f compose.vpn.yaml
  up -d --force-recreate hub` — only hub was recreated (restarts 0), the other seven containers
  kept their uptime.
- Checks: public `/api/v1/health` `{"status":"ok","version":"1.5.33"}`, `/presence` 200,
  `/chat/admin/accounts` and `/chat/admin/requests` 401 without a token, Drasl
  `https://skins.dedseckms.ru/authlib-injector` 200, hub healthy, caddy active. Logs: `vpn: on`,
  `hub 1.5.33 listening on 0.0.0.0:8080`, chat sockets opening, no error/panic/migration lines.
- Migration verified on a copy of the live DB: `PRAGMA integrity_check` ok, `journal_mode` wal,
  column `users.request_secret` present, 17 users, 58 messages, 8 audit rows. The `request-key`
  settings row is created on the first account request, so it is absent until then.
- Live traffic confirmed the new endpoints: an admin `rejectAccount` audit entry appeared at
  2026-11-03T11:06:58Z; the open request from before the deploy was handled, so
  `SELECT count(*) FROM users WHERE request_password IS NOT NULL` is 0 now.
- Rollback: `cp backups/hub-1.5.33-20261103-104713/hub-prev hub/hub` then
  `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml -f compose.vpn.yaml up -d
  --build hub`; or `docker tag mineup/hub:prev-20261103-104713 mineup/hub:1.3.0` + `up -d hub`
  (no rebuild). The schema change is additive and nullable, so 1.5.31 runs on the migrated DB;
  the chat volume is never touched. Restoring `chat.sqlite3` from the backup is only needed if
  the DB itself is damaged.


## 2026-11-03 — hub 1.6.0 (server-ops)

Ban appeals for guests and accounts: `POST /chat/appeal`, `GET /chat/admin/appeals`,
`POST /chat/admin/appeals/{uuid}/accept|dismiss`. Also new here: `POST /chat/admin/reject/{uuid}`,
`DELETE /chat/admin/accounts/{uuid}`, the account-request password sealed with XChaCha20-Poly1305 into
`users.request_secret` (key in the `settings` row `request-key`, created on the first account request),
a ban now closes the member's WebSocket, and `guest_identity` no longer refuses a banned guest (the
client needs an identity to render the ban/appeal screen). New table `appeals`.

- Backup first, because of the schema migration:
  `/opt/relay/backups/hub-1.6.0-20261103-225601/` (mode 700): `chat.sqlite3` from the SQLite online
  backup API (`PRAGMA integrity_check` = ok, 19 users, 62 messages, 15 audit rows), `files.tar.gz`,
  previous binary `hub-prev` (sha256 `e8b8dc1c…836c`), `Dockerfile`, `image-prev-id.txt`.
  Previous image kept as `mineup/hub:prev-20261103-225601`.
- Source: `git archive` of `main` at `3e7409f` (1.6.0) -> `ws-1.6.0.tgz`
  (sha256 `ab76b5fad68e4ec49003deda071d3ab55b386a7d0649b3776077c88c43e8e063`), extracted over
  `/opt/relay/hub-1.3/ws`; five stale frontend files removed afterwards, none in `server/hub`.
- Build: `./build-hub.sh` (incremental, 2m15s), log `build-1.6.0.log`. Binary
  `/opt/relay/hub-1.3/hub` sha256 `40ecf20aa8fb33ea4654477461d077f6c3782fc143b7ca415e9ab13ec5c85812`,
  static-pie musl -> `/opt/relay/hub/hub`. Image `mineup/hub:1.3.0` rebuilt, new id
  `sha256:685b084f…` (`HUB_VERSION` untouched by design; the version comes from the binary).
- Smoke test before the switch, on a throwaway volume holding a copy of the backup DB and files:
  health `{"status":"ok","version":"1.6.0"}`, `/presence` 200, `401` without a token for `/chat/me`,
  `/chat/appeal`, `/chat/admin/appeals` and `/chat/admin/reject/<uuid>` (so the new routes exist,
  they are not 404), guest identity 200, no panic. Migration on that copy: `integrity_check` ok,
  table `appeals` present, columns `users.request_secret` and `users.username_hint` present,
  19 users / 62 messages unchanged. Throwaway container and volume removed afterwards.
- Restart: `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml -f compose.vpn.yaml
  up -d --force-recreate hub` — only hub was recreated, the other seven containers kept their uptime.
- Checks: local and public `/api/v1/health` `{"status":"ok","version":"1.6.0"}`, `/presence` 200,
  `/chat/appeal` 401, Drasl `https://skins.dedseckms.ru/authlib-injector` 200, hub healthy, caddy
  active. Logs: `vpn: on`, `hub 1.6.0 listening on 0.0.0.0:8080`, chat sockets opening, no
  error/panic/migration lines.
- Live DB after the migration: `PRAGMA integrity_check` ok, `journal_mode` wal, table `appeals`
  present (0 rows), `users.request_secret` and `users.username_hint` present, 19 users / 62 messages /
  15 audit rows (unchanged by the migration).
- Rollback: `cp backups/hub-1.6.0-20261103-225601/hub-prev hub/hub` then
  `docker compose -p mineup-relay -f compose.yaml -f compose.qad.yaml -f compose.vpn.yaml up -d
  --build hub`; or `docker tag mineup/hub:prev-20261103-225601 mineup/hub:1.3.0` + `up -d hub`
  (no rebuild). The schema change is additive (a new table and nullable columns), so 1.5.33 runs on
  the migrated DB; the chat volume is never touched. Restoring `chat.sqlite3` from the backup is only
  needed if the DB itself is damaged.
