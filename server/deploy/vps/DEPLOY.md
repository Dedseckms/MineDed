# VPS (Germany, 31.59.192.133) — deploy log

No secrets here. SSH key lives only on the operator machine (outside the repo).

## Host

- Ubuntu 24.04.4 LTS, kernel 6.8.0-111-generic, KVM guest (qemu-guest-agent).
- 1 vCPU (AMD Ryzen 9 9950X host), 2 GB RAM, 512 MB swap, 20 GB disk (`/dev/vda2`, 16 GB free).
- NIC `ens3`: IPv4 31.59.192.133/32, IPv6 2a12:bec4:1480:9e::2/64. Link speed not exposed by the guest;
  traffic quota/bandwidth must be checked in the provider panel.
- Preinstalled at handover: only stock Ubuntu services (sshd on 22, systemd-resolved on localhost).
  No Docker, no web server. Root password login was enabled (cloud-init drop-in).

## Phase 1 — 2026-10-26 (recon + hardening + base)

### SSH
- ed25519 key installed for `root` and for new user `ops` (groups `sudo`, `users`;
  `/etc/sudoers.d/90-ops` = NOPASSWD, because `ops` has no password).
- Backup of original sshd config: `/root/backup-20261026-074110/` (`sshd_config`, `sshd_config.d/`).
- `/etc/ssh/sshd_config.d/50-cloud-init.conf`: `PasswordAuthentication yes` -> `no`.
- `/etc/ssh/sshd_config`: `PermitRootLogin prohibit-password`, `PasswordAuthentication no`.
- New `/etc/ssh/sshd_config.d/10-hardening.conf`: `PermitRootLogin prohibit-password`,
  `PasswordAuthentication no`, `KbdInteractiveAuthentication no`, `X11Forwarding no`.
- `sshd -t` then `systemctl reload ssh`. Verified: key login root and ops (new connections),
  password auth refused (`Permission denied (publickey)`). Port stays 22.

### Firewall (ufw)
- Default deny incoming / allow outgoing; allowed 22/tcp, 80/tcp, 443/tcp, 443/udp (v4+v6).
- NOTE for phase 2: Docker-published ports bypass ufw. Publish container ports on `127.0.0.1`
  only and front everything with host Caddy (or filter in the `DOCKER-USER` chain).

### fail2ban
- `/etc/fail2ban/jail.d/sshd.local`: sshd jail, backend systemd, maxretry 5, findtime 10m, bantime 1h.

### Automatic updates
- `unattended-upgrades` enabled; `/etc/apt/apt.conf.d/20auto-upgrades` (daily lists + upgrade).

### Base packages (official repos)
- Docker: `/etc/apt/keyrings/docker.asc`, `/etc/apt/sources.list.d/docker.list`;
  `docker-ce`, `docker-ce-cli`, `containerd.io`, `docker-buildx-plugin`, `docker-compose-plugin`
  (Docker 29.9.2, Compose v5.6.1). Smoke test `hello-world` passed, image removed.
- Caddy: cloudsmith `caddy/stable` repo (`/usr/share/keyrings/caddy-stable-archive-keyring.gpg`,
  `/etc/apt/sources.list.d/caddy-stable.list`); Caddy v2.11.7, systemd unit enabled, default
  config (listens on :80). No services deployed yet.

### Rollback
- SSH: restore files from `/root/backup-20261026-074110/`, remove `10-hardening.conf`,
  `sshd -t && systemctl reload ssh` (only from an open key session; re-enabling passwords is not intended).
- ufw: `ufw disable`. fail2ban: `systemctl disable --now fail2ban`.
- Docker/Caddy: `apt-get purge` the packages and remove the repo files listed above.

## Phase 2 — 2026-10-26 (migration of skinserver + relay from the home server)

Owner approved moving the skins (incl. the Drasl texture signing key). Provider: 8 TB traffic,
10 Gbit. Same compose projects, container names, ports and folders as on the home server
(`server/skinserver/DEPLOY.md`, `server/relay/DEPLOY.md` describe the stacks).

### What was deployed
- `/opt/skinserver` (project `mineup-skin`): repo files from main + live `config/config.toml`,
  `.env`, `.secrets` from the home server (chmod 600). Image `mineup/drasl:4.0.1-hc` built here.
  Drasl `127.0.0.1:25585`, network `mineup-skin_skin_net` 172.30.42.0/24.
