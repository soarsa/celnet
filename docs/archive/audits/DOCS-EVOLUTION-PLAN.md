# Celnet — Docs-Evolution Plan (Honesty-Reframe + Genuine In-Repo Gaps)

> Architect synthesis of three read-only lenses (Market-SOTA scope, In-repo completeness/SOTA,
> Honesty-reference reframe). Two deliverables: **(A)** a mechanical honesty-language reframe across
> the published capabilities corpus that PRESERVES every validation-locus fact (no fabrication, no
> converting deploy-gated items into in-repo claims); **(B)** the short, genuine, in-repo-buildable
> SOTA/scope gap list with build scope + oracle + priority. Run the reframe **after** the concurrent
> doc-QA workflow lands (it is editing the same files); build the gaps independently.

## 0. Verdict

Celnet's FX-options analytics catalogue is at or above the 2026 market-SOTA bar (Murex MX.3 / Numerix
CrossAsset / Fenics kACE / Bloomberg OVML-MARS / SynOption) and the prior audit's two flagged analytics
gaps (American/Bermudan; correlated basket/best-of/worst-of) are **both built, on the wire, and
parity-gated**. The platform is **materially complete**. Verified directly at the tree:

- No `celnet-curve` crate and no `DiscountCurve/ZeroCurve/YieldCurve/RateCurve/bootstrap` in any `src` —
  pricing is off single flat continuous rates per currency (`r_dom`/`r_for`, `DF=exp(-rT)`).
- No `wide` dependency anywhere in the workspace.
- No `american_gk.csv` / no `baw`/`whaley` golden, and no `crates/celnet-parity/tests/american.rs`.
- `heston_fo.csv` is HAND-PINNED from published Fang-Oosterlee literature, not a QuantLib grid.

**The work is overwhelmingly the reframe.** The genuine in-repo gap list is short (one material SOTA
capability + a small rigor/oracle tail). Everything else is either already-complete-SOTA or correctly
**environment-bound** (CUDA absolute throughput, cross-host wire p99, live JVM CelNet estate, Raft §6
cross-DC, live vendor/CSA VALUES) — which must stay neutrally reframed as deploy-validated, **never**
claimed in-repo and **never** fabricated.

---

# PART A — THE HONESTY-REFRAME

## A.1 Principle (the hard rule)

Remove the self-referential meta-language ("honest", "honestly", "honest boundary", "overclaim",
"never claim in-repo", "no marketing fiction") and replace it with neutral, confident
**Deployment & validation** statements. **The validation locus of every fact is preserved exactly.**
A deploy-validated bullet stays a deploy-validated bullet — only the editorial wrapper is deleted. Never
rewrite an environment-bound item into an in-repo claim; never invent in-repo proof that does not exist.

## A.2 New section titles

- Primary heading (replaces every `Honest boundary` / `## Honest boundary` / `### 14.8 The honest boundary`):
  **`Deployment & validation scope`**.
- Per-product-class validation tables (replaces "Validation honesty (per product class)"):
  **`Where each capability is validated`**.
- Figure-HTML visible footnote/banner label `Honest boundary[:/—/.]` → **`Deployment & validation scope`**;
  HTML comment `<!-- HONEST BOUNDARY banner -->` → `<!-- DEPLOYMENT & VALIDATION SCOPE banner -->`.

## A.3 Wording templates

**Lead-in template** (replaces every "we never claim in-repo / none of these is claimed in-repo" lead):

> Celnet's capabilities are validated where each one can be validated: in-repo by benches, parity rows,
> and frozen golden tables; on target hardware or the live estate at deployment for the absolutes that
> require it. The following are measured at deployment — in-repo establishes their correctness seams,
> loopback/host-local proofs, and ratios:

**Per-deploy-bound-item bullet template:**

> `<Capability>` — validated on `<target hardware / live estate>` at deployment; in-repo establishes
> `<correctness seam / loopback proof / host-local ratio>`.

**Adjective/phrase rules (apply globally, in prose, figure HTML, and diagram-meta strings):**

