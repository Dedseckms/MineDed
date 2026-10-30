# MineDed chat storage and operations

No server changes were made. Deploy only in `/opt/relay`; existing Docker/mihomo,
networks and other projects stay under their owner's management.

## Runtime

Set `HUB_CHAT_DIR=/data/chat` and mount the persistent `hub_chat` volume. The hub
opens `chat.sqlite3` with WAL, FULL synchronization, foreign keys, and a busy timeout.
Startup fails when storage cannot open; without HUB_CHAT_DIR chat returns 503.
Keep SQLite and WAL/SHM together. Never copy a live SQLite file with plain `cp`.
`files/` holds content-addressed blobs; attachment metadata and authorization live
in SQLite. Images accept PNG/JPEG/WebP, max 4096 per side / 8 million pixels /
64 MiB decoder allocation, re-encoded WebP plus a 320px thumbnail. WebP encoding
is lossless (the built-in Rust encoder does not expose lossy quality). Only two
uploads and eight blocking jobs run at a time. Budget 384 MiB hub memory.

`HUB_DRASL_ADMIN_URL` is an INTERNAL HTTP `/drasl/api/v3` root; the admin API token
is server-only. Role verification resolves player UUID -> player.userUuid ->
user.isAdmin, verifies player membership, and rejects locked owners. Cached at
most 30 seconds. Failures remove admin privileges; client UUID/role claims never
assign permissions. Moderator assignments live in SQLite. Drasl source reference:
https://github.com/unmojang/drasl/blob/v4.0.1/api.go (GPL-3.0).

## Offsite backup and notifications

Templates: `chat.env.example`. Configure secrets only on the server (mode 600).
Run `backup.py` daily using an existing owner-approved scheduler. It uses SQLite's
online backup API, validates integrity and attachment SHA-256, transfers blobs
first with rsync checksum and publishes a timestamped snapshot last. The SSH key
must be restricted on the home server to a dedicated backup folder; preinstall
the known host key and require BatchMode/StrictHostKeyChecking via SSH config.
Do not use rsync `--delete`. No passwords are accepted in command arguments.
The host needs Python 3.11+, sqlite3 stdlib, rsync, and OpenSSH.

Run `notify.py` hourly with the same host HUB_CHAT_DIR. It sends the owner only
80/95% disk warnings, backup failures and new report notifications. Tokens/IDs
stay in environment; exceptions are redacted. Dedup state commits after delivery.
Backup job writes `backup-status.json` only after successful offsite transfer;
admin health displays its last successful timestamp/error. Missing status means
no confirmed backup, never an assumed successful backup.

Monthly restore drill: copy a timestamped `chat.sqlite3` into an isolated folder,
copy its `files` from the offsite files folder, run `PRAGMA integrity_check`, verify
each attachment filename against SHA-256, then start an isolated hub and check
history and private downloads with test accounts. Never test against live volumes.
Retain the snapshot and record the restore drill result in `/opt/relay/DEPLOY.md`.

## Disk and migration

Disk accounting follows the active attachment filesystem. >=80% logs hourly;
>=95% returns 507 for uploads while text remains available. Existing downloads
and history remain authorized. Telegram notification uses the host-side job.

An admin chooses newDisk or objectStorage. Configure its absolute existing path
via HUB_CHAT_NEW_DISK_DIR or HUB_CHAT_OBJECT_STORAGE_DIR. The latter must be an
owner-managed durable S3-compatible filesystem mount (credentials remain outside
hub); native S3 HTTP uploads are not implemented. The operation refuses missing,
relative, nested or identical destinations. It copies every referenced blob,
checks the source and destination SHA-256, verifies the complete copy again,
then atomically persists the active directory in SQLite. Source data is retained.
Copying runs in a background worker and publishes admin progress events. Uploads
pause during migration; text and history continue. Failures leave the original
pointer intact and log a redacted error. Sources remain available for rollback.
No source deletion is automatic; any later cleanup requires owner approval.

Before deployment: back up compose/env/config and make a verified offsite chat
backup. Apply only the hub service change, check `/api/v1/health`, Drasl metadata,
`GET /chat/me` with a valid account, and admin state with a Drasl administrator.
Record exact changes, backup path, health results and rollback in service DEPLOY.md.
Rollback: restore previous binary/config (chat stays disabled in older binaries)
and preserve the chat volume. No daemon restart, firewall changes, or pruning.

