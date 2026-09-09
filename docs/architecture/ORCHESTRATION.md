# Cross-session task orchestration

> How any Claude session on this repo, on **SessionStart** or after **/clear**, autonomously
> continues the TRACKED tail tasks shared across sessions — with claim-locking so there is
> **no duplicated, conflicting, or wasted effort**. This is the L1 git backbone of the ratified
> cross-session design (auto-memory `cross-session-orchestration`). It **supersedes the live
> lane table in `docs/PARALLEL-SESSIONS.md` §5** (the manifest below is now the lock + board);
> the §6 coordinator narrative and §1–4 rules in that doc still stand.

## TL;DR

- **Manifest** `.celnet/tasks.jsonl` — git-committed, one JSON object per line, the durable
  source of truth for what's open / claimed / done.
- **CLI** `tools/celnet-task` — claim/release/progress/heartbeat/done/… ; claims are taken with
  **git-push-as-CAS** on a dedicated `coord/board` branch.
- **Hooks** (`.claude/settings.json`) — SessionStart runs the **selector** (picks + claims a task,
  injects "continue task X"); Stop **heartbeats**; PreCompact **checkpoints**. All fail-safe.
- **Done = lodestar-verified** — `done` flips status only when the deliverable's lodestar
  roll-up is `active` (see the env caveat at the bottom).

## 1. The manifest — `.celnet/tasks.jsonl`

One JSON object per line (line-diff merge-friendly; edits rewrite only the changed line).

**Task records** (have an `id`):

| field | meaning |
|-------|---------|
| `id` | short task id (e.g. `D`) |
| `title` | one-line description |
| `scope[]` | crate/file regions this task owns — **disjointness is the conflict-avoidance unit** |
| `status` | `open → claimed → in_progress → in_review → done` &#124; `blocked` |
| `owner_session` | the lock tag of the holder (`null` when open) |
| `lease_expiry` | ISO-8601 UTC; the claim is valid until this time |
| `heartbeat_at` | last lease bump |
| `deps[]` | task ids that must be `done` before this is selectable |
| `gate_tier` | `T1` (worker-claimable) &#124; `T2` (**coordinator-only**) |
| `branch` | the working branch for the task |
| `priority` | integer, **higher = more urgent** (selector picks the highest) |
| `deliverable` | the `docs/acceptance/<slug>.acceptance.json` slug used by the done-gate |
| `requires_cargo` | `true` ⇒ must hold the `cargo_lane` singleton (the single-M4 mutex) |
| `notes` | free text (real prerequisites, churn-deferrals, audit breadcrumbs) |
| `updated_at` | ISO-8601 UTC of the last mutation |

**Singleton control records** (have a `kind`, no `id`):

- `{"kind":"cargo_lane","holder","lease_expiry","purpose"}` — the **single-machine (M4) cargo
  mutex**. A `requires_cargo` task may build heavy cargo only while it holds this.
- `{"kind":"batch_window","owner":"coordinator","open","since","gate_ledger_ref"}` — the
  coordinator's merge window. `open:true` ⇒ workers pause heavy builds and trust the gate ledger
  (`docs/PARALLEL-SESSIONS.md` §4.2/§6).

The seeded tail tasks are **D** (`dormant-crate-activation`), **G** (`ws-codec-from-proto`),
**J** (`desk-session-risk-scoping`) — all T2, `requires_cargo:true`. Cross-task prerequisites
that are not themselves tracked tasks (e.g. D needs C landed) live in `notes`, not `deps`, so the
manifest stays self-consistent.

## 2. The CLI — `tools/celnet-task`

```
selector [--session ID]      pick + claim the best task, emit the SessionStart directive (fail-safe)
claim <id>                   claim a specific task (git-CAS)
release <id> [--force]       release a claim back to open (--force overrides ownership)
progress <id>                claimed -> in_progress (+ lease bump)
heartbeat [--session ID]     bump the lease on this session's active claims (fail-safe; Stop hook)
checkpoint [--session ID]    PreCompact: lease bump + breadcrumb (fail-safe)
done <id> [--attest]         lodestar-verified completion: in_review (worker) / done (coordinator)
block <id> --reason <text>   mark a task blocked
unblock <id>                 return a blocked task to open
cargo-lane <acquire|release> [--purpose <text>]   the single-M4 cargo mutex
window <open|close> [--ledger <ref>]              coordinator batch window
digest                       print the board
```

