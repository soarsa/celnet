#!/bin/sh
# Shared non-blocking incremental lodestar re-index (absolute path — relative '.' corrupts
# the projects table). Backgrounded so no git op ever blocks. The FS watcher also keeps fresh;
# these hooks guarantee bulk git ops (merge/checkout/rebase) are caught even with no live session.
command -v lodestar >/dev/null 2>&1 || exit 0
ROOT="$(git rev-parse --show-toplevel 2>/dev/null)" || exit 0
nohup sh -c "lodestar index '$ROOT' >> '$ROOT/.lodestar/index.log' 2>&1" >/dev/null 2>&1 &
exit 0
