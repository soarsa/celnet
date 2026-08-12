# Corporate-Action Monitor & Dynamic Bond Pricing — Gap Analysis / Design

> Status: **gap analysis + design for review** (2026-08-12). Reconciles an **externally-supplied**
> specification ("Corporate Action Monitor & Dynamic Bond Pricing Integration", 3pp, undated,
> marked confidential to a third party) against Celnet **as built on `feature/hedging-model`**.
> Every "already exists" claim is `file:line`-cited and was read, not inferred.
>
> **This is a reconciliation, not a greenfield spec.** A large majority of the source spec's
> pipeline is already implemented. This doc EXTENDS
> [`BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md`](BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md)
> (the sourcing/ingestion layer) — it does not restate it. Read that doc for the CA standards,
> the open-vs-commercial sourcing verdict, and the canonical data model.
>
> **Provenance caveat.** The source PDF carries a third-party confidentiality marking. Its
> *requirements* are reconciled here; none of its text is reproduced verbatim, and no Celnet
> identifier is derived from it (guardrail 8).

---

## 0. Verdict in one paragraph

The source spec describes a four-stage pipeline (feed → monitor service → security master →
pricing/analytics), four event families, and two risk guardrails. **Stages 1–3 and three of the
four event families are already built and wired to a GUI.** `celnet-corpactions` models the
ISO 15022/20022 CAEV vocabulary with hand-verified effect math; `celnet-refstore` is a
journal-backed, bitemporal golden source with a vendor-neutral ingest port and a full
announce→elect→confirm→apply→reverse lifecycle; a `CorporateActionsService` exposes four RPCs
and a Corporate Actions workspace ships in the GUI. **What is genuinely missing is the
back half of the spec's title: the "dynamic bond pricing".** An applied corporate action
updates the golden-source schedule and books a position leg, but **never reaches the live
pricer** — the instrument the pricer values is still built from the static curated universe.
Alongside that: no yield-to-worst/yield-to-call anchor switch, no pool factor in the pricing
crates, no consent-fee event shape, no valuation of an exchange target, no per-instrument
quote lock, and no ex-date-aware accrued. Seven concrete gaps, listed in §2.

---

## 1. What already exists (cited, read not inferred)

### 1.1 The event model and effect math — `celnet-corpactions` (1,472 lines, complete)

- **Event vocabulary.** `Caev` (`crates/celnet-corpactions/src/event.rs:18-49`) — 10 ISO CAEV
  variants: `Redm`, `Intr`, `Mcal` (full call), `Pcal` (partial call), `Pred`, `Draw`
  (sinking-fund), `Bput` (put), `Tend`, `Exof`, `Conv`. Mandatory/voluntary indicator `Camv`
  (`event.rs:96-106`); lifecycle `CaStatus` `Announced→Elected→Confirmed→Applied` (+`Reversed`,
  `Cancelled`) with legality gated by `CaStatus::can_transition_to` (`event.rs:149-162`).
- **Lifecycle dates including ex-date.** `CaDates` (`event.rs:165-178`) carries `announcement`,
  `record`, **`ex`**, `response_deadline`, `payment`. The ex-date is *modelled* — it is simply
  not consumed by the accrued-interest calculation (gap G7).
- **Economic terms.** `CaTerms` (`event.rs:184-198`) — `cash_per_100`, `redeemed_fraction`,
  `target_instrument`, `target_units_per_100`, all per-100-face to match the schedule convention.
- **The effect transform.** `apply_event` (`effect.rs:123-142`) is a pure
  `(BondSchedule, CaEvent) → (BondSchedule, PositionEffect)` dispatch: `Intr`→`Income`,
  `Redm`→`Realise`, full call/put/tender→`Realise`, partial (`Pcal`/`Pred`/`Draw`/partial
  `Bput`/`Tend`)→`Scale`, `Exof`/`Conv`→`Exchange`. `position_delta` (`effect.rs:254-290`)
  converts the per-100 shape to a concrete face/cash delta on a holding (short-safe).
- **Pool factor is already the scaling carrier.** `BondSchedule.pool_factor`
  (`schedule.rs:85-88`); `BondSchedule::scaled` (`schedule.rs:239-252`) multiplies every flow
  *and* the pool factor by the retained fraction — this **is** a principal write-down.
  Notional conservation is property-tested (`tests/oracle.rs:291-309`).
