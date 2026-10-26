#!/usr/bin/env bash
# Smoke test of all public hostnames through the host Caddy (local resolve, -k).
R() { echo "--resolve $1:443:127.0.0.1"; }
c() { # host path [expected]
  local code
  code=$(curl -sk $(R "$1") -o /dev/null -w '%{http_code}' "https://$1$2")
  echo "$code $1$2"
}
c skins.dedseckms.ru /authlib-injector
c skins.dedseckms.ru /web/registration
c relay.dedseckms.ru /api/v1/health
c relay.dedseckms.ru /api/v1/mirrors
c relay.dedseckms.ru /ping
c relay.dedseckms.ru /healthz
c relay.dedseckms.ru /updates/status.json
c mineded.dedseckms.ru /
c mineded.dedseckms.ru /README.md
curl -skI $(R mineup.dedseckms.ru) "https://mineup.dedseckms.ru/a?b=1" | grep -iE '^(HTTP|location)' | tr -d '\r'
curl -sk $(R relay.dedseckms.ru) "https://relay.dedseckms.ru/updates/status.json"; echo
# Open room: create (anonymous) then delete with hostToken.
body='{"authMode":"open","endpointId":"'$(head -c 32 /dev/urandom | od -An -tx1 | tr -d ' \n')'"}'
resp=$(curl -sk $(R relay.dedseckms.ru) -H 'Content-Type: application/json' -d "$body" \
  -w '\n%{http_code}' https://relay.dedseckms.ru/api/v1/rooms)
code=$(echo "$resp" | tail -1); json=$(echo "$resp" | head -n -1)
echo "create room: $code $(echo "$json" | head -c 160)"
if [ "$code" = 201 ]; then
  id=$(echo "$json" | sed -n 's/.*"roomId":"\([^"]*\)".*/\1/p')
  tok=$(echo "$json" | sed -n 's/.*"hostToken":"\([^"]*\)".*/\1/p')
  curl -sk $(R relay.dedseckms.ru) -X DELETE -H "Authorization: Bearer $tok" \
    -o /dev/null -w 'delete room: %{http_code}\n' "https://relay.dedseckms.ru/api/v1/rooms/$id"
fi
