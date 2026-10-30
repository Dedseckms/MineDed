# Restricted CurseForge catalogue broker

This change is prepared locally. No server configuration or deployment has been performed.
The owner must approve a concrete deployment plan before changing the VPS.

The broker is optional. An absent or empty `CURSEFORGE_API_KEY` leaves catalogue requests
at HTTP 503; existing hub features continue normally. Provision the credential only on
the VPS, in the service's protected environment file. Never put it in the launcher,
Git, command arguments, logs, support reports, or examples. See `broker.env.example`.

Before an approved rollout, back up `/opt/relay` service configuration privately on
the VPS, retain the previous hub binary/image, and set the credential in
`/opt/relay/.env` (mode 600) together with the `CURSEFORGE_API_KEY` passthrough in the
`hub` service of `/opt/relay/compose.yaml`. Avoid commands that print resolved environment values
(including unredacted `docker compose config` or inspect output). Restart only the
project hub service. Do not restart Docker, mihomo, or unrelated services, and do not
change firewall, DNS, routes, systemd, or network configuration.

Verify `/api/v1/health`, Drasl authlib-injector metadata, a Minecraft search and project/file
lookup, bounded metadata batches, then verify unknown query keys, client API-key headers, unlisted methods and
non-Minecraft resources are rejected. An upstream credential failure must return a
generic HTTP 502 without upstream bodies or headers. Record checks, backup location,
changed files and rollback in `/opt/relay/DEPLOY.md` and a secret-free repository note.

Rollback: restore the previous hub binary/image and its backed-up service configuration,
then restart only hub and repeat health/Drasl metadata checks. Removing the variable
disables this broker on the next hub start. Keep Drasl state and signing keys unchanged.

The fixed upstream is `https://api.curseforge.com/v1`; redirects and arbitrary upstream
URLs are disabled. Six GET routes and two read-only POST batch routes under `/api/v1/catalog/curseforge` enforce game 432,
project/file ownership, allowed query names and ranges. File-ID resolution uses official
`POST /mods/files` with one numeric ID. Batches accept exactly `modIds` or `fileIds`,
containing 1–50 unique positive int32 IDs, with a 4 KiB request cap. Batch files are
checked against a fixed official project batch before returning any data.
Downloads require author distribution permission
and a non-null official download URL; CDN paths are never reconstructed.

Limits: 15 seconds per broker operation, 4 MiB streamed response cap, eight concurrent
upstream operations, per-IP token bucket with 60 initial tokens and one token per second.
Successful public metadata is cached for 60 seconds, at most 64 entries and 16 MiB.
Per-IP tracking is bounded by the hub limiter. With `HUB_TRUST_PROXY=true`, expose hub
only through the trusted proxy that overwrites `X-Real-IP`; clients must not reach the
hub port directly. Limits apply to cached hits too. No third-party credentials or
response headers are forwarded.

Official API reference: https://docs.curseforge.com/rest-api/.

## Runtime trust roots

The existing `busybox:1.37.0-musl` runtime now receives only the public CA bundle
from the pinned official `caddy:2.11.7-alpine` build stage. `SSL_CERT_FILE` points
to `/etc/ssl/cert.pem` so the rustls platform verifier can validate the fixed HTTPS
upstream. No Caddy executable, configuration or service is copied or started.
The upstream [official image definition](https://github.com/caddyserver/caddy-docker/blob/9234f5785658cae8129bfae366e196d0670468ac/2.11/alpine/Dockerfile)
installs `ca-certificates`; the [official image tag listing](https://github.com/docker-library/docs/blob/master/caddy/README.md)
identifies this pinned tag. Local Docker is unavailable, so the image build and
in-container TLS smoke test remain required before an approved rollout.