| Find | Replace |
|------|---------|
| `honest <noun>` | `<noun>` |
| `honest price_std_error` / `honest MC std-error` | `reported (Monte-Carlo) price standard error` |
| `<verb> honestly` | `<verb>` (or `<verb> faithfully` where a quality word is wanted) |
| `honest exception` | `documented exception` |
| `honestly deferred/zeroed` | `deferred (the Greek strip is returned as zeros alongside the real premium and std-error)` |
| `an honest DRC = 0` | `a documented, cited DRC of zero (MAR22 — no issuer jump-to-default in deliverable FX)` |
| `zero overclaim` | `one contract` / `proven across five clients` |
| `rather than overclaiming …` / `never overclaimed` | `Celnet states this distinction …` / `stated as such in the file header` |
| `not / no marketing fiction` | (delete; state the positive) `a reviewer doing diligence finds runnable proof` |
| `Never claim f64 on Metal` | `the M4 Metal backend is f32-only` |
| `Honesty as a differentiator` / `per-class honesty IS the rigor` | `Per-class validation` / `This per-class validation discipline is the rigor` |

## A.4 The canonical block (carried verbatim in 3 anchor locations)

The same ~8-bullet block appears in **(1)** `docs/celnet-capabilities.html` (§15, `id="boundary"`,
kicker "Verbatim — carry everywhere", h2 "Honest boundary", lead "None of these is claimed as
in-repo-proven"); **(2)** `docs/CELNET-CAPABILITIES.md` (`## Honest boundary`); **(3)**
`docs/celnet-capabilities/14-engineering-rigor.md` (`### 14.8 The honest boundary`). Reframe identically
in all three: rename to **Deployment & validation scope**, drop the kicker / the
"never claimed as in-repo-proven" / "None of these is claimed in-repo" clauses, apply the lead-in
template, keep **every bullet's factual content** (cross-host wire p99; CUDA/NVIDIA absolute throughput +
≤50ms exotic + Workload-A/B absolutes; §11 SLOs; live JVM CelNet estate; Raft §6 cross-DC; Tier-1/3
plugins; XVA synthetic netting/internal-only; MC std-error), restated as plain validation-locus
statements. Example bullet:

> CUDA/NVIDIA absolute GPU throughput, the ≤50ms exotic, and Workload-A/B absolutes — validated on NVIDIA
> hardware at deploy; in-repo establishes GPU/CPU correctness (f32 == f64 == golden) and host-local
> ratios (M4/Lavapipe). The M4 Metal backend is f32-only.

## A.5 Per-doc change list (apply after concurrent QA lands; line numbers are indicative — match on text)

Honesty-meta hit counts at audit time (search `honest|overclaim|marketing fiction|never claim`):

**Markdown showcase**
- `docs/CELNET-CAPABILITIES.md` (13): `## Honest boundary` → **Deployment & validation scope** (A.4);
  figure-table caption strings mirrored from diagram-meta (A.6); inline adjectives per A.3.
- `docs/celnet-capabilities.html` (25): §15 boundary block (A.4); exec heading
  "One platform, five clients, zero overclaim" → "…one contract"; basket lines 640/665
  "honestly deferred/zeroed" → "documented deferral (zeroed)"; line ~433 "an honest DRC = 0" → A.3;
  `alt=` attributes mirrored from diagram-meta (A.6).

**Chapters** (`docs/celnet-capabilities/NN-*.md`)
- `01-executive-summary` (2): `> **Honest boundary.**` → `> **Deployment & validation scope.**`
  (bullets kept; "are never claimed in-repo" → "are validated at deployment").
- `02-capability-map` (7): `> **Validation honesty (per product class).**` → `> **Per-class validation.**`;
  table row "Honesty as a differentiator | … not marketing fiction" → "Per-class validation evidence | …
  every figure labelled by where it was measured … a reviewer finds runnable proof"; "honest exceptions,
  e.g. basket Greeks deliberately zeroed" → "documented exceptions, e.g. basket Greeks deliberately
  zeroed"; "honest DRC = 0" → A.3.
