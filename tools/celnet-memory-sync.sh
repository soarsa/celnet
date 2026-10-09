#!/usr/bin/env bash
# tools/celnet-memory-sync.sh — mirror the per-machine Agent auto-memory into the
# git-committed repo so durable facts/decisions SURVIVE /clear, session termination and
# restart, and are SHARED across developer PCs.
#
# WHY: the live auto-memory lives at ~/.agents/projects/<slug>/memory/ where <slug> is the
# repo's absolute checkout path with '/' -> '-'. It is therefore per-developer + per-machine
# and NOT in git — a /clear does not touch it, but a fresh clone, a new machine, or a wiped
# ~/.agents loses it entirely, and it is never shared between developers. This script keeps a
# committed mirror at .celnet/memory/ (private repo) and syncs both ways.
#
#   restore   (fail-safe; SessionStart hook) — copy any mirror memory file MISSING from the
#             live dir into it, bootstrapping a fresh clone/machine. NEVER clobbers a file
#             that already exists locally (a locally-newer memory is preserved).
#   snapshot  — make the committed mirror match the live dir EXACTLY (adds/updates + drops
#             removed files) and `git add` it. The caller then commits + pushes to share.
#
# Both modes exit 0 on any internal error so a hook can never block the session.
set -uo pipefail

REPO_ROOT="$(git rev-parse --show-toplevel 2>/dev/null || pwd)"
SLUG="$(printf '%s' "$REPO_ROOT" | sed 's#/#-#g')"
LIVE="$HOME/.agents/projects/$SLUG/memory"
MIRROR="$REPO_ROOT/.celnet/memory"
MODE="${1:-restore}"

case "$MODE" in
  restore)
    [ -d "$MIRROR" ] || { echo "celnet-memory-sync: no mirror at $MIRROR — nothing to restore"; exit 0; }
    mkdir -p "$LIVE" 2>/dev/null || { echo "celnet-memory-sync: cannot create $LIVE"; exit 0; }
    n=0
    for f in "$MIRROR"/*.md; do
      [ -e "$f" ] || continue
      base="$(basename "$f")"
      if [ ! -e "$LIVE/$base" ]; then cp "$f" "$LIVE/$base" 2>/dev/null && n=$((n + 1)); fi
    done
    echo "celnet-memory-sync: restored $n missing memory file(s) into $LIVE"
    ;;
  snapshot)
    [ -d "$LIVE" ] || { echo "celnet-memory-sync: no live memory dir at $LIVE — nothing to snapshot"; exit 0; }
    mkdir -p "$MIRROR" 2>/dev/null || { echo "celnet-memory-sync: cannot create $MIRROR"; exit 0; }
    if command -v rsync >/dev/null 2>&1; then
      rsync -a --delete --include='*.md' --exclude='*' "$LIVE"/ "$MIRROR"/ 2>/dev/null || true
    else
      rm -f "$MIRROR"/*.md 2>/dev/null || true
      cp "$LIVE"/*.md "$MIRROR"/ 2>/dev/null || true
    fi
    git -C "$REPO_ROOT" add "$MIRROR" >/dev/null 2>&1 || true
    echo "celnet-memory-sync: mirror updated from $LIVE ($(ls "$MIRROR"/*.md 2>/dev/null | wc -l | tr -d ' ') files) + git-add'd — commit & push to share"
    ;;
  *)
    echo "usage: $(basename "$0") restore|snapshot" >&2
    exit 2
    ;;
esac
exit 0
