#!/usr/bin/env python3
"""Rebuild the lodestar verified-knowledge projection from the committed anchored mirror.

WHY THIS EXISTS
---------------
lodestar's durable in-tree event log (`.lodestar/knowledge/events/*.json`) records claim
text + kind + verdict + state transitions, but it does NOT persist anchors
(qualified_name / node_content_hash) — those live only in the machine-local SQLite
projection (`~/.cache/lodestar/<key>.db`). There is currently no lodestar command that
re-projects the durable log into an *anchored* claim set, so a fresh `git clone` (log
present, projection absent) — or any projection loss (e.g. `delete_project`) — yields
ZERO live claims even though the "why" is committed.

`claims-mirror.json` closes that gap: it is the git-committed, anchor-carrying source of
truth. This script replays it through `lodestar cli knowledge_put`, deterministically
rebuilding the local projection. Run it after a fresh clone, or any time
`knowledge_coverage` shows fewer claims than the mirror.

Upstream tracking: filed as a lodestar issue (durable log must fully rehydrate the
anchored projection; persist anchors in the log; delete_project must not orphan).

USAGE
-----
    python3 tools/lodestar/replay-knowledge.py            # replay all
    python3 tools/lodestar/replay-knowledge.py --dry-run  # show what would replay

Idempotent: claim keys are content-addressed, so re-running converges (no duplicates).
"""
import json
import os
import subprocess
import sys

REPO = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
MIRROR = os.path.join(REPO, ".lodestar", "knowledge", "claims-mirror.json")
PROJECT = "github.com-soarsa-celnet"
# FIXED, machine/developer-neutral author so the claim key is identical on every machine —
# this is what makes replay idempotent (re-running converges instead of duplicating). Do NOT
# read LODESTAR_AUTHOR here: a per-developer author would mint a distinct claim key per person.
AUTHOR = "celnet-knowledge"


def main() -> int:
    dry = "--dry-run" in sys.argv
    claims = json.load(open(MIRROR, encoding="utf-8"))
    print(f"mirror: {len(claims)} claims -> project {PROJECT}")
    active = draft = err = 0
    for i, c in enumerate(claims, 1):
        anchors = [{"qualified_name": a} for a in c["anchors"]]
        if dry:
            print(f"[{i}/{len(claims)}] {c['kind']:18} {c['anchors'][0] if c['anchors'] else '-'}")
            continue
        arg = {"project": PROJECT, "kind": c["kind"], "text": c["text"],
               "anchors": anchors, "author": AUTHOR}
        out = subprocess.run(["lodestar", "cli", "knowledge_put", json.dumps(arg)],
                             capture_output=True, text=True)
        parsed = None
        for line in out.stdout.splitlines():
            line = line.strip()
            if line.startswith("{"):
                try:
                    parsed = json.loads(line)
                except json.JSONDecodeError:
                    pass
        state = parsed.get("state") if parsed else "ERROR"
        active += state == "active"
        draft += state == "draft"
        err += state not in ("active", "draft")
        print(f"[{i}/{len(claims)}] {state:8} {c['kind']:18} {c['anchors'][0] if c['anchors'] else '-'}", flush=True)
    if not dry:
        print(f"\nreplayed: active={active} draft={draft} other={err} / {len(claims)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