- **Validation is real** (guardrail 5): a 12-case hand-computed oracle plus property tests
  (`tests/oracle.rs`), including full call, call-on-coupon-date, partial call, sinking draw and
  voluntary tender (`tests/oracle.rs:97-198`).

### 1.2 The security master — `celnet-refstore`

- **The golden record.** `InstrumentMaster` (`crates/celnet-refstore/src/master.rs:120-131`):
  `instrument_id`, `external_ids` (FIGI/ISIN/LEI/CUSIP, `master.rs:16-29`), `terms:
  InstrumentTerms` (`master.rs:47-71`), **`schedule: BondSchedule`** (carrying the pool factor),
  and bitemporal `provenance` (`master.rs:78-95`).
- **Durable and mutable at runtime.** `GoldenSourceStore` (`store.rs:75-245`) is append-only and
  `celnet-journal`-backed; written via `upsert_instrument`/`record_corp_action`
  (`store.rs:112-128`). Not load-only.
- **The vendor-neutral ingest port already exists.** `RefDataSource` / `CorpActionSource`
  (`crates/celnet-refstore/src/source.rs:40-66`) — **this is precisely the purpose-named,
  vendor-neutral port the source spec's "feed provider" stage requires. It does not need to be
  designed; it needs a second implementation.** Today the only implementation is `GovvieSource`
  (`source.rs:76-281`), which *derives* the deterministic govvie set (INTR + REDM) from
  `celnet_refdata::government_universe()`. No second `impl CorpActionSource` exists in-tree.
- **The full lifecycle is implemented.** `lifecycle.rs` (316 lines) drives
  announce→elect→confirm→apply→reverse, calling `celnet_corpactions::apply_event` /
  `position_delta` directly (`lifecycle.rs:193-231` apply, `:270-282` reverse-by-inversion).

### 1.3 Server and GUI — the monitor service is live

- **RPCs.** `CorporateActionsService` (`crates/celnet-server/src/services/corpactions/mod.rs`,
  517 lines): `ListInstrumentSchedule`, `ListCorporateActions`, `ConfirmCorporateAction`,
  `ApplyCorporateAction`. Proto at `crates/celnet-proto/proto/celnet.proto:8995-9193`.
- **Apply books a real position leg** through `RatesPositionStore::book`
  (`services/corpactions/mod.rs:326-370`).
- **Authorization.** Confirm/apply gated on `Action::Refdata × AssetClass::FixedIncome`
  (`services/corpactions/mod.rs:37-44`); `Action` enum at
  `crates/celnet-entitlements/src/capability.rs:47…`.
- **GUI.** `gui/src/workspaces/CorporateActionsWorkspace.tsx` (380 lines) +
  `gui/src/hooks/useCorporateActions.ts`, registered as a top-level nav item
  (`gui/src/lib/commands.ts:298`), typed contract at `gui/src/data/contract.ts:5638-5827`.

### 1.4 Bond analytics — `celnet-bond` / `celnet-rates`

- **Present:** `yield_to_maturity` (`crates/celnet-bond/src/yield_solve.rs:44`; also
  `crates/celnet-rates/src/bond.rs:184`), `z_spread` (`rates/src/bond.rs:195`), `g_spread`
  (`:216`), `asset_swap_spread` (`:229`), `accrued_interest`
  (`crates/celnet-bond/src/price.rs:22-24` → `CashflowSchedule::accrued`,
  `celnet-bond/src/schedule.rs:136`), and the `bond_risk` family (DV01/duration/convexity).
- **Absent:** yield-to-worst, yield-to-call, any call/put-schedule-aware yield — zero matches
  across both crates. Also absent: any `pool_factor` / current-face vs original-face concept in
  either pricing crate (zero matches), and any ex-date/ex-coupon handling in the accrued path.

### 1.5 Trading halt — `PricingControl`

A **firm-wide, two-boolean** kill switch exists and is genuinely hot-path-safe:
`crates/celnet-server/src/services/pricing_control.rs:1-45` — `outbound_enabled` /
`inbound_enabled` read through `Relaxed` atomics at each enforcement seam, versioned and fanned
out over a `tokio::sync::watch`. Enforced in FIX outbound (`services/fix.rs:279-285, 786, 1007,
1284, 1756`) and LP ingest (`services/aggregation.rs:480, 628-654, 1830`); persisted
(`config/identity.rs:1020-1022`); driven by `SetPricingControl` and mirrored to every client
(`ws/mod.rs:153-155, 413-415`). **It has no instrument, session or desk key.** No
per-instrument halt type exists anywhere in the repo.