**Configuration (env, top of the script):**

| var | default | meaning |
|-----|---------|---------|
| `CELNET_ROLE` | `worker` | `coordinator` ⇒ may claim T2, finalise `done`, open/close the window |
| `CELNET_BOARD_BRANCH` | `coord/board` | the dedicated claims branch (NOT `main`) |
| `CELNET_BOARD_REMOTE` | `origin` | the one sanctioned remote |
| `CELNET_LEASE_MIN` | `90` | claim lease lifetime |
| `CELNET_GRACE_MIN` | `30` | grace past expiry before a lease is reclaimable |
| `CELNET_CAS_RETRIES` | `5` | bounded CAS push retries under contention |
| `CELNET_TASK_DRY_RUN` | `0` | `1` ⇒ compute everything, **never push to the remote** |

## 3. Claim / lease / CAS protocol

**Claim = git-push-as-CAS.** A claim never edits your working tree or current branch. The CLI:

1. `git fetch` the claims branch and read `.celnet/tasks.jsonl` off its tip.
2. Re-check the task is still claimable (`open`, already yours, or an expired lease past grace).
3. Rewrite the **one** task line (via git plumbing: `hash-object` → `read-tree`/`update-index` →
   `write-tree` → `commit-tree` with the fetched tip as the parent).
4. `git push` the new commit to the claims branch **without `--force`**. A fast-forward push
   succeeds = **lock acquired**. A non-FF rejection = someone else pushed first = **lock lost** →
   re-fetch, re-evaluate, retry (bounded). The plain ref-push *is* the compare-and-swap.

**Leases.** `LEASE = 90 min`; `heartbeat`/`checkpoint`/`progress` bump it; a lease unbumped past
`expiry + GRACE (30 min)` is **reclaimable** — the selector (and any `claim`) auto-reclaims it back
to `open` (reclaim is itself a CAS), so a crashed/walled session never wedges a task forever.

**Lock tag** = `${USER}-$(hostname -s)-${CLAUDE_SESSION_ID:0:8}` — stable per session, so a session
recognises and resumes its own claims across SessionStart / heartbeat / done.

**Selector algorithm** (the SessionStart entrypoint):

1. fetch; if the remote is unreachable → inject a **"do not start tracked work"** notice, exit 0.
2. reclaim expired leases.
3. if this tag already holds a `claimed`/`in_progress` task → **re-emit a RESUME directive** (never
   grab a second task).
4. otherwise pick the **highest-priority `open`** task whose `deps` are all `done`, whose `scope`
   is **disjoint** from every other live task's scope, that is claimable by this role
   (`T2` ⇒ coordinator), and whose `requires_cargo` is satisfiable vs the `cargo_lane` (if the lane
   is held by another, a cargo task is skipped in favour of a non-cargo one).
5. claim it (CAS) and emit the claim + directive as `hookSpecificOutput.additionalContext`.

**Fail-safe.** `selector`, `heartbeat`, `checkpoint` are hook-invoked and **never block a session**:
any internal error prints a one-line note to stderr and exits 0. Only the directive JSON goes to
stdout; all diagnostics go to stderr. `heartbeat`/`checkpoint` skip all network work when this
session owns nothing locally (the common case = zero overhead).

## 4. Hooks (`.claude/settings.json`)

Added **alongside** the existing backgrounded lodestar SessionStart/Stop hooks (never replacing them):

- **SessionStart** matcher `startup|resume|clear|compact`, **synchronous**, `timeout 60` →
  `bash tools/celnet-task selector --session "$CLAUDE_SESSION_ID"`. Foreground so its
  `additionalContext` is injected into the new/cleared/compacted session.
