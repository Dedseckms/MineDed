#!/usr/bin/env bash
# Build and start the mineup-skin compose project after checking ports.
# Usage: ./start.sh            (sudo is used only for docker compose -p mineup-skin)
set -euo pipefail
cd "$(dirname "$0")"
[ -f .env ] || { echo "missing .env (copy .env.example)"; exit 1; }
set -a; . ./.env; set +a
SUDO=${SUDO:-sudo}

FORBIDDEN_RE='^(53|80|443|1053|789[0-9]|8080|9090)$'
for p in "$DRASL_PORT"; do
  [[ "$p" =~ ^[0-9]+$ ]] || { echo "bad port: $p"; exit 1; }
  if [[ "$p" =~ $FORBIDDEN_RE ]]; then echo "port $p is forbidden"; exit 1; fi
  # Port must be free, unless it is already held by our own running container.
  if ss -tln "( sport = :$p )" | tail -n +2 | grep -q .; then
    if $SUDO docker ps --filter name=^mineup-skin-drasl$ --format '{{.Ports}}' | grep -q "127.0.0.1:$p->"; then
      echo "port $p: held by mineup-skin-drasl (ok)"
    else
      echo "port $p is busy:"; ss -tln "( sport = :$p )"; exit 1
    fi
  else
    echo "port $p: free"
  fi
done

$SUDO docker compose -p mineup-skin config -q
$SUDO docker compose -p mineup-skin up -d --build
$SUDO docker compose -p mineup-skin ps