---

## 2. The genuine gap — seven items, nothing more

| # | Gap | Spec origin | Evidence it is missing |
|---|---|---|---|
| **G1** | **An applied CA never reaches the live pricer.** `gov_bond_to_instrument_def` (`crates/celnet-server/src/config/reference_data.rs:859-895`) builds `BondDef.redemption`/`coupon_rate` from the **static** `celnet_refdata::GovBondSpec`, never from `InstrumentMaster.schedule`. `celnet-refstore` is referenced in only three server files (`lib.rs:583-599`, `ws/limits.rs`, the corpactions service) — never by the pricer. | Spec stage 3→4 | The whole "dynamic" half of the spec |
| **G2** | **No valuation-anchor switch (YTM→YTW/YTC), and no static call/put schedule to switch onto.** Calls exist only as discrete `Caev::Mcal`/`Pcal`/`Bput` *events*; `InstrumentTerms` (`master.rs:47-71`) has no `call_schedule`/`put_schedule` field. | Family I | Zero YTW/YTC matches; no schedule to solve against |
| **G3** | **Pool factor does not reach pricing.** It lives in `BondSchedule` and is scaled correctly, but `celnet-bond`/`celnet-rates` have no pool-factor concept, so clean/dirty price, accrued and DV01 are computed on **original** face. | Family II | Zero matches in both pricing crates |
| **G4** | **No consent-fee / covenant-amendment event shape.** `Caev` stops at 10 variants; there is no cash-only, no-schedule-change, no-redemption `PositionEffect`. `CaTerms` has no standalone-fee field independent of `redeemed_fraction`. | Family III | `event.rs:18-49`, `event.rs:184-198` |
| **G5** | **Exchange target is never valued or mastered.** `PositionEffect::Exchange` + `ExchangeLeg` exist (`effect.rs:45-53`), but `apply_exchange` (`effect.rs:228-242`) only validates the target id is non-empty and the ratio positive. It does not price the target, does not create an `InstrumentMaster` for it, and nothing downstream does either. The holder ends up owning units of a bare string. | Family IV | `effect.rs:228-242` |
| **G6** | **No per-instrument quote lock.** Only the firm-wide two-boolean switch (§1.5). | Guardrail 1 | No per-instrument halt type in repo |
| **G7** | **Accrued interest is not ex-date aware.** `CaDates.ex` is modelled (`event.rs:172-173`) but never read by `celnet-bond`'s accrued path. | Guardrail 2 | `price.rs:22-24`, `schedule.rs:136` |

### 2.1 A documentation defect found en route (fix regardless of scope)

`crates/celnet-server/src/services/corpactions/mod.rs:18-27` claims a bond priced through the
reference-data seam "re-derives clean/dirty price, accrued, YTM, DV01, duration and convexity off
the post-event schedule automatically." **That is not true today** (G1). Under guardrail 2 (no
placeholders, no faked depth) and guardrail 10 (docs stay in sync), this comment must be
corrected to state the seam is not yet wired, independently of whether G1 is scheduled.

---

## 3. Guardrail conflicts in the source spec — resolved, not copied