- **PreCompact** matcher `auto|manual` → `bash tools/celnet-task checkpoint --session …`.
- **Stop** → `bash tools/celnet-task heartbeat --session …` (added to the existing Stop hooks).

## 5. Single authority & serial cargo

- **Exactly one coordinator** (`CELNET_ROLE=coordinator`) owns merges-to-`main`, the `T2` gate, and
  the `batch_window`. Only the coordinator claims `gate_tier:T2` tasks and flips terminal `done`;
  workers advance their own task to `in_review` and hand off.
- **Serial cargo on the one M4.** A `requires_cargo` task must hold the `cargo_lane` singleton
  before heavy builds; the selector won't hand a worker a cargo task while the lane is held by
  another. Reuse the resumable gate-runner + `.gate-ledger.jsonl` (gitignored, machine-local);
  the manifest is the committed, shared layer. One T1 per batch; T2 only at landing.

## 6. Done = lodestar-verified — and the current env caveat

`done` queries the deliverable roll-up via the lodestar CLI:

```
lodestar cli knowledge_get '{"deliverable":"<slug>","project":"github.com-soarsa-celnet"}'
```

- roll-up `state == "active"` ⇒ advance (`in_review` for a worker, `done` for the coordinator).
- otherwise ⇒ **REFUSE**, printing the roll-up (failing children + counts).

**Caveat (this env):** `LODESTAR_EXECUTE_VERIFY=off` / judge=`none` keeps every roll-up **`draft`**
through the CLI even when child claims exist — so lodestar-verified `done` is effectively
unavailable here and the honest default is to **refuse**. The coordinator may record a documented
override with `--attest` (coordinator-only, prints a loud warning) — use it only after
**independently** confirming the deliverable. A task with no `deliverable` slug likewise requires
`--attest`. The seeded tail tasks D/G/J are forward specs whose roll-ups are intentionally not
active yet, so `done` correctly refuses them until they ship and the knowledge layer activates.

## 7. Bootstrap / operating notes

- The first claim seeds the `coord/board` branch from the committed manifest if it doesn't exist
  remotely; thereafter the branch carries only manifest churn (it never races code merges on `main`).
- To watch the board without claiming: `tools/celnet-task digest`.
- To dry-run the whole flow without touching the remote: `CELNET_TASK_DRY_RUN=1 tools/celnet-task …`.

## 8. Agent teams per session (how a session burns down its claim)

After the SessionStart selector hands a session its claim, the session IMPLEMENTS that task with an
**agent team** — not by hand, one file at a time:

1. **Fan out** a Workflow (or parallel `Agent` calls) scoped to the claim's **disjoint scope**.
   Right-size it: broad decomposable work (mapping, drafting, per-lane builds, verification) fans
   out; a small/precise edit stays in-session. Reference files by path — don't paste corpora
   (see the `token-and-context-discipline` memory).
2. **Adversarially verify** the team's output independently (hidden mocks/placeholders, contract
   drift, byte-identity, "is it actually done") before trusting it.
3. **Gate at the task's tier** — T0 (`cargo check -p`) per edit, ONE T1 per accumulated batch, T2
   only at a landing milestone. Never gate per-fix; accumulate.
4. **`tools/celnet-task done <id>`** (lodestar roll-up gate; coordinator `--attest` for
   deliverable-less tasks), then **re-run `tools/celnet-task selector`** for the next task —
   continuous collaborative burn-down across all sessions.

**Exactly one coordinator.** Precisely one session sets `CELNET_ROLE=coordinator`; it owns
merges-to-`main`, the T2 gate, the `batch_window`, and flipping terminal `done`. Every other
session runs as a worker (the default) on its disjoint claim and pushes a branch / marks
`in_review`; the coordinator lands it. Two coordinators ⇒ racing merges — don't.

**Single-machine cargo.** Co-located sessions share the `cargo_lane` mutex (one heavy build at a
time); non-cargo lanes (docs / viz / knowledge / front-end) run fully in parallel around it.
Cross-*machine* sessions parallelize cargo via separate lanes off the shared board.
