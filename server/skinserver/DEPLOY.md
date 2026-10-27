# Skin server (Drasl) — /opt/skinserver

Журнал деплоя и эксплуатации. Секретов здесь нет: логины/пароли админа и тестового
пользователя, неиспользованный инвайт — только в `/opt/skinserver/.secrets` (chmod 600).
Копия в репозитории: `server/skinserver/`.

## Что развёрнуто

| | |
|---|---|
| Сервер | Drasl 4.0.1 (`unmojang/drasl:4.0.1`, Go, Nix-образ без shell) |
| Образ | `mineup/drasl:4.0.1-hc` = Drasl + статический busybox 1.37.0-musl (`/busybox`) для healthcheck и backup |
| Compose-проект | `mineup-skin` (`compose.yaml`), контейнер `mineup-skin-drasl` |
| Сеть | `mineup-skin_skin_net`, bridge, `172.30.42.0/24` (`SKIN_SUBNET` в `.env`) |
| Volume | `mineup-skin_drasl_state` → `/var/lib/drasl` (sqlite, текстуры, **ключ подписи текстур**) |
| Порт | `127.0.0.1:25585` → 25585 (`DRASL_PORT` в `.env`) |
| Конфиг | `config/config.toml` (ro-mount в `/etc/drasl`) |
| Домен | `skins.dedseckms.ru` (DNS и Caddy — запрос владельцу, см. ниже) |
| API root (authlib-injector) | `https://skins.dedseckms.ru/authlib-injector` |

Изоляция и лимиты: `cap_drop: [ALL]`, `no-new-privileges`, `read_only` rootfs + tmpfs `/tmp`,
без `privileged`/`cap_add`/`network_mode: host`; `mem_limit 192m` (swap запрещён),
`cpus 0.5`, `cpu_shares 256` (у контейнеров владельца 1024 по умолчанию), `pids_limit 128`,
`GOMEMLIMIT=140MiB`; логи `json-file` 10m × 3. Healthcheck: `wget /authlib-injector` раз в 30 с.
Внутри контейнера процесс uid 0 (у образа нет USER), но без capabilities; chown тома
без `cap_add` невозможен, поэтому non-root не делали.

Почему 0.5 CPU / 192 MiB: пароли хешируются scrypt (N=32768, r=8) — ~32 MiB и заметное
CPU-время на каждый вход. Остаток бюджета (≤ 512 MiB / ≤ 1 CPU на наш стек) — hub/relay/mirror.

## Регистрация и rate limit

- Регистрация только по инвайтам (`RegistrationUsernamePassword.CreateNewPlayer.RequireInvite`),
  username = ник, 1 игрок на пользователя, смена ника выключена, текстуры по URL выключены.
- Пароли: **scrypt** (Drasl не умеет argon2), минимальная длина 10.
- Rate limit — встроенный в Drasl (`[RateLimit]` 2 rps, burst 20) на `/authenticate`,
  `/signout`, `/web/login`, `/web/register`, `/drasl/api/v3/login`, `/drasl/api/v3/users`,
  поиск профилей. Ключ — пользователь или реальный IP клиента (из `X-Forwarded-For` /
  `X-Real-IP`, которые Caddy перезаписывает, см. `caddy/skin.caddy`). Админы не лимитируются.
  В Caddy 2.6.2 модуля rate_limit нет, поэтому лимит не на прокси.
- Первый админ `mu_admin` создан по стартовому инвайту из лога Drasl
  (`DefaultAdmins = ["mu_admin"]`).

## Эксплуатация

```bash
cd /opt/skinserver
./start.sh                                    # проверка портов (ss -tln) + build + up -d
sudo docker compose -p mineup-skin ps
sudo docker compose -p mineup-skin logs --tail 100 drasl
sudo docker compose -p mineup-skin restart drasl
```

Админка: `https://skins.dedseckms.ru/web/admin` (пользователи, админ/бан = Locked, инвайты);
сброс пароля — страница пользователя `/web/user/<uuid>` или `PATCH /drasl/api/v3/users/{uuid}`
с `password`. Личный кабинет `/web/user`, `/web/player/<uuid>`: скин classic/slim, плащ,
3D-превью (skinview3d), смена пароля. API: `/drasl/api/v3/swagger.json`.

## Бэкап и восстановление

```bash
/opt/skinserver/backup.sh            # стоп drasl на ~2 с, tar тома + конфиги → backups/skinserver-<UTC>.tar.gz
/opt/skinserver/backup.sh --online   # без остановки (sqlite может быть несогласован)
```

Архив: `data/` (содержимое тома) и `conf/` (compose, Dockerfile, .env, .secrets, config, caddy,
скрипты). Файл chmod 600. Хранить копию вне сервера.