- `03-system-architecture` (2): in-text "see the honest boundary at the end of this chapter" →
  "see the deployment-and-validation-scope note …"; `> **Honest boundary.**` → A.4 heading/lead.
- `04-quant-coverage` (7): `### Honest boundary (carried verbatim …)` → `### Deployment & validation
  scope`; "Honesty as a differentiator: …" → "The validation method is stated per product class — Celnet
  states the strongest oracle each class admits"; two "honest Monte-Carlo price_std_error" → "reported";
  remaining inline per A.3.
- `05-extensibility-plugins` (2): "rather than overclaiming universal bit-identity" →
  "Celnet states this distinction: bit-identity holds for an op-order-matched twin, and a tolerance-based
  comparator is the correct gate for an arbitrary native/Wasm pair."
- `06-risk-management` (4): `## Honest boundary (carried into every claim above)` →
  `## Deployment & validation scope`; `> **Honest boundary — XVA is internal-only.**` →
  `> **XVA is internal-only.**`; "documented, cited zero (MAR22) … stated honestly rather than
  fabricated" → drop the editorial tail (citation carries credibility).
- `07-performance-latency` (5): `### The wire-path, kept honest` → `### The wire path`;
  "reported honestly" → "is reported"; fig-07 strings per A.6.
- `08-scalability-scaleout` (3): boundary refs → A.4 (loopback proves compute/framing/quorum;
  cross-host p99 + §11 absolute SLOs + Raft §6 deploy-validated).
- `09-api-contract-parity` (7): `#### Honesty as a contract feature: price_std_error` →
  `#### price_std_error as a contract feature`; `> **Honest boundary.**` → A.4; "(multi-asset Greeks
  honestly deferred)" → "(multi-asset Greeks deferred)"; `**Honest exception:** correlated-basket Greeks
  are deliberately zeroed` → `**Documented exception:** …`; "carries an honest price_std_error" → A.3.
- `10-excel-integration` (3): `> **Honest exception — CELNET.BASKET Greeks.**` →
  `> **Documented exception — CELNET.BASKET Greeks.**`; "honestly 0" → "returned as zeros (documented
  deferral); the premium and its std-error are real".
- `11-trader-gui` (6): strip the adjective per A.3 ("honest price standard error" → "price standard
  error"; "honest per-row stream-health badge" → "per-row stream-health badge driven by real sequence
  and resync state"; "honest provenance, not the requested label" → "the calibrated provenance, not the
  requested label"; "reads magnitude honestly" → "reads magnitude faithfully"; two "stated/labelled
  honestly" → "stated"/"labelled 'largest of N'").
- `12-celnet-integration` (2): `### 12.5 Honest boundary` → `### 12.5 Deployment & validation scope`
  (seams + FIX 4.4 loopback in-repo; live JVM estate lifecycle deploy-validated).
- `13-competitive-positioning` (4): "the validation bar is honest per product class" → "stated per
  product class"; "with honest std-error" → "with a reported std-error".
- `14-engineering-rigor` (10): `### 14.8 The honest boundary` → A.4; "This per-class honesty IS the
  rigor" → "This per-class validation discipline is the rigor"; "labelled honestly when no exact oracle
  exists" → "labelled by oracle class when no exact oracle exists"; "so the provenance is never
  overclaimed" → "so the provenance is explicit".

**Figure HTML sources** (`docs/assets/celnet-capabilities/_src/fig-*.html`) — render INTO the PNGs:
- fig-01 (2), fig-02 (1), fig-03 (2), fig-06 (1), fig-07 (4), fig-08 (1), fig-09 (1), fig-10 (1),
  fig-11 (1), fig-12 (1), fig-13 (1). In each: visible label `Honest boundary[:/—/.]` →
  `Deployment & validation scope` (keep body text — it already states the deploy locus); HTML comment
  banner renamed; in-body adjectives ("honest price std-error", "honest Monte-Carlo standard error",
  "honest … server telemetry", "record the slow end honestly", "Validated per class, honestly") per A.3.
  fig-04/fig-05 are already clean (0 hits).
- **DOWNSTREAM BUILD STEP (owned by the doc-editing workflow):** re-render `assets/celnet-capabilities/
  fig-*.png` from the edited HTML, or the images still show "Honest boundary".

