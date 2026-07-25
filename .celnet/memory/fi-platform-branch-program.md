---
name: fi-platform-branch-program
description: FI platform expansion (reference-data → curves → FI Excel) — branch feature/fi-reference-data MERGED to main + DELETED 2026-07-01; the never-merge rule is RETIRED
metadata: 
  node_type: memory
  type: project
  originSessionId: 3759f335-a305-4b8e-b3aa-727dea5a173c
---

**RULE RETIRED (2026-07-01):** the branch **`feature/fi-reference-data`** was **merged into
`main` and deleted** (local + `origin`) at the user's explicit instruction — overriding the
earlier "never merge" rule. Landing merge on `origin/main` = **`ae689fc`** (merged the branch's
29 FI commits + this session's FIX desk-inbox feature into the fast-moving trunk, resolving
conflicts across fix.rs/fix_registry.rs/quote.rs/lib.rs/client rates.rs — union of main's
pre-trade-limit `store` param + my desk-routing; 394 server tests + workspace clippy green).
Do NOT recreate the branch or re-apply the never-merge rule. Ongoing FI work now lands on `main`
directly (respecting CLAUDE.md rule 1: push only to `origin` = `soarsa/celnet`). The FI history
below is retained as the delivered-feature record.

--- Historical (pre-merge) instruction, kept for provenance ---
**User instruction (was HARD, now RETIRED):** keep committing the FI platform expansion onto branch
**`feature/fi-reference-data`** and push the BRANCH; NEVER merge into `main`. Push via
`eval "$(ssh-agent -s)" && ssh-add ~/.ssh/github-key`. Toolchain prefix:
`export PATH="/opt/homebrew/opt/rustup/bin:$HOME/.cargo/bin:$PATH" && export CARGO_INCREMENTAL=0`
then plain `cargo` (`just`/`nextest` absent). GUI gate = `cd gui && npm run build`; live e2e =
`CELNET_ACCESS_MODE=enforce` via the Playwright harness (`gui/e2e/demoEdge.ts`; `~/.cargo/env`
shim exists). e2e that create users need RUN-UNIQUE emails (demo edge persists its store).

**Program = 3 phases, sequential (later phases depend on earlier):**
1. **Reference-data repository — DONE** (`6ba2605` on the branch). Cross-asset `InstrumentDef`
   registry under the **Administration** domain (admin-only UI; data server-resolvable by any
   caller). Families: Deposit/Fra/StirFuture/VanillaIrs/Ois/Bond + ExternalId (ISIN/CUSIP/…).
   Persisted on `IdentityStore` (serde-default). Resolver API for next phases:
   `IdentityStore::instrument_by_id`/`instrument_by_external_id`/`instruments`, and
   `crates/celnet-server/src/config/reference_data.rs::{accrual_basis_from_label, roll_rule_from_label,
   centre_from_label, payment_frequency_from_label}`. RPCs List/Get (any caller) + Create/Update/Delete
   (admin) on AuthService. `refdata` rail item is in ADMIN_ONLY_WORKSPACES, domain=administration.