Восстановление (том пустой или заменяем содержимое):

```bash
cd /opt/skinserver
F=backups/skinserver-<дата>.tar.gz
mkdir -p /tmp/skin-restore && tar -xzf "$F" -C /tmp/skin-restore conf      # при необходимости вернуть конфиги вручную
sudo docker compose -p mineup-skin stop drasl
gunzip -c "$F" | sudo docker run --rm -i --network none --entrypoint /busybox \
  -v mineup-skin_drasl_state:/data mineup/drasl:4.0.1-hc tar -xf - -C / data
sudo docker compose -p mineup-skin start drasl
```

Если образа `mineup/drasl:4.0.1-hc` нет — сначала `./start.sh` (соберёт его и создаст том).
Ключ подписи текстур лежит в томе: его потеря/ротация ломает подпись кэшированных скинов.

## Публикация

Основной вариант: хостовый Caddy владельца, `import /opt/skinserver/caddy/*.caddy`
(файл `caddy/skin.caddy`), роутер уже пробрасывает 80/443. Запасной (если пропадёт белый IP /
проброс): VPS с frps или rathole-server, на хосте клиент в отдельном контейнере в
`/opt/relay` с исходящим TCP-туннелем `127.0.0.1:25585` → VPS; TLS терминирует Caddy на VPS
(`reverse_proxy` в туннель). Требования: на VPS открыт 443 и порт управления туннелем,
токен туннеля — секрет только на серверах.

## mihomo

Проверено 2026-10-21: в `/etc/mihomo/config.yaml` нет секции `tun:` (строки `enable: true` —
у `dns:` и `health-check:`), TUN-интерфейса нет, `ip rule` стандартный → трафик контейнеров
в mihomo не попадает. Если владелец включит TUN с `auto-route`, добавить в `tun:`:
`route-exclude-address: [172.30.42.0/24]`.

## Журнал

### 2026-10-21 — первичный деплой (server-ops)
1. До: `docker ps` — remnawave_bot (Up 5 months, unhealthy, RestartCount=16 — было до нас),
   remnawave_bot_db / _redis healthy, restarts 0; mihomo и caddy active; сайты владельца
   через `127.0.0.1:8080` — md5 ответов зафиксированы.
2. `sudo mkdir -p /opt/skinserver /opt/relay && sudo chown dedseckms:dedseckms ...`.
3. `docker pull unmojang/drasl:4.0.1`; `./start.sh` → собран `mineup/drasl:4.0.1-hc`,
   созданы сеть `mineup-skin_skin_net`, том `mineup-skin_drasl_state`, контейнер healthy.
4. Через API: `mu_admin` (стартовый инвайт), 2 инвайта, `mu_tester` по инвайту, slim-скин
   (`test/skin.png`), второй инвайт оставлен неиспользованным. Креды → `.secrets`.
5. Проверки curl (127.0.0.1:25585): регистрация без инвайта → 400; по инвайту → 200;
   `/authlib-injector/authserver/authenticate` → профиль + токен; validate → 204;
   PUT skin → 204; session profile → подписанное свойство textures, SKIN url + model slim;
   текстура отдаётся `image/png`; метаданные `/authlib-injector` → Drasl 4.0.1, publickey;
   rate limit → 429 после серии запросов.
6. `backups/skinserver-20261021-080542.tar.gz` — пустой файл от первой (неудачной) попытки
   backup.sh (в образе нет gzip-апплета; исправлено, gzip на хосте). Удалить с разрешения владельца.
7. После: чужие контейнеры те же (StartedAt и RestartCount не изменились), mihomo/caddy
   active, ответы сайтов через 8080 совпадают по md5. Caddy и mihomo не трогались.

## 2026-10-26: theme1 + Accept-Language default
- Image mineup/drasl:4.0.1-hc-theme1 (theme baked in); ApplicationName = "MineDed Skins". Previous image kept as mineup/drasl:4.0.1-hc-rollback.
- Backups on server: backups/skinserver-20261026-125105.tar.gz, backups/*.pre-theme1, backups/skin.caddy.<ts>.bak.
- Rollback: restore *.pre-theme1, then docker compose -p mineup-skin up -d --no-build --force-recreate --no-deps drasl.
- Caddy: requests without Accept-Language get Accept-Language: ru.
- Signing key unchanged (signaturePublickey identical before/after).

## 2026-10-27 security headers
- Site block: HSTS, nosniff, X-Frame-Options DENY, Referrer-Policy, -Server + CSP (script-src 'unsafe-inline': Drasl inline module scripts). Backup on server: caddy/*.caddy.bak-20261027-060540. Rollback: restore backup, caddy validate, systemctl reload caddy.

