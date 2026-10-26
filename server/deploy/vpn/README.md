# MineDed VPN (VPS 31.59.192.133, /opt/vpn)

Mirror of `/opt/vpn/DEPLOY.md` without secrets.

## What runs

- Container `mineded-vpn`: s-ui `v1.6.6` (pinned by digest, embedded sing-box core),
  `docker-compose.yml` here. mem 256M, json-file logs 10m x3, image healthcheck `./sui healthcheck`.
- Inbounds (shared port 8443):
  - `MineDed HY2` - Hysteria2 on 8443/udp, salamander obfs, TLS with the Caddy Let's Encrypt cert
    for `relay.dedseckms.ru` (mounted read-only into `/certs`), masquerade `https://mineded.dedseckms.ru`.
  - `MineDed VLESS` - VLESS + Reality (xtls-rprx-vision) on 8443/tcp, handshake/SNI `www.microsoft.com`.
- Full VPN; the core's route skeleton blocks only: BitTorrent (sniff), SMTP port 25,
  private/LAN/loopback destinations (after resolve, IPv4-only - the docker bridge has no IPv6).
  Core log level `warn`, no access log.
- Per-user clients, 256 GiB (274877906944 B) each; `globalReset = "0 0 1 * *"` (1st of every month,
  `timeLocation = Europe/Moscow`) zeroes usage and re-enables depleted clients. A client over quota is
  disabled by the deplete job: auth fails, its subscription URL returns 400.
- Panel `127.0.0.1:2095`, subscription server `127.0.0.1:2096`; exposed only via host Caddy at secret
  paths under `relay.dedseckms.ru` (`vpn.caddy.template` -> `/opt/vpn/caddy/vpn.caddy`, imported in the
  relay site block of `/opt/relay/caddy/relay.caddy`). s-ui `webPath`/`subPath` equal those paths.
- Subscription per client: `https://relay.dedseckms.ru/<subPath>/<client name>` (base64 links),
  `?format=clash` (mihomo), `?format=json` (sing-box). The client `name` is a random id (`u<32 hex>`)
  and acts as the subscription secret; `remark` is the human name.
- ufw: `8443/tcp`, `8443/udp` allowed.
- `mineded-vpn-cert.path` (systemd) restarts the container when Caddy renews the cert.
- `/etc/cron.d/mineded-vpn-backup` runs `backup.sh` daily 04:17: `getdb?exclude=stats,changes`
  into `/opt/vpn/backups/` (14 kept).

## Secrets (server only)

`/opt/vpn/.secrets` (0600): `PANEL_PATH`, `SUB_PATH`, `ADMIN_USER`, `ADMIN_PASS`, `HY2_OBFS`,
`SUI_TOKEN` (API token "hub", no expiry), `REALITY_PRIV/PUB/SID`. Client credentials live in the s-ui DB.

## API for hub (s-ui `/apiv2`)

Base `http://127.0.0.1:2095/<PANEL_PATH>/apiv2/`, header `Token: <SUI_TOKEN>`; failures are HTTP 200
with `success:false`. Create user: `POST save` form `object=clients&action=new&data=<json>` with
`{enable, name, remark, inbounds:[1,2], volume:274877906944, expiry:0, links:[],
config:{vless:{name,uuid,flow:"xtls-rprx-vision"}, hysteria2:{name,password}}}`.
Disable: `action=edit` with the full object and `enable:false`. Delete: `action=del&data=<id>`
(a bare id; the wiki's `{"id":N}` form is rejected in v1.6.6). List/usage: `GET clients`.
The hub container is on another docker network: reach it via a shared network or host gateway (TBD).

## Deploy from scratch

1. Copy `docker-compose.yml`, `vpn.caddy.template`, `mineded-vpn-cert.*`, `backup.sh` to `/opt/vpn`.
2. Generate `.secrets` (random paths/creds), `docker compose up -d`,
   `docker exec mineded-vpn ./sui admin -username .. -password ..`,
   `./sui setting -path /<PANEL_PATH>/ -subPath /<SUB_PATH>/`, restart.
3. Login (`POST <panel>/api/login`), create API token (`POST api/addToken`, `expiry=0&desc=hub`).
4. Via `apiv2/save`: `config` (route rules above), two `tls` objects, two `inbounds`
   (with `addrs` = `relay.dedseckms.ru:8443` and `out_json`), clients, settings
   (`globalReset`, `timeLocation`, `subURI`, `webURI`, `subShowInfo`, `subClashUdp`).
5. Render `caddy/vpn.caddy`, add `import /opt/vpn/caddy/vpn.caddy` to the relay site block
   (before the iroh catch-all `handle`), `caddy validate`, `systemctl reload caddy`.
6. `ufw allow 8443/tcp; ufw allow 8443/udp`; install and enable `mineded-vpn-cert.path`; cron backup.

## Rollback

- Caddy: restore `/opt/vpn/backups/relay.caddy.<ts>` to `/opt/relay/caddy/relay.caddy`,
  `caddy validate`, `systemctl reload caddy`.
- VPN off: `cd /opt/vpn && docker compose down`; `ufw delete allow 8443/tcp`, `ufw delete allow 8443/udp`;
  `systemctl disable --now mineded-vpn-cert.path`; remove `/etc/cron.d/mineded-vpn-backup`.
- DB restore: `POST apiv2/importdb` with a file from `/opt/vpn/backups/`.
