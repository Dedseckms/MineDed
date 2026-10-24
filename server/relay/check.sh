#!/usr/bin/env bash
# Post-deploy checks for mineup-relay. Read-only except for one test room that is
# created and closed again. Usage on the server:
#   ./check.sh                          # health + relay + neighbours
#   TEST_USER=mu_tester ./check.sh      # also create/join/close a room (asks password)
set -uo pipefail
cd "$(dirname "$0")"
set -a; . ./.env; set +a
SUDO=${SUDO:-sudo}
H=http://127.0.0.1:$HUB_PORT/api/v1
R=http://127.0.0.1:$RELAY_HTTP_PORT
ok=0; fail() { echo "FAIL: $*"; ok=1; }

echo "== containers"
$SUDO docker compose -p mineup-relay ps --format '{{.Name}} {{.Status}}'
echo "== hub health";   curl -fsS "$H/health" && echo || fail "hub health"
echo "== relay healthz"; curl -fsS -o /dev/null -w '%{http_code}\n' "$R/healthz" || fail "relay healthz"
echo "== relay ping";    curl -fsS -o /dev/null -w '%{http_code}\n' "$R/ping" || fail "relay ping"
echo "== updates status"
curl -fsS "http://127.0.0.1:${UPDATES_PORT:-18090}/status.json" && echo || fail "updates status"
echo "== updates via Caddy (expect 200)"
curl -sS -o /dev/null -w '%{http_code}\n' https://relay.dedseckms.ru/updates/status.json
echo "== site via Caddy (expect 200)"
curl -sS -o /dev/null -w '%{http_code}\n' https://mineup.dedseckms.ru/
echo "== join unknown room (expect 404)"
curl -sS -o /dev/null -w '%{http_code}\n' -H 'Content-Type: application/json' \
  -d '{"endpointId":"probe"}' "$H/rooms/AAAA-AAAA/join"
echo "== create without token (expect 401)"
curl -sS -o /dev/null -w '%{http_code}\n' -H 'Content-Type: application/json' -d '{}' "$H/rooms"

if [ -n "${TEST_USER:-}" ]; then
  read -rsp "password for $TEST_USER: " TEST_PASSWORD; echo
  AUTH=$(curl -sS -H 'Content-Type: application/json' \
    -d "{\"agent\":{\"name\":\"Minecraft\",\"version\":1},\"username\":\"$TEST_USER\",\"password\":\"$TEST_PASSWORD\"}" \
    "http://127.0.0.1:25585/authlib-injector/authserver/authenticate")
  unset TEST_PASSWORD
  TOKEN=$(printf '%s' "$AUTH" | sed -n 's/.*"accessToken":"\([^"]*\)".*/\1/p')
  [ -n "$TOKEN" ] || fail "authenticate"
  BODY='{"endpointId":"checkhost","sig":{"mc":"1.21.1","loader":"vanilla","modsHash":"x"},"authMode":"open","maxPeers":4}'
  ROOM=$(curl -sS -H "Authorization: Bearer $TOKEN" -H 'Content-Type: application/json' -d "$BODY" "$H/rooms")
  CODE=$(printf '%s' "$ROOM" | sed -n 's/.*"code":"\([^"]*\)".*/\1/p')
  RID=$(printf '%s' "$ROOM" | sed -n 's/.*"roomId":"\([^"]*\)".*/\1/p')
  echo "== created room code=$CODE"
  [ -n "$CODE" ] || fail "create room: $(printf '%s' "$ROOM" | sed 's/"hostToken":"[^"]*"/"hostToken":"***"/')"
  echo "== join (expect 200)"
  curl -sS -o /dev/null -w '%{http_code}\n' -H 'Content-Type: application/json' \
    -d '{"endpointId":"checkguest"}' "$H/rooms/$CODE/join"
  echo "== close (expect 204)"
  curl -sS -o /dev/null -w '%{http_code}\n' -X DELETE -H "Authorization: Bearer $TOKEN" "$H/rooms/$RID"
  unset TOKEN AUTH ROOM
fi

echo "== neighbours (must be unchanged)"
systemctl is-active mihomo caddy
$SUDO docker ps --format '{{.Names}} {{.Status}}'
curl -fsS -o /dev/null -w 'drasl %{http_code}\n' http://127.0.0.1:25585/authlib-injector || fail drasl
exit $ok
