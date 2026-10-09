---
name: cross-session-orchestration
description: "✅ LANDED + LIVE on origin/main (ba35a01) 2026-06-30: autonomous cross-session task continuation — .celnet/tasks.jsonl + tools/celnet-task CLI (git-CAS on coord/board) + SessionStart/PreCompact/Stop hooks + done=lodestar roll-up. Any session auto-claims+continues the next unblocked task, no dup/conflict. Tail D/G/J seeded. Set CELNET_ROLE=coordinator to claim T2."
metadata: 
  node_type: memory
  type: project
  originSessionId: 905eb293-4d97-4ffa-9bf5-b9e251021c51
---

**Goal (operator):** ANY agent session on this repo, on SessionStart or after /clear, autonomously continues TRACKED tasks shared across the team, updating status as it goes, with claim-locking so there is NO duplicated/conflicting/wasted effort. Free to change anything; June-2026 SOTA.

## Recommended design — LAYERED HYBRID (git backbone mandatory)
- **L1 backbone (mandatory):** git-committed manifest + **git-push-as-CAS** claim/lease lock + **synchronous SessionStart hook** (matcher `startup|resume|clear|compact`) that selects→claims→injects "continue task X"; `Stop` heartbeat + `PreCompact` checkpoint; lease-expiry reclaim for crashes. Only this satisfies cross-session/cross-machine/post-clear durability.
- **L2 accelerator (optional):** Agent **Agent Teams / Swarm** (native, shipped Feb-2026) for in-window burst fan-out — BUT it's host-local `~/.agents`, one-team-per-session, no-resume → NOT the source of truth; its teammates still claim/complete via the git manifest; its `TaskCompleted` hook runs the lodestar done-gate.
- **L3 (optional):** Agent Remote `create_trigger` for unattended scheduled drive (fresh session runs the selector).

## THE CONTRACT — ratify with coord/board-ops BEFORE either writes code (coord/board-ops has 0 commits ahead of main + an 18-day-stale base `8ccce28` → no fork risk yet, but lock the schema now)
1. **Manifest:** `.celnet/tasks.jsonl` (git-committed, ONE json/line — line-diff merge-friendly; replaces the conflict-prone PARALLEL-SESSIONS.md §5 table; §6 narrative + §1–4 rules stay). Per task: `{id,title,scope[],status(open→claimed→in_progress→in_review→done|blocked),owner_session,lease_expiry,heartbeat_at,deps[],gate_tier(T1|T2),branch,priority,deliverable,requires_cargo,notes,updated_at}`. Singletons (kind field): `{kind:"cargo_lane",holder,lease_expiry,purpose}` (the M4 mutex) + `{kind:"batch_window",owner:"coordinator",open,gate_ledger_ref}`.
2. **CLI:** `tools/celnet-task` with `selector|claim|release[--force]|progress|heartbeat|checkpoint|done|block|unblock|cargo-lane|window|digest`.
3. **Claims integration branch:** a dedicated `coord/board` branch (recommended) so manifest churn never races code merges on main. git-CAS: claim = commit+push the one line; non-FF push = the lock; loser `pull --rebase` + re-select. LEASE 90m / HEARTBEAT ~15m (Stop-hook) / GRACE 30m.
4. **Done = lodestar-verified:** `celnet-task done` calls `knowledge_get deliverable=<task.deliverable>`; flips to `done` ONLY if the roll-up is `active` (every child claim active; floored-at-stale if any acceptance target unreadable → never a silent pass). Needs `docs/acceptance/<slug>.acceptance.json` per task (only `carry-seam`+`rfq-multidealer` exist → author one per tracked task; `deliverable:null` ⇒ weaker coordinator-attested done).

## Hooks (literal, add alongside the existing backgrounded lodestar hooks; the selector is FOREGROUND, emits hookSpecificOutput.additionalContext)
SessionStart `startup|resume|clear|compact` → `bash tools/celnet-task selector --session "$CELNET_SESSION_ID"` (timeout 60); PreCompact `auto|manual` → `celnet-task checkpoint`; Stop → `celnet-task heartbeat`. (Agent-Teams mode also: TaskCompleted → `celnet-task verify-done` exit-2-to-block.)

## Single-authority / serial-M4 (encode, don't just document)
ONE coordinator owns merges-to-main + T2 (only it claims gate_tier:T2 + flips terminal done post-merge). `requires_cargo` tasks must hold the `cargo_lane` singleton (selector refuses a cargo task when held by another → offers a non-cargo one). `batch_window` open ⇒ workers pause heavy builds, trust the gate-ledger. Reuse the resumable gate-runner + `.gate-ledger.jsonl` (stays gitignored; the manifest is committed).

## Sequencing
Repo is locked by the A–L t2 now → BUILD this post-landing (repo free) OR the parallel session builds on a **rebased** coord/board-ops (must rebase 8ccce28→main 682eefd first). Coordinator (this session) reconciles + LANDS it. Full research (candidates, citations, failure-modes, selector pseudocode) in agent a37493108a73c6541's deliverable (this session's transcript). Sources incl. agent-docs/docs/en/agent-teams, the session-lifecycle-hooks guide, distributed work-leasing 2026.

## Deliverable done-gate — BLOCKED on lodestar#19 (diagnosed 2026-06-30, loop iter 2)
The `celnet-task done` roll-up gate (`knowledge_get deliverable=<slug>`) **cannot go green offline** in lodestar 0.9.0. Root cause (was mis-attributed to EXECUTE_VERIFY=off): a `spec:satisfies` claim's `design-target:docs/acceptance/<slug>.acceptance.json` sentinel anchor reports `design target unreadable: the sentinel anchor's ref must name a committed file under the repo root` **even though the file IS committed at HEAD under the repo root** (`git cat-file -e HEAD:<path>` passes). Every deliverable roll-up returns `children:0, unreadable_targets:15` (incl. the *landed* carry-seam), and `knowledge_put` of all 11 mirror `spec:satisfies` claims errors RC=1. Likely cause: the validator checks lodestar's indexed *code*-file table (docs/*.json has no File node → `query_graph` 0 rows), not git. Filed 0.9.0 evidence on **soarsa/lodestar#19** (issue pre-existed). **Until it ships: deliverable done-ness is coordinator-attested (`celnet-task done --attest`), NOT roll-up-verified.** The 4 deferral tasks (lode-sentinel[blocked], G-full, D-xva, D-replog) are on coord/board `5fc96b8`. Also: the live knowledge projection is decoupled from the committed event log (coverage=1186 claims vs mirror=1645; replay writes events but the projection doesn't fold them → lodestar#18).
