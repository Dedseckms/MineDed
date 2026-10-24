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

Все выполнены к 2026-10-24 (п. 2 и 4 — агентом с разрешения владельца).

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
   `docker compose` агенту недоступен — запуск выполняет владелец (`./start.sh`).
   Создан `.env` из `.env.example`, архив загрузки — `backups/upload-20261022-031952.tar`.
3. Smoke-тест бинаря без Docker: `timeout 15 ./hub/hub` на `127.0.0.1:18081` (процесс
   завершился сам, порт закрыт): health 200; create с чужим токеном → 401 `authExpired`
   (запрос реально ушёл в Drasl); join `AAAA-AAAA` → 404; ws с плохим токеном → 401.
   После: mihomo/caddy active, Drasl 200.

### 2026-10-24 — запуск, Caddy, QAD, зеркало обновлений (server-ops)
Владелец: DNS `relay.dedseckms.ru` готов, проброс UDP 7842 → 192.168.31.83 сделан; импорт
в Caddyfile и reload Caddy разрешены агенту.
1. `./start.sh` (QAD=0): поднялись `mineup-relay-hub`, `mineup-relay-iroh` (healthy).
2. Caddy: бэкап `/etc/caddy/Caddyfile.bak-mineup-relay-20261024-100230`, в конец добавлена
   строка `import /opt/relay/caddy/*.caddy`, `caddy validate` OK, `systemctl reload caddy`.
   Серт выпущен; `https://relay.dedseckms.ru/api/v1/health`, `/ping`, `/healthz` → 200.
3. `.env`: `QAD=1` (бэкап `backups/env-*.bak`), `./start.sh`: `0.0.0.0:7842/udp`
   слушает, в логе relay `QUIC server listening on 0.0.0.0:7842`, `Reloaded the certificate`.
4. Зеркало обновлений: загрузка `backups/upload-20261024-100813.tar` (прежние файлы —
   `backups/relay-20261024-100813.tar.gz`), в `.env` добавлены `GITHUB_REPO=Dedseckms/MineUP`,
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
