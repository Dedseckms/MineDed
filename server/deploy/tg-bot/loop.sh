#!/bin/sh
# Announces new MineDed GitHub releases in the Telegram channel.
# Loop: every INTERVAL_SEC poll releases/latest; if tag != last posted, fetch CHANGELOG.md and
# scripts/telegram-changelog.mjs at that tag and run it; save the tag only on success.
# One-shot check: sh /app/loop.sh --dry-run <tag>   (prints the message, posts nothing)
set -u
REPO="${REPO:-Dedseckms/MineDed}"
INTERVAL="${INTERVAL_SEC:-600}"
STATE="${STATE_FILE:-/home/node/state/last_tag}"
SEED="${SEED_TAG:-v1.2.1}"

log() { echo "$(date -u +%FT%TZ) $*"; }

# fetch_run <tag> [extra args] -> exit code of telegram-changelog.mjs
fetch_run() {
  tag="$1"; shift
  dir="$(mktemp -d)"
  ok=""
  # Prefer files at the tag; fall back to main (older tags predate CHANGELOG.md/the script).
  for ref in "$tag" main; do
    raw="https://raw.githubusercontent.com/$REPO/$ref"
    if wget -q -O "$dir/CHANGELOG.md" "$raw/CHANGELOG.md" 2>/dev/null &&
      wget -q -O "$dir/telegram-changelog.mjs" "$raw/scripts/telegram-changelog.mjs" 2>/dev/null; then
      ok="$ref"
      break
    fi
  done
  if [ -n "$ok" ]; then
    [ "$ok" = "$tag" ] || log "files not found at $tag, using $ok"
    (cd "$dir" && node telegram-changelog.mjs "$tag" "$@")
    rc=$?
  else
    log "download failed for $tag"
    rc=1
  fi
  rm -rf "$dir"
  return "$rc"
}

if [ "${1:-}" = "--dry-run" ]; then
  fetch_run "${2:?tag required}" --dry-run
  exit $?
fi

# Never post without a writable state file: otherwise every cycle would re-announce.
if [ ! -s "$STATE" ]; then
  echo "$SEED" > "$STATE" || { log "FATAL: cannot write $STATE"; sleep 60; exit 1; }
  log "seeded state with $SEED"
fi
touch "$STATE" 2>/dev/null || { log "FATAL: $STATE not writable"; sleep 60; exit 1; }
log "watching $REPO every ${INTERVAL}s, last posted $(cat "$STATE")"

while true; do
  tag="$(wget -q -O - -U mineded-tg-bot --header 'Accept: application/vnd.github+json' \
    "https://api.github.com/repos/$REPO/releases/latest" 2>/dev/null |
    sed -n 's/.*"tag_name"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' | head -n 1)"
  last="$(cat "$STATE" 2>/dev/null)"
  if [ -z "$last" ]; then
    log "FATAL: state $STATE empty or unreadable"; sleep 60; exit 1
  elif [ -z "$tag" ]; then
    log "github api: no tag (rate limit or network)"
  elif [ "$tag" != "$last" ]; then
    log "new release $tag (last $last)"
    if fetch_run "$tag"; then
      if echo "$tag" > "$STATE.tmp" && mv "$STATE.tmp" "$STATE"; then
        log "announced $tag"
      else
        log "FATAL: announced $tag but cannot save state"; sleep 60; exit 1
      fi
    else
      log "announce failed for $tag, retry next cycle"
    fi
  fi
  sleep "$INTERVAL"
done
