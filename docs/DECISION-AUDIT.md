# Decision audit — why a rule fired, or did not

As-built. The audit trail over CelNet's three trader-configurable decision engines, and
the rule advisor derived from it.

## 1. The problem

Three graphs gate inbound flow:

| Engine | Crate | Decides |
|---|---|---|
| Acceptance | `celnet-acceptance` | Accept / Reject / HoldForReview on an inbound lift |
| Risk routing | `celnet-risk-routing` | Which risk book an accepted fill lands in |
| Hedge exit policy | `celnet-hedge-routing` | The exit action when a warehouse band is breached |

Each left a trail when it **acted** — a booked `Deal`, a stamped `HedgeProvenance` with its
walked `policy_path`. None left anything when it **did not**. A green-band book, a graph
that walked to a `WAREHOUSE` leaf, a kill-switched desk, a rate guard that downgraded a
live fire to advisory, a lift accepted only because nothing was configured: all returned
silently. "Why did my hedge rule not fire?" had no answer anywhere in the system.

## 2. What was added

### 2.1 The walked path, everywhere

`celnet_hedge_routing::Resolution` already carried `path`. `celnet_acceptance::
AcceptanceOutcome` now carries `path` too (the same shape, including the *partial* walk
taken before a structurally broken graph gave up). Risk routing's `RiskRouter::route` still
returns only the resolved book — see §5.

### 2.2 The decision journal

`crates/celnet-server/src/services/decision_journal.rs` — a bounded, FIFO-evicting ring of
`DecisionRecord`. **Every** evaluation lands here, `FIRED` and `NO_ACTION` alike, carrying:

* the walked `policy_path` (empty when no graph was walked — `reason` then says why);
* a **stated `reason`** (structurally required: `decision()` takes it as an argument, so a
  no-action row cannot be built without one);
* the `scope` that governed (`firm`, `book:…`, `bucket:…`, or the
  `SCOPE_NONE` marker meaning *no policy was authored at any scope*);
* the risk state at that instant (metric, net risk, threshold, utilisation, band);
* the join keys — `trace_id` (to `TraceEvent`), `position_id` (to the fill), `hedge_id`
  (to the realised `HedgeProvenance`), `request_id` (to the desk request).

`seq` is process-monotonic and is the evidence-citation key.

### 2.3 Emission sites

| Site | Rows emitted |
|---|---|
| `AutoHedgeEngine::evaluate` — kill-switch / desk disabled | `NO_ACTION` / `HALTED`, empty path |
| …— graph did not resolve | `NO_ACTION` / `UNRESOLVED`, empty path |
| …— `WAREHOUSE` leaf | `NO_ACTION` / `WAREHOUSE`, **with** the walked path |
| …— any other action | `FIRED` / the action kind, with the path + the minted `hedge_id`; the guard-downgrade reason rides on `reason` |
| `RfqDeskEdge::evaluate_fix_acceptance` — no policy configured | `FIRED` / `accept`, reason "accepted by default" |
| …— unknown desk request | `FIRED` / `accept`, reason naming the miss |
| …— a real walk | `FIRED`/`accept` or `NO_ACTION`/`reject`\|`hold`, with the path |

The engine's row is also returned on `HedgeOutcome::decision`, so an engine with no journal
attached (the unit tests) still reports exactly what it decided.

### 2.4 The wire

One contract, no versioning:

* `rpc ListDecisionJournal(ListDecisionJournalRequest) returns (ListDecisionJournalResponse)`
  — filters (engine, outcome, book, instrument, counterparty, since, limit) AND together;
  the reply carries `total_recorded` and `evicted`.
* `rpc ListRuleAdvice(ListRuleAdviceRequest) returns (ListRuleAdviceResponse)` — the derived
  suggestions plus `rows_considered`.

Both are gated on `hedge × FixedIncome`, the same gate `ListHedgeProvenance` uses, and are
mirrored on the WS transport as `list_decision_journal` / `list_rule_advice`.

### 2.5 The rule advisor

`crates/celnet-server/src/services/rule_advisor.rs`. Derivations, all counted over recorded
rows with a floor of 3 occurrences:

