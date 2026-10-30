#!/usr/bin/env python3
"""Online SQLite backup + verified offsite copy. Secrets stay in the environment."""
import datetime
import hashlib
import json
import os
from pathlib import Path
import sqlite3
import subprocess
import sys
import tempfile


def stamp():
    return datetime.datetime.now(datetime.timezone.utc).isoformat()


def main():
    root = Path(os.environ['HUB_CHAT_DIR']).resolve(strict=True)
    destination = os.environ['CHAT_BACKUP_DEST']  # restricted SSH host:/absolute/directory
    if ':' not in destination or not destination.split(':', 1)[1].startswith('/'):
        raise ValueError('CHAT_BACKUP_DEST must be host:/absolute/directory')
    # Never pass arbitrary remote shell fragments through rsync's destination.
    import re
    if not re.fullmatch(r'[A-Za-z0-9_.@-]+:/[A-Za-z0-9_./-]+', destination):
        raise ValueError('invalid backup destination')
    status_path = root / 'backup-status.json'
    old = json.loads(status_path.read_text()) if status_path.exists() else {}
    try:
        with tempfile.TemporaryDirectory(prefix='chat-backup-', dir=root) as temp:
            staging = Path(temp)
            source = sqlite3.connect(root / 'chat.sqlite3', timeout=30)
            # BEGIN IMMEDIATE excludes uploads/migrations while snapshot and file list
            # are captured. There is no source deletion, so files remain available later.
            source.execute('BEGIN IMMEDIATE')
            files_row = source.execute("SELECT value FROM settings WHERE key='files'").fetchone()
            files = Path(files_row[0]) if files_row else root / 'files'
            hashes = [r[0] for r in source.execute('SELECT hash FROM attachments UNION SELECT thumb FROM attachments WHERE thumb IS NOT NULL')]
            # A separate connection sees the consistent committed WAL snapshot; don't
            # invoke sqlite3_backup from the connection holding the write transaction.
            reader = sqlite3.connect(root / 'chat.sqlite3', timeout=30)
            backup = sqlite3.connect(staging / 'chat.sqlite3')
            reader.backup(backup)
            if backup.execute('PRAGMA integrity_check').fetchone()[0] != 'ok':
                raise RuntimeError('backup integrity check failed')
            # Restore into its own directory, independent of a migrated VPS path.
            backup.execute("DELETE FROM settings WHERE key='files'")
            backup.commit()
            backup.close()
            reader.close()
            source.rollback()
            source.close()
            manifest = {}
            for digest in hashes:
                if not re.fullmatch('[a-f0-9]{64}', digest):
                    raise RuntimeError('invalid content address')
                with (files / digest).open('rb') as stream:
                    actual = hashlib.file_digest(stream, 'sha256').hexdigest()
                if actual != digest:
                    raise RuntimeError('source attachment checksum failed')
                manifest[digest] = digest
            (staging / 'manifest.json').write_text(json.dumps(manifest))
            # No --delete. Preserve all historical files; rsync checksum confirms the
            # receiver's bytes match, rather than relying on size/mtime.
            subprocess.run(['rsync', '-a', '--checksum', '--', str(files) + '/', destination.rstrip('/') + '/files/'], check=True)
            # Publish snapshot last so every referenced file is already transferred.
            subprocess.run(['rsync', '-a', '--checksum', '--', str(staging) + '/', destination.rstrip('/') + '/snapshots/' + stamp().replace(':', '-') + '/'], check=True)
        status = {'lastBackup': stamp(), 'backupError': None}
    except Exception:
        status = {'lastBackup': old.get('lastBackup'), 'backupError': 'Offsite chat backup failed; inspect the backup job on the server'}
        raise
    finally:
        # Atomic status update; never include hostnames, usernames or exception text.
        temp_status = status_path.with_suffix('.tmp')
        temp_status.write_text(json.dumps(status))
        temp_status.replace(status_path)


if __name__ == '__main__':
    try:
        main()
    except Exception:
        print('chat backup failed', file=sys.stderr)
        sys.exit(1)