**Figure metadata + mirrors** (`docs/assets/celnet-capabilities/_src/diagram-meta.json`, 15 hits —
SOURCE OF TRUTH for captions/alt-text; edit first then re-sync the two mirrors: the
`CELNET-CAPABILITIES.md` figure table and the `celnet-capabilities.html` `alt=` attributes):
- "with an explicit honest boundary on deploy-gated absolutes" → "with an explicit deployment-and-
  validation-scope note on deploy-validated absolutes"
- "a verbatim honest boundary" → "an explicit deployment-and-validation-scope note"
- "a muted/dashed/coral honest-boundary footnote" → "a muted/dashed/coral deployment-and-validation-scope
  footnote"
- "honest Monte-Carlo standard error" → "reported Monte-Carlo standard error"
  (alt/caption text needs no PNG re-render; only the in-image banner does, per the figure-HTML item).

## A.6 Also scan (non-showcase docs with honesty-meta)

`COMPETITIVE-ANALYSIS.md`, `SCALE-OUT.md`, `GPU-AT-SCALE-PLAN.md`, `SOTA-2026.md`, `HARDENING.md`,
`POST-COMPLETION-AUDIT.md`, `INTERFACES.md`, `ANALYTICS-SPEC.md`, `API-CLIENTS.md`,
`CAPABILITIES-VS-COMPETITION.md`, `CLIENT-PARITY-MATRIX.md`. Apply the same A.3 rules. The existing
`COMPETITIVE-ANALYSIS.md` "Honest boundary" deploy-gated facts are correctly worded — keep the FACTS,
rename the heading to "Deployment & validation scope" and drop the meta wrapper. (GUIDE.md ledger and
`~/.agents` memory are operational logs, not published corpus — leave as-is.)

---

# PART B — GENUINE IN-REPO SCOPE / SOTA GAPS

Only items that are real, SOTA, in-repo-buildable (no hardware/estate dependency). The list is short by
design — the platform is materially complete.

## B.1 (P1) Multi-curve / OIS-CSA discounting term-structure

The single genuine market-SOTA capability missing from the pricing core. 2026 institutional FX-options
valuation is multi-curve: collateralized (CSA) trades discount on the OIS/CSA curve, the FX forward is
built from an FX/cross-currency-basis curve, XVA needs distinct funding curves. Celnet prices off a
single flat continuous rate per currency (`VanillaInputs.r_dom/r_for`, `DF=exp(-rT)`); there is NO
bootstrapped curve object anywhere in `src` (verified). `ANALYTICS-SPEC.md §0` already designed the SEAM
("DFs stored separately so … dual-curve discounting plug in cleanly") — pure deterministic in-repo
numerics, not deploy-bound.

- **Scope:** new crate `celnet-curve` (deps {core, types}). A bootstrapped piecewise term-structure of
  discount factors — log-linear-in-DF (≡ piecewise-flat instantaneous forward) interpolation over pillar
  instruments; `DiscountCurve {pillars, log_dfs}` exposing `df(t)`/`zero_rate(t)`; an OIS/CSA discount
  curve + an FX-forward curve (read F from the curve, not `S·exp((r_d−r_f)T)`); cross-currency basis as
  the spread between FX-implied and OIS curves. `VanillaInputs` accepts EITHER flat rates (current; exact
  byte-for-byte special case = a 1-pillar curve) OR a curve handle; preserve the single unversioned
  contract via an **additive** proto curve message (no `schema_version`, no renumber).
- **Oracle/gate (`crates/celnet-parity/tests/curve.rs`):** (1) a flat 1-pillar curve reproduces the
  existing flat-rate GK price to `f64::to_bits` identity (zero-regression); (2) bootstrap round-trip:
  a curve bootstrapped from synthetic deposit/FX-forward pillars reprices those pillars ~1e-12; (3) an
  independently coded log-linear-DF interpolation oracle matches `df(t)` ~1e-12; (4) CSA-vs-flat
  divergence monotone and sign-correct. `clippy -p celnet-parity --test curve -D warnings` + full
  `just check`.
