#!/usr/bin/env bash
# Build and start the mineup-relay compose project after pre-flight checks.
# Usage: ./start.sh            (sudo is used only for docker compose -p mineup-relay
#                               and read-only checks: docker network inspect, test -r)
set -euo pipefail
cd "$(dirname "$0")"
[ -f .env ] || { echo "missing .env (copy .env.example)"; exit 1; }
set -a; . ./.env; set +a
SUDO=${SUDO:-sudo}
P=mineup-relay

FORBIDDEN_RE='^(53|80|443|1053|789[0-9]|8080|9090|25585)$'
check_port() { # proto port container
  local proto=$1 p=$2 c=$3 flag=-tln
  [ "$proto" = udp ] && flag=-uln
  [[ "$p" =~ ^[0-9]+$ ]] || { echo "bad port: $p"; exit 1; }
  if [[ "$p" =~ $FORBIDDEN_RE ]]; then echo "port $p is forbidden"; exit 1; fi
  if ss $flag "( sport = :$p )" | tail -n +2 | grep -q .; then
    # Busy is fine only if it is held by our own container.
    if $SUDO docker ps --filter "name=^$c\$" --format '{{.Ports}}' | grep -q ":$p->"; then
      echo "$proto/$p: held by $c (ok)"
    else
      echo "$proto/$p is busy:"; ss $flag "( sport = :$p )"; exit 1
    fi
  else
    echo "$proto/$p: free"
  fi
}

check_port tcp "$HUB_PORT" mineup-relay-hub
check_port tcp "$RELAY_HTTP_PORT" mineup-relay-iroh
FILES=(-f compose.yaml)
if [ "${QAD:-0}" = 1 ]; then
  check_port udp "$QAD_PORT" mineup-relay-iroh
  $SUDO test -r "$CADDY_CERT_DIR/relay.dedseckms.ru.crt" \
    || { echo "no Caddy cert in $CADDY_CERT_DIR (DNS + Caddy import first)"; exit 1; }
  FILES+=(-f compose.qad.yaml)
fi

[ -x hub/hub ] || { echo "missing hub/hub binary (run deploy.ps1)"; exit 1; }
$SUDO docker network inspect mineup-skin_skin_net >/dev/null \
  || { echo "network mineup-skin_skin_net missing: start /opt/skinserver first"; exit 1; }

$SUDO docker compose -p $P "${FILES[@]}" config -q
$SUDO docker compose -p $P "${FILES[@]}" up -d --build
$SUDO docker compose -p $P "${FILES[@]}" ps
