# tg-bot: release announcements to Telegram

Runs on the VPS in `/opt/tg-bot` (Telegram is blocked from Russia, the VPS reaches it).
Container `mineded-tg-bot` (`node:22.20.0-alpine3.22`, 64M, no ports, log rotation 3x1M)
executes `loop.sh` every 10 min:

1. `GET api.github.com/repos/Dedseckms/MineDed/releases/latest` (unauthenticated).
2. If `tag_name` differs from the state file (`tg-bot_state` volume, `last_tag`), download
   `CHANGELOG.md` and `scripts/telegram-changelog.mjs` from raw.githubusercontent.com at the tag
   (fallback `main` for old tags) and run `node telegram-changelog.mjs <tag>`.
3. Save the tag only on success. Without a readable/writable state file the loop exits
   (never posts), so a broken volume cannot cause repeated posts.

Secrets: `/opt/tg-bot/.env` (0600, `TELEGRAM_BOT_TOKEN`, `TELEGRAM_CHAT_ID`), server only.

## Deploy / update

```sh
cd /opt/tg-bot            # docker-compose.yml, loop.sh, .env
docker compose up -d      # init-state chowns the volume, then the bot starts
docker logs --tail 20 mineded-tg-bot
```

Dry run (prints the message, posts nothing):
`docker compose run --rm --no-deps tg-bot sh /app/loop.sh --dry-run v1.2.1`

Re-post or skip a release: write the desired tag into the state
(`docker run --rm -u node -v tg-bot_state:/home/node/state node:22.20.0-alpine3.22 sh -c 'echo vX.Y.Z > /home/node/state/last_tag'`).

Rollback: `docker compose down` (state volume is kept).