- **Deploy boundary (state in docs, do NOT build):** live CSA/collateral schedules + live curve VALUES
  are deploy/integration-gated — only the curve ALGEBRA + bootstrap is in-repo, mirroring the
  multi-source-surface ALGORITHM-vs-live-VALUES line already drawn.

## B.2 (P1) Independent QuantLib golden + parity row for American/Bermudan

American is built and wired (`crates/celnet-exotics/src/american.rs`: PSOR free-boundary CN-FD +
Longstaff-Schwartz LSM; on-wire `AmericanOption`/`ExerciseStyle`; reachable from all 5 clients) but is
gated ONLY by in-crate structural invariants + the published L-S Table-1 value + an internal
`fd_matches_lsm_within_stderr` self-cross-check. PSOR-FD == LSM-MC is a check between two of OUR methods —
a shared-error risk (exactly the circular-oracle failure mode the FRTB 0.75ρ bug taught). Every other
analytic/PDE product meets the independent-QuantLib-golden bar; American does not (no `american_gk.csv`,
no `tests/american.rs` — verified).

- **Scope (M):** add frozen `crates/celnet-golden/data/american_gk.csv` from QuantLib
  `FdBlackScholesVanillaEngine` (and/or `BaroneAdesiWhaleyApproximationEngine`) over an FX dual-carry grid
  (moneyness × vol × T × r_dom/r_for, incl. the deep-ITM put and foreign-rate-driven call early-exercise
  regimes) — exactly mirroring how `heston_fo.csv` closed PC-HESTON-GOLD. Add
  `crates/celnet-parity/tests/american.rs` gating `american_price` (PSOR-FD) to the golden at ~1e-4 abs /
  1e-3 rel (early-exercise FD precision band, honestly STATED — NOT machine precision) and
  `american_price_lsm` within reported `price_std_error`.
- **Gate:** `clippy -p celnet-parity --test american -D warnings` + full `just check`.

## B.3 (P2) QuantLib AnalyticHestonEngine golden (verification depth)

`heston_fo.csv` is published Fang-Oosterlee literature (verified header); the celnet-heston CM≈COS
cross-check is two transforms over a SHARED CF (shared-error risk). Every other core payoff is gated vs
an independent QuantLib-derived grid.

- **Scope (M, disjoint — golden test + data file):** generate `crates/celnet-golden/data/heston_gk.csv`
  from QuantLib `AnalyticHestonEngine` over the ≤3y FX domain (keep `heston_fo.csv` as a second
  independent anchor); gate CM+COS in `crates/celnet-golden/tests/heston_grid.rs` to ~1e-6 abs + 1e-5 rel.
  Catches a shared CF/quadrature error the internal CM≈COS check cannot.

## B.4 (P2) `wide` portable-SIMD batch path (SOTA-2026 ADOPT #4)

No `wide` dep in the workspace (verified). The batch slice/payoff hot path is autovectorized scalar only;
`ARCHITECTURE`/`SOTA-2026` already name `wide` (MIT/Apache-2.0, stable, no nightly, no FMA contraction)
as the determinism-controlled SIMD escape hatch.

- **Scope (S, disjoint to surface/core):** profile first to confirm the hotspot still matters at current
  numbers; `cargo-deny`-vet `wide`; replace the profiled batch smile/Greek inner loop with explicit f64x
  lanes. **Gate:** bit-identical-to-scalar reconciliation (determinism) + a `celnet-bench` divan delta
  showing the speedup win.

## B.5 (P2/P3) Audit rigor tail + numerics-depth follow-ons

These widen the proven-fault surface / numerics depth to the platform's own gold standard — quality, not
missing capability. Adopt the audit/SOTA-2026 scopes verbatim (sound):