| Kind | Signal | Editor |
|---|---|---|
| `hedge_policy_missing` | rows whose scope is `SCOPE_NONE` | Hedging |
| `hedge_policy_broken` | `UNRESOLVED` rows | Hedging |
| `desk_halted` | `HALTED` rows | Hedge config |
| `hedge_warehouses_hot_book` | `WAREHOUSE` leaf reached at amber/red (and a policy *is* authored) | Hedging |
| `hedge_guard_suppressing` | rows whose reason contains the `→ advisory` downgrade | Hedge config |
| `hedge_vehicle_unresolved` | rows whose reason names an unresolved vehicle | Hedge vehicles |
| `acceptance_hold_recurring` | repeated `hold` verdicts, grouped by counterparty | Acceptance |
| `acceptance_reject_recurring` | repeated `reject` verdicts, grouped by counterparty | Acceptance |
| `routing_unrouted` | rows whose reason names a missing routing policy | Risk routing |

Every suggestion carries `occurrences` (the true count), `first_seen`/`last_seen`, and
`evidence_seqs` (a capped sample of the exact rows). A green-band warehouse hold is
deliberately **not** advice — the policy working correctly is not a finding.

### 2.6 The surface

`gui/src/workspaces/audit/` — reached at **Risk → Audit**.

* A sortable, filterable table of the recorded rows (engine/verdict chips, server-side book
  filter, client-side free text, total sort so rows never reshuffle).
* An inspector: the walked path replayed step by step against the graph **as it stands
  now** — a node the graph no longer contains is flagged stale, never relabelled — plus the
  utilisation-against-band series for that `(book, instrument)` cell, built only from
  recorded rows.
* The advice rail, each card citing its rows; "Show the N cited rows" filters the table to
  exactly that evidence.
* In-app help: `concept.decision-audit` in `gui/src/lib/help.ts`.

## 3. Honesty rules encoded in the code

1. A `NO_ACTION` row **cannot be constructed without a reason** (the builder's signature).
2. `evicted > 0` renders a prominent "this window is INCOMPLETE" banner naming the loss. A
   bounded ring that has rolled must never read as a complete audit log.
3. The offline mock transport returns an **empty** journal and **empty** advice. A mock
   cannot invent an audit trail.
4. The WS codec **throws** on an unknown engine/outcome ordinal rather than decoding to a
   plausible arm.
5. A walked node the current graph no longer contains is reported `stale` with a `null`
   label.
6. A failed query renders as failed, never as "no decisions were taken".

## 4. Retention — the honest limitation

The journal is a **bounded, in-process** ring (`DEFAULT_JOURNAL_CAPACITY = 16 384`), like
the hedge provenance ring (4 096) and the lift trace store (4 096). It is a recent-history
control-plane window, **not** a durable compliance archive:

* it does not survive a process restart;
* it rolls under sustained load;
* it is per-process, so a scaled-out deployment has one window per node.

The surface states this whenever rows have been evicted. A durable audit store (append-only,
queryable across restarts and nodes) is **not built** and would be a separate deliverable —
the natural shape is to tee `DecisionJournal::record` onto the existing `celnet-replog`
append-only log and serve historical queries from there, leaving the ring as the hot path.

## 5. Deliberately not built

* **Risk-routing journaling.** `RiskRouter::route` returns only the resolved book id, not
  the walked path, and the two call sites (`rates_book.rs`, `risk/store.rs`) sit inside the
  booking write path. The proto enum, the filters and the advisor derivation for
  `DECISION_ENGINE_RISK_ROUTING` are in place; the emission is not. Adding it means
  giving `RiskRouter::route` a `Resolution`-shaped return (as `HedgeRouter` and now
  `AcceptanceEngine` have) and threading the journal into both stores.
* **Counterparty adverse-selection skew advice** ("this name's fills are consistently
  followed by an adverse mark move ⇒ skew them"). This needs a post-fill mark trajectory per
  fill — the mid at `t+n` for several `n`. Nothing retains that today: the journal records
  the state *at* the decision, the deal blotter records the dealt level, the aggregation
  book keeps only the live top-of-book. Rather than fabricate a plausible-looking skew
  recommendation from data that does not exist, no such suggestion is emitted. Building it
  requires a post-fill mark sampler keyed on `position_id` first.