2. **Curves — Part A DONE; Part B next.** **Part A DELIVERED** (backend `645a8b1` + frontend
   `91fb7e2`, pushed on branch). Generalised the pillar to a **`PillarTenor` oneof**
   `{years | months | maturity_date}` (NOT the FX `Tenor` — that's ON/TN/SN/IMM-flavoured;
   purpose-built minimal message instead). Engine: new `usd_ois_schedule_to_maturity` /
   `usd_ois_schedule_for_months` in `celnet-rates/src/schedule.rs` (annual rolls + final stub;
   byte-identical to whole-year builder for N∈1..30 — parity oracle passes). Server `build_quotes`
   orders pillars by final ACT/365F pay-time (arm-agnostic) + new errors MissingPillarTenor/
   InvalidPillarDate. Full ripple done: rates_pricing, ws/codec (JSON `tenor:{years|months|
   maturity_date}`), client SDK (`UsdSofrCurve.pillar_months`/`.pillar_on`), fix backend,
   aggregate/federation tests. GUI: contract `OisCurvePillar.tenor: PillarTenor` + `pillarTenorLabel`/
   `yearsPillarTenor`/`pillarYears`; `ratesPricing.ts` mirror builders + `pillarMaturityYears`;
   CurveWorkspace pillar editor (arm select Years/Months/Date + native date picker + add/remove).
   **Deferred (user choice 2026-06-30):** the per-tenor key-rate ladder (federation `convert.rs` +
   mock) still whole-year-labels and returns `invalid_argument` for month/dated pillars — lossless
   days-from-spot relabel is the follow-on. Gates green: cargo check --workspace 0-warn, celnet-rates
   27, server lib 355, federation 5, GUI `npm run build` + vitest rates/curve/desk. **Part B — server side DONE (2026-06-30); client surface (inc.3) NEXT.**
   Multi-instrument curve build resolving against the reference-data registry, oracle gate
   **reprice-to-par** (1e-8 rate / 1e-10 DF) — all server-side increments landed + pushed:
   - **inc.1** (`baacaa2`): generalised `celnet_rates::bootstrap_curve(&[CalibrationInstrument])`
     over deposit/STIR-future/FRA/OIS/vanilla-IRS; new `Deposit` cash instrument (closed-form
     DF). `bootstrap_ois` is now a byte-identical wrapper. Worst reprice residual 2.6e-15.
   - **inc.2a** (`7b2f713`): `celnet-server/src/config/curve_calibration.rs` —
     `calibration_instrument(def,quote,value_date)`/`calibration_set` mapping refdata
     `InstrumentDef` → `CalibrationInstrument` (Bond → typed NotCalibratable). New builders:
     `tenor_to_date`, generic `fixed_ois_schedule`/`swap_leg`, day-count accrual. **Deferred:**
     EOM/IMM swap roll returns `UnsupportedRollConvention` (only regular roll generated).
   - **chore** (`9b202d0`): cleared the branch's pre-existing `clippy -D warnings` (slice-A
     PillarTenor proto doc-comment list → SINGLE-LINE bullets; prost+clippy reject wrapped/over-
     indented bullets; + collapsible_if + module allow(result_large_err) for tonic::Status).
     **Branch is clippy-D-warnings green now.**
   - **inc.2b** (`d1744c3`): **`rpc BuildCurve(BuildCurveRequest) -> CalibratedCurve`** on
     **AuthService** (registry-owning service — decision: keep PricingEdge store-free, NOT
     PriceRates). Resolves `{instrument_id, quote}` via `IdentityStore::instrument_by_id` →
     `calibration_set` → `bootstrap_curve` → per-instrument points `{instrument_id, time_years,
     discount_factor, zero_rate}` sorted by maturity. **Authenticated** via `session_token`
     (field 5; consistent w/ list/get_instrument). reprice-to-par e2e residual 1.2e-15.
   **inc.3 (`bcc4aab`) — client surface DONE + live-verified.** Server WS mirror frame
   `build_curve`→`calibrated_curve` (codec.rs + mod.rs dispatch → AuthService.build_curve,
   session-authed). GUI: BuildCurve contract+wsCodec+transport; **CurveWorkspace gains a "By
   instrument reference" mode** (pick registry instruments by currency + quote per pillar → call
   BuildCurve → render DF/zero curves + calibrated-points grid) alongside the unchanged slice-A
   pillar editor; MockTransport.buildCurve bootstraps offline (no stub). Excel codec mirrored.
   Gates: cargo check proto+server + clippy server-lib -D clean; gui `npm run build` clean;
   vitest gui 58 + excel 3; **live e2e under Enforce passed** (login→build from seeded registry
   instruments→calibrated curve renders, slice-A unregressed), **axe 0**. Screenshot
   `gui/test-results/curve-build-by-instrument.png`. **SDK (celnet-client) deferred:** no
   AuthService client surface exists yet (would need standing one up).
   **inc.4 (backend `4ec4f6a` + client `c99f88a`) — STANDALONE DATE-ANCHORED PILLARS DONE.**
   Operator feedback ("when building curves we should allow pillars to be defined by date"). A
   curve pillar can now be an explicit maturity DATE + simple ACT/360 rate (chosen design: a
   synthetic cash **deposit** `DF=1/(1+r·τ)` from the reference date — reuses `bootstrap_curve`;
   NOT a per-instrument tenor override). Backend: proto `DatePillar{maturity_date,quote}` +
   `BuildCurveRequest.date_pillars` (both pillar arrays now optional/default-empty) +
   `CalibratedCurvePoint.label`; `curve_calibration::date_pillar_instrument`; handler unifies
   instrument+date pillars into one labeled ladder; WS codec does date_pillars/label. Oracle:
   deposit reprices to quote 1e-8 on wire curve; server lib 368 green, clippy/workspace-check clean.
   Client: GUI+Excel contract+wsCodec; `CurveWorkspace` "By instrument reference" mode gains a
   "+ Date pillar" affordance (native date + % rate); points grid shows registry name for
   instrument pillars and `Date YYYY-MM-DD` for date pillars; `MockTransport.buildCurve` does the
   REAL closed-form deposit DF offline. Gates: gui build+vitest 901, excel typecheck+vitest 477.
   **Live Playwright e2e DEFERRED per operator (build+vitest only this increment).**
   **Part B (instrument + date pillars) COMPLETE.** Remaining FI tail: slice-A's deferred per-tenor
   key-rate ladder relabel (federation convert.rs); EOM/IMM swap roll (currently typed
   `UnsupportedRollConvention`); add a date-pillar leg to `buildCurve.e2e.ts` + `curve-date-pillar.png`
   at the next live-e2e milestone; then **phase 3 = FI Excel tab** (CELNET.* functions + FI rail tab;
   needs GetCurve + rates-streaming RPCs per `docs/FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md`).
   Source: `docs/CURVES-AND-INSTRUMENT-REFERENCE-DATA-REVIEW.md` §A/§B.
   **Pre-existing red (NOT mine, NOT curves):** `celnet-client/tests/examples_smoke.rs` 3 tests fail
   `Unauthenticated AcceptQuote` — the smoke harness runs `AccessMode::Enforce` (deliberately, per its
   comment) but the example flows never log in; permissions-lane gap, proven red at clean HEAD via stash.
3. **FI Excel tab — PENDING (last).** FI `CELNET.*` functions (RATES/DV01/KEYRATE/RATESRISK/
   RATESPOSITIONS/RATESBOOK/DEALS/RATESRFQ — most already wire-supported per the WS mirror's 9 FI
   frames) + a Fixed-Income Excel rail tab under fixed-income domain. Blocked extras need new server
   RPCs first: **`GetCurve`** (for `CELNET.CURVE`) + **rates streaming** (for `CELNET.RATESSUBSCRIBE`).
   Spec: `docs/FIXED-INCOME-EXCEL-INTEGRATION-REVIEW.md`.

**Blocked (needs user input):** bond deal-capture WIRE model needs MarketAxess's actual order +
execution-report objects (`docs/FI-BOND-DEAL-CAPTURE-GAP-ANALYSIS.md`). Reference-data covers bond
*definitions* now.

**CAVEAT — agent infra was stalling pre-compact:** two consecutive background `celnet-quant` agents
for curves stalled at the 600s stream watchdog ("compression hook mangling output"). Left NO partial
work (tree clean at `6ba2605`). After compact, retry delegation (likely transient) OR build curves
Part A inline. Session was ~$2.8k / ~190 files when this was written.
