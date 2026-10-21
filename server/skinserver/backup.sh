#!/usr/bin/env bash
# One-command backup: ./backup.sh [--online]
# Produces backups/skinserver-<UTC date>.tar.gz containing:
#   data/  - contents of volume mineup-skin_drasl_state (sqlite DB, textures,
#            texture signing key: losing it breaks cached signed skins)
#   conf/  - compose.yaml, Dockerfile, .env, config/, caddy/, scripts, .secrets
# Default: stops drasl for a few seconds so the sqlite DB is consistent.
# --online: no stop (DB may be caught mid-write; use only for ad-hoc copies).
# No temp files: tar runs inside our own image (busybox) and streams to host gzip.
set -euo pipefail
cd "$(dirname "$0")"
umask 077
SUDO=${SUDO:-sudo}
STAMP=$(date -u +%Y%m%d-%H%M%S)
OUT="backups/skinserver-$STAMP.tar.gz"
IMG=mineup/drasl:4.0.1-hc
VOL=mineup-skin_drasl_state
mkdir -p backups

FILES=()
for f in compose.yaml Dockerfile .env .env.example start.sh backup.sh DEPLOY.md .secrets config caddy; do
  [ -e "$f" ] && FILES+=("conf/$f")
done

ONLINE=0; [ "${1:-}" = "--online" ] && ONLINE=1
[ $ONLINE = 1 ] || $SUDO docker compose -p mineup-skin stop drasl
rc=0
$SUDO docker run --rm --network none --entrypoint /busybox \
  -v "$VOL:/data:ro" -v "$(pwd):/conf:ro" "$IMG" \
  tar -cf - -C / data "${FILES[@]}" | gzip > "$OUT" || rc=$?
[ $ONLINE = 1 ] || $SUDO docker compose -p mineup-skin start drasl
[ $rc = 0 ] || { echo "backup FAILED (rc=$rc), partial file: $OUT"; exit $rc; }
echo "backup: $(pwd)/$OUT ($(du -h "$OUT" | cut -f1))"
