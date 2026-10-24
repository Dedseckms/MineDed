#!/bin/sh
# Update mirror: every UPDATES_INTERVAL_SECS copies the assets of the latest GitHub release
# of GITHUB_REPO into /data/www and publishes latest.json with its platform URLs rewritten
# to UPDATES_BASE_URL/<file>, so clients that cannot reach GitHub can still update.
# Empty GITHUB_REPO = idle. No credentials: unauthenticated API (60 req/h, we use 2/h).
# /data/www/status.json is always refreshed (healthcheck + debugging); assets are published
# file by file (atomic mv), latest.json last, files of older releases are pruned after it.
set -u
REPO=${GITHUB_REPO:-}
BASE=${UPDATES_BASE_URL:-https://relay.dedseckms.ru/updates}
INTERVAL=${UPDATES_INTERVAL_SECS:-1800}
MAX_MB=${UPDATES_MAX_TOTAL_MB:-1024}
WWW=/data/www
TMP=/data/tmp
UA="mineup-updates-mirror"
mkdir -p "$WWW" "$TMP"

log() { echo "$(date -u +%Y-%m-%dT%H:%M:%SZ) $*"; }

status() { # state tag message
  jq -n --arg repo "$REPO" --arg state "$1" --arg tag "$2" --arg msg "$3" \
    --arg at "$(date -u +%Y-%m-%dT%H:%M:%SZ)" \
    '{repo:$repo, state:$state, tag:$tag, message:$msg, checkedAt:$at}' >"$TMP/status.json" \
    && mv "$TMP/status.json" "$WWW/status.json"
}

valid_name() {
  case "$1" in
    "" | .* | *[!A-Za-z0-9._+-]*) return 1 ;;
    status.json) return 1 ;;
  esac
  return 0
}

fetch_asset() { # name size url digest -> $WWW/name (verified)
  name=$1 size=$2 url=$3 digest=$4
  if [ -f "$WWW/$name" ] && [ "$(stat -c %s "$WWW/$name")" = "$size" ]; then
    return 0
  fi
  log "download $name ($size bytes)"
  curl -fsSL --retry 2 --connect-timeout 20 --max-time 1800 -A "$UA" \
    -o "$TMP/$name.part" "$url" || { log "download failed: $name"; return 1; }
  if [ "$(stat -c %s "$TMP/$name.part")" != "$size" ]; then
    log "size mismatch: $name"; rm -f "$TMP/$name.part"; return 1
  fi
  case "$digest" in
    sha256:*)
      got=$(sha256sum "$TMP/$name.part" | cut -d' ' -f1)
      if [ "sha256:$got" != "$digest" ]; then
        log "digest mismatch: $name"; rm -f "$TMP/$name.part"; return 1
      fi
      ;;
  esac
  mv "$TMP/$name.part" "$WWW/$name"
}

sync_once() {
  if [ -z "$REPO" ]; then status idle "" "GITHUB_REPO is empty"; return; fi
  case "$REPO" in
    */* ) ;;
    *) status error "" "bad GITHUB_REPO"; return ;;
  esac
  case "$REPO" in *[!A-Za-z0-9._/-]* | */*/*) status error "" "bad GITHUB_REPO"; return ;; esac

  code=$(curl -sS --connect-timeout 20 --max-time 60 -A "$UA" -o "$TMP/release.json" \
    -w '%{http_code}' -H 'Accept: application/vnd.github+json' \
    -H 'X-GitHub-Api-Version: 2022-11-28' \
    "https://api.github.com/repos/$REPO/releases/latest") || code=000
  case "$code" in
    200) ;;
    404) log "no published release for $REPO"; status idle "" "no release"; return ;;
    *) log "release lookup failed: HTTP $code"; status error "" "github HTTP $code"; return ;;
  esac

  tag=$(jq -r '.tag_name // ""' "$TMP/release.json")
  total=$(jq '[.assets[]?.size] | add // 0' "$TMP/release.json")
  if [ "$total" -gt $((MAX_MB * 1024 * 1024)) ]; then
    log "release $tag too large: $total bytes"; status error "$tag" "release too large"; return
  fi
  jq -r '.assets[]? | [.name, (.size | tostring), .browser_download_url, (.digest // "")] | @tsv' \
    "$TMP/release.json" >"$TMP/assets.tsv"

  keep=" status.json latest.json "
  failed=0 has_latest=0
  tab=$(printf '\t')
  while IFS="$tab" read -r name size url digest; do
    if ! valid_name "$name"; then log "skip asset with unsafe name"; continue; fi
    if [ "$name" = latest.json ]; then
      has_latest=1
      curl -fsSL --retry 2 --connect-timeout 20 --max-time 120 -A "$UA" \
        -o "$TMP/latest.src.json" "$url" || failed=1
      continue
    fi
    keep="$keep$name "
    fetch_asset "$name" "$size" "$url" "$digest" || failed=1
  done <"$TMP/assets.tsv"

  if [ "$failed" = 1 ]; then status error "$tag" "some assets failed, will retry"; return; fi

  if [ "$has_latest" = 1 ]; then
    # Rewrite every platform URL to our mirror; refuse to publish if a file is missing.
    jq --arg base "$BASE" \
      'if (.platforms | type) == "object" then
         .platforms |= with_entries(.value.url = ($base + "/" + (.value.url | split("/") | last)))
       else . end' "$TMP/latest.src.json" >"$TMP/latest.json" \
      || { status error "$tag" "bad latest.json"; return; }
    for f in $(jq -r '.platforms // {} | .[].url | split("/") | last' "$TMP/latest.json"); do
      if ! valid_name "$f" || [ ! -f "$WWW/$f" ]; then
        log "latest.json references missing file"; status error "$tag" "latest.json references missing file"
        return
      fi
    done
    mv "$TMP/latest.json" "$WWW/latest.json"
  fi

  # Prune files that are not part of the current release (only after latest.json switched).
  for path in "$WWW"/*; do
    [ -f "$path" ] || continue
    f=${path##*/}
    case "$keep" in *" $f "*) ;; *) log "prune $f"; rm -f "$path" ;; esac
  done
  [ "$has_latest" = 1 ] || rm -f "$WWW/latest.json"
  rm -f "$TMP"/*.part
  log "release $tag mirrored"
  status ok "$tag" ""
}

trap 'exit 0' TERM INT
log "start: repo='${REPO}' interval=${INTERVAL}s"
while :; do
  sync_once
  sleep "$INTERVAL" &
  wait $!
done