- Drasl volume `mineup-skin_drasl_state` restored from the home backup
  `/opt/skinserver/backups/skinserver-20261026-082750.tar.gz` with the procedure in
  `server/skinserver/DEPLOY.md` (stop, busybox tar into the volume, start). Checks:
  `signaturePublickey` identical to the live home server, `mu_tester` profile present,
  signed `textures` property served.
- `/opt/relay` (project `mineup-relay`): repo files from main, hub musl binary built from main
  (hub 1.0.7), site files from `site/` (README excluded), live `.env` from the home server with
  changes: `CADDY_UID=999` (caddy uid on this host; gid 988 same), `QAD=0` until a real cert
  exists, `GITHUB_REPO=Dedseckms/MineDed` (the home value `Dedseckms/MineUP` now returns
  GitHub 301 -> mirror state error). Previous `.env`: `/opt/relay/backups/env-20261026.bak`.
  Ports 127.0.0.1: hub 18080, iroh-relay 13340, updates-web 18090, site 18091. All containers healthy.
- Host Caddy: `/etc/caddy/Caddyfile` (original kept as `Caddyfile.orig-20261026`) now only
  `import /opt/skinserver/caddy/*.caddy` and `import /opt/relay/caddy/*.caddy` -> sites
  `skins.`, `relay.`, `mineded.`, `mineup.` (301 to mineded) `.dedseckms.ru`.
  Tested first with a temporary `local_certs` global option, then switched to the production
  config (ACME) and restarted Caddy; ACME will keep failing until DNS points here.
- ufw: + `7842/udp` (iroh-relay QAD; iroh-relay 1.4.1 has no separate STUN port).
  Docker publishes only on 127.0.0.1 except QAD (`0.0.0.0:7842/udp` when `QAD=1`).
- Cron `/etc/cron.d/mineup-skin-backup`: daily 04:17 `backup.sh` (stops drasl ~2 s), log in
  `/opt/skinserver/backups/backup.log`. Test run OK (`skinserver-20261026-083555.tar.gz`).
- Smoke test script `/opt/relay/smoke-vps.sh` (curl `--resolve <host>:443:127.0.0.1 -k`):
  skins `/authlib-injector` 200 + `X-Authlib-Injector-API-Location`, `/web/registration` 200;
  relay `/api/v1/health` 200 `{"status":"ok","version":"1.0.7"}`, `/api/v1/mirrors` 501
  (by design), `/ping` 200, `/healthz` 200, `/updates/status.json` 200 (`state: ok`, v1.0.7
  mirrored); mineded `/` 200, `/README.md` 404; mineup -> 301 mineded with path/query.
  Room create without a signature -> 400 `invalidInput` (hub validates as expected).

### Cutover (after the owner switches DNS)
1. DNS A records -> 31.59.192.133 (optional AAAA -> 2a12:bec4:1480:9e::2) for
   `skins`, `relay`, `mineded`, `mineup` `.dedseckms.ru`.
2. Final Drasl sync: stop drasl on the home server, take `backup.sh` there, restore here with the
   same procedure (otherwise registrations/skin changes made after 2026-10-26 08:27 UTC are lost).
3. `systemctl restart caddy` (skip ACME retry backoff), wait for certs:
   `ls /var/lib/caddy/.local/share/caddy/certificates/acme-v02.api.letsencrypt.org-directory/`.
4. `/opt/relay/.env`: `QAD=1`, then `cd /opt/relay && SUDO= ./start.sh`; check log
   `QUIC server listening on 0.0.0.0:7842`.
5. Public checks: `/opt/relay/check.sh`, Drasl `https://skins.dedseckms.ru/authlib-injector`.
6. Home server stacks stay up until the owner decides (stopping them needs the owner's "ok").

### Rollback
- DNS back to the home server (nothing there was changed).
- Here: `docker compose -p mineup-relay down`, `docker compose -p mineup-skin stop drasl`
  (volume kept), `cp /etc/caddy/Caddyfile.orig-20261026 /etc/caddy/Caddyfile && systemctl reload caddy`.
