# Relay stack (iroh-relay + hub) — /opt/relay

Журнал деплоя и эксплуатации. Секретов здесь нет (у стека их нет вовсе: hub проверяет
токены через Drasl, relay открыт всем с лимитами). Копия в репозитории: `server/relay/`.

## Что развёрнуто

| | |
|---|---|
| Compose-проект | `mineup-relay` (`compose.yaml`, опционально `compose.qad.yaml`) |
| hub | `mineup/hub:0.1.0` = busybox 1.37.0-musl + статический бинарь `hub` (musl, собирается на машине разработчика, хост Rust не компилирует). Контейнер `mineup-relay-hub` |
| iroh-relay | `n0computer/iroh-relay:v1.4.1` (alpine). Контейнер `mineup-relay-iroh` |
| Сети | `mineup-relay_relay_net` bridge `172.30.43.0/24` (`RELAY_SUBNET`); hub также в `mineup-skin_skin_net` (наша сеть skinserver) — только чтобы достучаться до Drasl `http://mineup-skin-drasl:25585` |
| Порты | `127.0.0.1:18080` → hub 8080 (`HUB_PORT`); `127.0.0.1:13340` → relay 3340 (`RELAY_HTTP_PORT`); при `QAD=1`: `0.0.0.0:7842/udp` (`QAD_PORT`) |
| Volume | нет: оба сервиса без состояния (комнаты в памяти hub, теряются при рестарте — хосты пересоздают) |
| Домен | `relay.dedseckms.ru`: `/api/v1/*` → hub, остальное → iroh-relay (`caddy/relay.caddy`) |

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
