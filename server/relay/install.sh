#!/usr/bin/env bash
# Unpacks an upload made by deploy.ps1 into /opt/relay. No sudo, no rm: the previous
# files are archived to backups/ and the upload tarball is kept there too.
# Usage: bash /opt/relay/install.sh /opt/relay/upload-<ts>.tar <ts>
set -euo pipefail
TAR=$1; TS=$2
cd /opt/relay
mkdir -p backups && chmod 700 backups
if [ -f compose.yaml ]; then
  tar --exclude=./backups --exclude="./$(basename "$TAR")" -czf "backups/relay-$TS.tar.gz" .
  chmod 600 "backups/relay-$TS.tar.gz"
  echo "backup: /opt/relay/backups/relay-$TS.tar.gz"
fi
tar -xf "$TAR" --no-same-owner
mv "$TAR" "backups/upload-$TS.tar"
chmod +x start.sh check.sh install.sh hub/hub
if [ ! -f .env ]; then cp .env.example .env; chmod 600 .env; echo "created .env from .env.example"; fi
echo "installed $TS"
