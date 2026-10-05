#!/usr/bin/env bash
#
# Nightly backup of wcf: database dump + data directory (uploads, media).
#   sudo -u wcf bash deploy/backup.sh /var/backups/wcf
# Cron example (3:15 every night, keeps 14 days):
#   15 3 * * * cd /opt/wcf-1.0.0 && bash deploy/backup.sh /var/backups/wcf 14
#
# Restore: createdb wcf && pg_restore -d wcf db-<stamp>.dump
#          tar xzf data-<stamp>.tar.gz -C /opt/wcf-1.0.0
set -euo pipefail
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DEST="${1:-$ROOT/backups}"
KEEP_DAYS="${2:-14}"
STAMP="$(date +%Y%m%d-%H%M%S)"
DB_URL="$(sed -n 's/^DATABASE_URL=//p' "$ROOT/.env" | head -n1)"
[ -n "$DB_URL" ] || { echo "DATABASE_URL not found in $ROOT/.env" >&2; exit 1; }
mkdir -p "$DEST"
pg_dump --format=custom --file="$DEST/db-$STAMP.dump" "$DB_URL"
# Packaged watch media can be re-packaged from its sources, and movie sources
# (data/watch/incoming or WATCH_IMPORT_DIR) are usually too big for nightly
# backups – back those up separately if needed.
tar -C "$ROOT" --exclude='data/watch/media' --exclude='data/watch/incoming' --exclude='data/tmp'   -czf "$DEST/data-$STAMP.tar.gz" data
find "$DEST" -name 'db-*.dump' -mtime +"$KEEP_DAYS" -delete
find "$DEST" -name 'data-*.tar.gz' -mtime +"$KEEP_DAYS" -delete
echo "backup written to $DEST ($STAMP)"
