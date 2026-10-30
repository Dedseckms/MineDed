#!/usr/bin/env python3
"""Run hourly: disk 80/95%, backup errors, new public-chat reports -> owner Telegram."""
import json
import os
from pathlib import Path
import shutil
import sqlite3
import sys
import urllib.request


def main():
    root = Path(os.environ['HUB_CHAT_DIR']).resolve(strict=True)
    db = sqlite3.connect(f'file:{root / "chat.sqlite3"}?mode=ro', uri=True)
    row = db.execute("SELECT value FROM settings WHERE key='files'").fetchone()
    files = Path(row[0]) if row else root / 'files'
    disk = shutil.disk_usage(files)
    percent = disk.used * 100 // max(1, disk.total)
    reports = db.execute('SELECT COALESCE(max(id),0) FROM reports WHERE resolved=0').fetchone()[0]
    db.close()
    backup_path = root / 'backup-status.json'
    backup = json.loads(backup_path.read_text()) if backup_path.exists() else {}
    state_path = root / 'notify-status.json'
    old = json.loads(state_path.read_text()) if state_path.exists() else {}
    current = {'disk': 95 if percent >= 95 else 80 if percent >= 80 else 0,
               'report': reports, 'backupError': bool(backup.get('backupError'))}
    messages = []
    if current['disk'] and current['disk'] != old.get('disk'):
        messages.append('Chat storage is >= %s%% full.%s' % (current['disk'], ' New attachments are paused.' if percent >= 95 else ''))
    if reports > old.get('report', 0):
        messages.append('New public chat reports need moderation.')
    if current['backupError'] and not old.get('backupError'):
        messages.append('Offsite chat backup failed. Inspect the backup job.')
    if messages:
        body = json.dumps({'chat_id': os.environ['CHAT_TELEGRAM_OWNER_ID'], 'text': '\n'.join(messages)}).encode()
        request = urllib.request.Request('https://api.telegram.org/bot' + os.environ['CHAT_TELEGRAM_BOT_TOKEN'] + '/sendMessage', data=body, headers={'Content-Type': 'application/json'})
        with urllib.request.urlopen(request, timeout=15) as response:
            if not json.load(response).get('ok'):
                raise RuntimeError('notification rejected')
    # Commit dedup state only after successful delivery; transient failures retry.
    temp = state_path.with_suffix('.tmp')
    temp.write_text(json.dumps(current))
    temp.replace(state_path)


if __name__ == '__main__':
    try:
        main()
    except Exception:
        # URLs contain a bot token; don't print exception strings or tracebacks.
        print('chat notification failed', file=sys.stderr)
        sys.exit(1)