| Source-spec element | Celnet guardrail | Resolution |
|---|---|---|
| Names **Bloomberg CACS / Refinitiv Event Streams** as *the* feed provider | **7** (no commercial runtime deps), **8** (no vendor names in identifiers) | Already resolved by the as-built design: the ingest port is the purpose-named `CorpActionSource` (`refstore/src/source.rs:40-66`), with the open, deterministic `GovvieSource` as the **first-class** implementation. A licensed vendor feed is a customer-wired adapter behind the same trait, shipped as an interface with no bundled data. Vendor names appear only in integration prose — never in a crate, module, type or fn. |
| Keys the quote lock on **"the unique ISIN or CUSIP"** | **7** | CUSIP *data* is commercially licensed (CGS). Per the sourcing doc §4.2, Celnet keys on its internal `instrument_id` with FIGI+LEI+natively-present ISIN as external ids; CUSIP is carried opportunistically only, never bulk-sourced. The lock keys on `instrument_id`. |
| Component names — "Security Master Reference Database", "Auto-Quoter engine", "Component Valuation Module" | **8** | Another shop's component names. Celnet equivalents are `celnet-refstore` / the FIX+aggregation outbound path / (proposed) a component-valuation module inside `celnet-corpactions`. Adopt the *requirement*, never the name. |
| Stage 3→4 pushes "real-time notification alerts… without human lag", implying a synchronous chain into the pricing engine | **11** (pinned zero-alloc hot core stays log/lock/alloc-free) | CA ingest, mastering and apply must remain on the async edge. The pricer learns of a change by reading a **resolved, versioned** instrument definition (§4.1), never by having ingest call into it. This matches the existing discipline (`AggregationHub::ingest`, and the auto-hedge engine's off-core observation of a risk-version bump). |
| "Reset the active accrued-interest baseline **to zero**" on ex-date | **5** (validate against references, never assert plausible) | **This is not market-correct in general.** In an ex-dividend period several conventions (gilts being the canonical case) trade with **negative** accrued, not zero. Adopting the spec verbatim would introduce a pricing error. The requirement is "accrued must be ex-date aware"; the *formula* must be derived per day-count/ex-div convention and validated against QuantLib (G7 acceptance, §5). |
| Distribution to multi-dealer venues (in the sibling skew spec) | **7** | Same posture: the existing `celnet-fix` stack + `celnet-lp-sim` are the first-class path; commercial venue connectivity is a customer-wired adapter. |

**Checked and NOT found in the source spec:** no versioned/negotiated API is proposed
(guardrail 9 is not engaged), and no mock/placeholder is mandated (guardrail 2 is not engaged).
The spec's "disabling execution API endpoints" is an authorization/state question, not a
versioning one — treated in §4.3.

---

## 4. The Celnet design — compose with the spine, don't parallel it

### 4.1 G1 — one resolution point, no parallel store (the headline)

The defect is a **missing read**, not a missing store. `gov_bond_to_instrument_def`
(`config/reference_data.rs:859-895`) is the single place a bond becomes a priced
`InstrumentDef`. Make that one function resolve through `GoldenSourceStore` for the valuation
date, falling back to the curated static `GovBondSpec` when the store holds no effective record.
The static universe stays exactly what it is today — the OSS seed that bootstraps the golden
store (the posture the sourcing doc §7.3 already set out).

Consequences that fall out for free once that read exists: a confirmed call collapses the
schedule and the next price/DV01 re-derives off it; a partial redemption's scaled schedule and
pool factor flow into the same derivation (with G3); the key-rate ladder re-derives off the
post-event schedule. **No new store, no new pricing stack, no change to the pinned core** — the
pricer reads a resolved definition exactly as it does now.

Invalidation: the golden store is versioned and bitemporal (`master.rs:78-95`), so the resolution
point can carry a cheap version check. An apply bumps the version; the next resolve picks it up.
This mirrors the risk-version-bump pattern auto-hedging already observes off-core.

### 4.2 G2/G3 — extend the terms, then the analytics

- **Static schedules on the golden record.** Add `call_schedule` / `put_schedule` (date, price)
  and amortisation/sink terms to `InstrumentTerms` (`master.rs:47-71`). The sourcing doc §6.1
  already specified these as the NEW field set; this is that specification being honoured, not a
  new one. They are *static terms* — distinct from, and complementary to, the discrete `Caev`
  events that *exercise* them.
- **YTW/YTC in `celnet-bond`.** With a call/put schedule present, yield-to-call is YTM solved to
  each candidate redemption (date, price) pair, and yield-to-worst is the minimum across
  {maturity} ∪ {calls} (conventionally the minimum for the holder). Placement: alongside
  `yield_to_maturity` in `crates/celnet-bond/src/yield_solve.rs`, reusing the existing solver.
- **Pool factor in the cashflow expansion.** `CashflowSchedule::from_bond`
  (`celnet-bond/src/schedule.rs:55-126`) is the single expansion point. Carrying a current
  pool factor there yields exactly the spec's relation — coupon = original face × pool factor ×
  (rate / frequency) — which is also already the semantics of `BondSchedule::scaled`
  (`corpactions/src/schedule.rs:239-252`). The two must agree; that agreement is the acceptance
  test, not an assumption.

### 4.3 G6 — quote lock, and how it meets the capability model and the aggregated book

This is the design question with the most sharp edges, so it is stated explicitly.

- **A lock is state, not a permission.** It belongs beside `PricingControl`
  (`services/pricing_control.rs`) as a per-`instrument_id` set, read at the *same* enforcement
  seams that already honour the firm-wide switch (`services/fix.rs` outbound,
  `services/aggregation.rs` ingest). Reusing those seams is what keeps it hot-path-safe: the
  existing gates are `Relaxed` atomic loads; a per-instrument set must preserve that property
  (a lock-free read-mostly structure, not a mutex) or it violates guardrail 11.
- **It must suppress the composite, not merely the outbound quote.** This is the trap. If a lock
  only gates the outbound FIX quote, the aggregated book still consolidates and publishes a
  composite price for the locked instrument, and every GUI and the RFQ resolver keep showing and
  using it. The lock must therefore apply at **both** the composite-publish path and the
  outbound/execution paths — i.e. the instrument leaves the tradable set, it does not merely stop
  being quoted on one channel.
- **The capability model gates who CLEARS it, not who trips it.** Tripping is automatic (an
  unverified critical CA). Clearing is a human act with the same authority as confirming the CA
  that caused it — so it maps naturally onto the existing `Action::Refdata × FixedIncome`
  (already used at `services/corpactions/mod.rs:37-44`). **Recommendation: reuse `Refdata`; do
  not mint a new `Action`.** The capability model is at 17 actions and a new one must earn its
  place. Flag for ADR (§5) if review disagrees.
- **Honest scope note:** the spec says an unverified critical action places the security in the
  lock. "Critical" needs a definition, not a vibe — the defensible rule is: any event whose
  effect shape is `Realise`, `Scale` or `Exchange` (i.e. anything that changes principal or
  identity) locks on announcement and clears on confirm; `Income` does not lock. That is
  derivable from the existing `PositionEffect` enum with no new taxonomy.

### 4.4 G4/G5 — the two genuinely new shapes

- **G4 (consent fee)** is small: a cash-only effect that changes neither schedule nor principal.
  It needs a new `Caev` variant and a new `PositionEffect` arm, plus a fee field on `CaTerms`.
  **Research flag:** the correct ISO 20022 CAEV code must be read off the published code set
  before the variant is named — the sourcing doc §1.3 already mandates loading the current
  External Code Sets release rather than hard-coding. Do not guess the code.
- **G5 (component valuation)** is the largest and least certain item. Valuing a debt-to-equity
  package requires each component (equity shares, cash, junior bonds, warrants) to be a
  **mastered instrument with a price source**. Celnet has option leaves for equity
  (`celnet-equity-vanilla`) but **no cash-equity instrument type and no equity price source** —
  unverified whether any exists; nothing was found. The honest position is that G5 is not a
  corporate-actions problem but an instrument-coverage problem wearing a CA hat, and it should be
  sequenced last or explicitly declared out of scope (§6).

---

## 5. Phased future work

Each phase is independently shippable and gate-able (`just t1` per crate; `just t2` at land).
Phases are ordered by value-per-unit-risk, not by the source spec's ordering.

**CA-P1 — Wire the applied CA into the live pricer (closes G1).**
Resolve `gov_bond_to_instrument_def` through `GoldenSourceStore` with static fallback; correct the
overclaiming doc comment (§2.1).
*Acceptance:* a bond with a confirmed+applied full call prices off the collapsed schedule — clean
price, accrued, YTM and DV01 all change, and equal an independent computation on the post-event
schedule (hand truth table + QuantLib oracle, not self-comparison). An instrument with no golden
record prices bit-identically to today (no regression on the static path).

**CA-P2 — Pool factor through to pricing (closes G3).**
Carry current face into `CashflowSchedule::from_bond`.
*Acceptance:* for pool factor `f`, every coupon equals original face × `f` × (rate/frequency);
price and DV01 scale as expected; the value agrees with `BondSchedule::scaled`'s independently
derived schedule to `to_bits` or a stated tolerance. Pool factor 1.0 is bit-identical to today.

**CA-P3 — Call/put schedules as static terms + YTW/YTC (closes G2).**
Extend `InstrumentTerms`; add the yield measures to `celnet-bond`.
*Acceptance:* YTC on each call date/price and YTW = min across candidates, validated against
QuantLib callable-bond yields (guardrail 5) and a hand truth table covering a premium bond
(worst = first call) and a discount bond (worst = maturity). A bullet with no call schedule
returns YTW ≡ YTM exactly.
*ADR:* whether the valuation anchor auto-switches to YTW for quoting/risk, or YTW is reported
alongside YTM and the anchor stays explicit. This changes what a trader sees on a live quote and
must not be decided silently.

**CA-P4 — Per-instrument quote lock (closes G6).**
Per-`instrument_id` lock beside `PricingControl`, enforced at composite-publish **and** outbound
paths; auto-trip on announcement of a principal/identity-changing event; clear gated on
`Action::Refdata`.
*Acceptance:* a locked instrument disappears from the composite, from ESP streams, and from RFQ
resolution, and execution against it is refused — verified live, not unit-only. The hot-path read
is shown to be alloc/lock-free (guardrail 11). Unlocking restores from live with no stale replay
(the property `PricingControl` already documents).
*ADR:* reuse `Action::Refdata` vs a new capability (§4.3).

**CA-P5 — Ex-date-aware accrued (closes G7).**
*Acceptance:* accrued across an ex-date boundary matches QuantLib per day-count/ex-div convention,
**including a negative-accrued case** (§3). Explicitly does not adopt the source spec's
reset-to-zero rule.

**CA-P6 — Consent-fee event (closes G4).**
*Acceptance:* the fee lands in the cash-flow timeline at the payment date, principal and schedule
are unchanged, and notional conservation still holds under the existing property test.
*Research flag:* confirm the CAEV code before naming the variant.

**CA-P7 — Component valuation for exchange/conversion (closes G5).** Sequence last; see §6.

---

## 6. Honest boundaries — what this design deliberately does not do

- **It does not source corporate-action data.** The sourcing doc §3 already states the verdict
  plainly and it has not changed: government/rates CA data is deterministic and derivable from
  open issuance data (and *is* derived, by `GovvieSource`); comprehensive corporate CA data is a
  commercially-scrubbed product with no free equivalent. Nothing in this doc closes that gap, and
  no phase here should be read as implying corporate CA coverage. What Celnet ships is the port
  and the correctness behind it — the content stays customer-wired.
- **G5 is not really solvable inside corporate actions.** A debt-to-equity swap's target needs a
  cash-equity instrument type and an equity price source. Neither was found (unverified whether
  any partial support exists elsewhere). Building "component valuation" without them would
  produce a module that returns a number nobody can source inputs for — i.e. a placeholder
  (guardrail 2). Recommend declaring it out of scope until instrument coverage lands.
- **The quote lock's hot-path cost is unproven.** §4.3 asserts a per-instrument lock can stay
  lock-free at the enforcement seams. That is a design intent, not a measured result; CA-P4's
  acceptance must include the measurement, and if it cannot be met the lock moves off the pinned
  path and the design changes.
- **"Critical action" is a judgement call encoded as a rule.** §4.3 proposes deriving it from the
  `PositionEffect` shape. That is defensible and mechanical, but it is a policy choice a desk may
  want to override. No override surface is designed here.
- **Nothing here addresses the corporate-action GUI's pricing blindness.** The workspace shows CA
  lifecycle state and schedule effect; it displays no pricing impact (zero matches for
  ytm/dv01/price/duration/convexity in `CorporateActionsWorkspace.tsx`). Once CA-P1 lands there is
  a real "before/after price" story to tell a trader, but designing that surface is not in scope
  here.
- **Reversal correctness under the new pricing wire is untested territory.** `lifecycle.rs:270-282`
  reverses by inversion and that is property-tested at the schedule level, but once G1 wires
  schedules into live pricing, a reversal must also correctly restore the *priced* state. CA-P1's
  acceptance should include a reverse round-trip, and this doc flags it rather than assuming it.

---

## 7. Amendments this doc makes to the sourcing requirements

[`BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md`](BOND-DATA-AND-CORPORATE-ACTIONS-SOURCING-REQUIREMENTS.md)
is dated 2026-07-30 and states "No code yet". That is now false — its **P1 shipped in full**
(`celnet-corpactions` + `celnet-refstore`) and **P3 shipped partially** (the lifecycle state
machine and the position-effect apply exist; the MT 564/seev message parsers do not). Its **P4
(pricing/hedging wiring) did not ship** and is exactly gap G1 here. Its status header and §10
phase list have been amended in place to record as-built state and to point here; its §11 open
item "crate boundary: evolve `celnet-refdata` vs a new `celnet-refstore`" is **resolved as
built** — both exist, `celnet-refdata` stayed the static OSS seed and `celnet-refstore` became
the mastered store.
