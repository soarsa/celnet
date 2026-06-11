# Joint execution plan — both sessions' dynamic workflows to full implementation

**Status:** ACTIVE (2026-06-11). The binding division for everything still open, per §4.2
(planned builds, bounded concurrency, one milestone gate per merge window) and the operator
directives ([[planned-builds-no-blocking]], graph-first optimization). Edit via board claim
rules: each row is claimed/edited only by its owner; §6 carries progress pings.

## 1. Scope inventory → ownership (disjoint by construction)

| # | Unit | Owner | Worktree/branch | Files (disjointness) | Workflow shape |
|---|------|-------|-----------------|----------------------|----------------|
| A1 | risk-cube mutation finish (final config run) | **session-A** | `celnet-w6a` / `lane/w6-analytics` | `.config/mutants-celnet-risk-cube.toml` + risk-cube tests | 1 agent, bankable |
| A2 | surface mutation waves (rest incl. calibrate.rs cluster audit) | **session-A** | `celnet-w6a` | `mutants-surface.toml` + surface tests | 1 agent, per-wave commits |
| A3 | exotics E1/E2 + full-crate run (minus `touch.rs`) | **session-A** | `celnet-w6x` / `lane/w6-exotics` | `mutants-exotics.toml` + exotics tests | 1 agent, per-wave commits |
| A4 | crypto-surface leaf + neutral core | **session-A** | `celnet-csurf` / `lane/crypto-surface` | `celnet-surface/src/strike_quotes.rs` (new) + spec'd seam | 1 agent, per-stage commits |
| A5 | exotics `touch.rs` mutation wave | **session-A** | `celnet-w6x` | `touch.rs` tests | AFTER B2 merges (file owned by P0 fix until then) |
| B1 | arms-30/31 verify → land → **§6 gate-ledger entry** | **session-B** | their tree | proto/cli/server/clients/golden (their lane) | their workflow |
| B2 | P0 one-touch fix → set `lane/p0-touch` READY-FOR-MERGE | **session-B** | `celnet-p0touch` | `exotics/touch.rs` + golden + gui/pricing.ts | their verify, A merges |
| B3 | Round-2 remainder (their pick, posted to board before starting): entitlements trust-boundary, excel strategy-family, event-weighted clock | **session-B** | new lanes | disjoint from A1–A5 by file list | their workflows |
| J1 | Merge sequencing + the ONE milestone gate + ledger | **joint** | — | — | §3 below |
| J2 | GA-readiness delta + deploy-milestone tag | **joint** | — | — | after J1 green |

Unassigned Round-2 leftovers (QMC pathwise wiring, PnL attribution, rates leaf, normal.rs
Acklam refit w/ byte-identity re-freeze) → next window; tracked in WORLD-CLASS-BACKLOG.

## 2. Safety rails (both sessions' workflow agents inherit these)

- **Disjointness is by file list above** — an agent touching a file outside its row STOPS and
  posts NEEDS-COORDINATOR to §6. `touch.rs` is B2's until merged (A skips it, config documents it).
- **Bounded compute:** each session ≤2 concurrent heavy tasks, `--jobs 2`, never nextest,
  scoped builds only (`-p`/`--file`/`check-changed`). No workspace-wide cargo in lane steps.
- **Bankable units:** agents commit per wave/stage BEFORE summarizing; session-A's banker
  pushes lane branches to origin every 5 min. A crash/limit loses ≤1 unit's tail.
- **Graph-first:** agents scope gates via `detect_changes`, find code via `search_graph` —
  no grep/file-dump exploration (token economy + speed).
- **Session limits are expected:** on hitting one, bank → post §6 state → resume from
  workflow cache at reset. Never leave uncommitted work >1 unit.

## 3. Merge + gate sequencing (one full gate this window)

1. **B1 lands first** (arms-30/31 is the open proto window — owns the contract). session-B
   posts the §6 **gate-ledger entry** for it (their full verify = the window's milestone gate).
2. **B2 → READY-FOR-MERGE**; session-A (coordinator) merges `lane/p0-touch`, runs
   `check-changed` only (covered by B1's ledger + the fix's own oracle suite).
3. **A1–A4 merge** as each lane completes adversarial verify: rebase over main,
   `check-changed` + ledger reference — no second full gate.
4. **A5 runs post-B2-merge**, merges on `check-changed`.
5. If anything merges AFTER the ledger entry that the ledger didn't cover (cross-crate risk),
   the LAST merger runs the single closing gate + posts a fresh ledger line. → **J2**:
   GA-readiness delta vs `docs/GA-READINESS.md`, deploy-milestone tag, ledger + anchor update.

## 4. Verification (every unit)

Build → adversarial-verify stays the law: mutation lanes get the laundering-hunt verifier;
the crypto leaf gets the oracle-independence + seam-discipline verifier; B-lanes get their
own (session-B runs the same pattern). ACCEPT verdicts recorded in commit messages or §6.

## 5. Verification economy (operator-directed, 2026-06-11) — verify only what's needed

Exhaustive full-crate mutation sweeps re-prove what the frozen-bits/oracle suites already
kill by construction. The triage criterion — **verification is NEEDED where**:
1. **New code this window** (the crypto leaf) → full build→adversarial-verify. 
2. **A finding flagged weakness** (Round-2 items) → targeted oracle + kill tests.
3. **No independent oracle/frozen-bits coverage exists** for the file → file-scoped mutants.

**Everywhere else** (files whose arithmetic is pinned bit-for-bit by `fx_byte_identity`-class
suites or solver/fit pins): the pin IS the kill mechanism. Validate it with a per-file
**sample** (not a sweep), record the coverage rationale inline in the mutants config, move on.
Already-measured gates (qmc/xva/risk-cube: zero-missed full runs) are DONE — never re-run
them in later windows; the config + ledger entry is the record. Merges ride `check-changed`
+ the window's single gate ledger (§3). Partial mutants outcomes are always resumed with
`--iterate`, never re-ground.
