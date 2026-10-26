#!/bin/sh
# Daily s-ui DB backup (clients, inbounds, TLS, settings; without stats/changes).
# Installed as /opt/vpn/backup.sh, run by /etc/cron.d/mineded-vpn-backup. Keeps 14 copies.
set -eu
cd /opt/vpn
. ./.secrets
umask 077
mkdir -p backups
OUT="backups/s-ui-$(date +%F).db"
curl -sf -H "Token: $SUI_TOKEN" \
  "http://127.0.0.1:2095/$PANEL_PATH/apiv2/getdb?exclude=stats,changes" -o "$OUT.tmp"
mv "$OUT.tmp" "$OUT"
ls -1t backups/s-ui-*.db | tail -n +15 | xargs -r rm -f