- **Pathwise/AAD MC Greeks for the EXOTIC MC stack** (SOTA-2026 #3; P2, M-L): vanilla AAD + GPU pathwise
  exist; `crates/celnet-exotics/src/mc.rs` has NO pathwise/adjoint Greeks (bump-and-revalue today). Add
  hand-written pathwise (and LR where smoothing needed) estimators for the fixed smooth kernels
  (barrier/asian/lookback/basket), gated vs the bump oracle within MC stderr; bench cost ratio ≤~5× one
  price for the full vector. Document non-smooth payoffs needing LR/Malliavin as the boundary rather than
  half-building.
- **Mutation/fuzz/coverage widening** (PC-MUT-WIDEN/#6, PC-FUZZ-DECODE/#7, PC-SURFACE-COV/#11; P2/P3):
  per-crate `.config/mutants-<crate>.toml` + CI (start with celnet-surface); no-panic/total-decode/
  bounded-alloc fuzz targets for the replog/journal/proto byte parsers seeded from existing decode
  vectors; `cargo llvm-cov --fail-under-lines/--fail-under-regions` floor for celnet-surface. (NOTE: the
  in-repo-completeness lens reports several of these as already executed; reconcile against the tree
  before scheduling — only build the genuinely-missing subset.)
- **GPU PDE (tiled ADI / cyclic-reduction tridiagonal)** (SOTA-2026 §3.4; P3, L): no ADI/tridiag kernel
  in celnet-gpu; build an f32 kernel reconciled node-by-node to the f64 CPU CN/PSOR within a derived f32
  bound (mirror `batch.wgsl`). Absolute GPU-PDE throughput stays CUDA-deploy-gated.
- **American BARRIER early exercise** (P3, M-L): deliberately deferred by `american.rs`'s own boundary;
  composes the existing PDE KO wall + PSOR projection. Only after B.2 so it inherits the same oracle
  harness. Additive `ExerciseStyle` on `SingleBarrier`, wire + 5 clients, gate vs LSM within stderr +
  structural.

## B.6 NOT gaps — keep neutrally reframed (deploy/environment-bound; never claim in-repo, never fabricate)

CUDA/NVIDIA absolute GPU throughput + ≤50ms exotic + Workload-A/B absolutes (M4 Metal lacks f64 → in-repo
proves correctness f32==f64==golden + host-local RATIOS). Cross-host wire p99 / kernel-bypass NIC / §11
absolute SLOs (no multi-host network in-sandbox → in-repo proves §1.2 in-core truth-gate + loopback
fleet_slo). Live JVM CelNet estate lifecycle (seams + FIX 4.4 loopback in-repo). Raft §6 dynamic
membership / cross-DC / real partitions (fixed-membership correctness complete). Live vendor implied-vol
VALUES + live FX fixing VALUES + live CSA/collateral/wrong-way-risk for XVA (synthetic/published in-repo;
XVA deliberately internal-only, no wire surface). Plugin Tier-1 stabby signed-.so + Tier-3 Landlock/
seccomp (designed-only by intent). These are catalogued accurately in COMPETITIVE-ANALYSIS.md and
POST-COMPLETION-AUDIT.md class C — keep the FACTS, drop the meta wrapper (Part A).

---

# PART C — EXECUTION ORDER

1. **Wait** for the concurrent doc-QA workflow to land (it edits the same files).
2. **Part A reframe** — apply A.2–A.6 mechanically; edit `diagram-meta.json` before its two mirrors;
   re-render `fig-*.png` from edited HTML (downstream owner: doc workflow). Verify zero residual hits:
   `grep -ri "honest\|overclaim\|marketing fiction\|never claim" docs/celnet-capabilities* docs/assets/celnet-capabilities/_src`.
3. **Part B gaps** — build in priority order: B.1 (curve, P1) and B.2 (American golden+parity, P1) first;
   then B.3/B.4 (P2); B.5 tail (P2/P3) reconciled against the tree. Each gated implement→adversarial-
   verify→independently-re-gated, full `just check` green ("All gates passed."), clippy the parity TEST
   target, re-derive any constant vs the published source (no circular oracle).
4. **Doc sync after each B item** — when an item lands, document it as **built** (in-repo validated),
   and draw the same deploy boundary for its live-VALUES portion (B.1 CSA/curve values). NEVER place a
   newly-built in-repo item under the "Deployment & validation scope" label.
