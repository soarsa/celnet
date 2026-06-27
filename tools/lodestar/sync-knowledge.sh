#!/bin/sh
# Rehydrate the lodestar verified-knowledge projection from the committed mirror IF the
# live projection is behind it (fresh clone, after a `git pull`, or any projection loss).
#
# WHY: lodestar's machine-local SQLite projection is not reconstructable from the committed
# event log alone (the log carries no anchors — lodestar#18). The git-shared source of truth
# is .lodestar/knowledge/claims-mirror.json; this script replays it via knowledge_put so a
# teammate who pulls gets the live "why" automatically. Wired into SessionStart (background),
# guarded so normal sessions are a no-op.
set -eu

command -v lodestar >/dev/null 2>&1 || exit 0
REPO="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
MIRROR="$REPO/.lodestar/knowledge/claims-mirror.json"
[ -f "$MIRROR" ] || exit 0
PROJECT="$(cat "$REPO/.lodestar/project-id" 2>/dev/null || echo github.com-soarsa-celnet)"

mirror_n="$(python3 -c "import json;print(len(json.load(open('$MIRROR'))))" 2>/dev/null || echo 0)"
# Exact live count = rows in the machine-local projection's knowledge_claim table. (We do NOT
# use knowledge_coverage.total — it counts only claims anchored to "code" nodes and structurally
# undercounts proto/variable/test anchors, which would make this replay every session.)
DB="${XDG_CACHE_HOME:-$HOME/.cache}/lodestar/$PROJECT.db"
if command -v sqlite3 >/dev/null 2>&1 && [ -f "$DB" ]; then
  live_n="$(sqlite3 "$DB" "SELECT count(*) FROM knowledge_claim WHERE project='$PROJECT';" 2>/dev/null || echo 0)"
else
  live_n=0   # no projection db yet (fresh clone) => rebuild
fi

if [ "$live_n" -lt "$mirror_n" ]; then
  echo "$(date -u +%FT%TZ) knowledge projection behind mirror ($live_n < $mirror_n) — replaying"
  python3 "$REPO/tools/lodestar/replay-knowledge.py"
else
  echo "$(date -u +%FT%TZ) knowledge projection up to date ($live_n >= $mirror_n)"
fi
