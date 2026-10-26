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
