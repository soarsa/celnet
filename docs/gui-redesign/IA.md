# Celnet Experience — Unified Information Architecture (the spec)

Synthesis of `DISCOVERY.md` (code truth) + `VISION.md` (north-star) + the two research passes
(`RESEARCH-sota-ia.md`, `RESEARCH-synoption-spectraxe.md`). This is the settled architecture every
flow mockup implements. It resolves `VISION.md`'s open questions.

## 0. One-line thesis
**One cockpit, one contract, every asset class a peer** — a context-linked, command-driven workspace
where price · structure · axes/RFQ · surface · risk · scenario · XVA · blotter are *panes of one
surface*, each a single asset-class-parametric component. This out-functions Synoption (RFQ-only,
6 fragmented modules), out-executes SpectrAxe (firm CLOB but vanilla/FX-only, no analytics), and
subsumes OptAxe's axe distribution — because none of them do exotics + all assets + XVA in one place.

## 1. The IA spine (navigation grammar)
Replaces 3 domain tabs × 15 persistent workspaces with **one entity spine + task verbs joined by a
context bus** (FDC3-style linking — pick an entity once, every pane re-scopes):

```
 UNDERLIER (universal picker: FX·Crypto·Equity·Cmdty·Rates as peers, axe markers)
    → TICKET (registry-driven, class-adaptive; one instrument, swappable Table⇄Payoff⇄Ladder views)
       → STRUCTURE (leg ladder + template gallery + live net-greek strip + payoff)
    → PRICE (two-way, class-appropriate greeks, explainable skew overlay)
    → AXES · RFQ (cross-asset Axe Board; 3 protocols on one order)
    → RISK · SCENARIO (unified spot×vol grid + ladders, cross-asset factor sets)
    → XVA (portfolio CVA/DVA/FVA + pre-trade incremental what-if)
    → BLOTTER (one virtualized cross-asset book; row → any verb above)
```
Navigation primitives: **(a) command palette** (primary — everything reachable by mnemonic/search),
**(b) the spine breadcrumb** (drill/undrill), **(c) composable dockable panels saved as named
"perspectives"** (replaces hard-mounted workspaces). No silo is ever re-implemented per asset class.

## 2. Parametric component taxonomy (kills the replication)
Every surface is ONE component taking `(underlier, assetClass, instrument)` — generalizing the one
already-clean pattern (`GreeksStrip`, label-only class fork). The four duplicated blotters, the twin
FX/FI risk grids, and the parallel rates stack collapse to one each:

| Pane | One component, parameterized by class |
|---|---|
| UnderlierPicker | peer list; class = filter facet, not a separate app |
| Ticket + StructureBuilder | `PRODUCT_REGISTRY` drives InputBlock + applicable structures per class |
| Greeks | class-appropriate two-rho pair via `RateSensitivities` |
| AxeBoard / DealerPanel | one board across classes; firmness + skew + LP rank |
| VolSurface | 5 smile models; FX/metal/crypto/equity share it (adds crypto strike-axis) |
| Risk/Scenario | one shock grid; factor-set is a prop (FX rd/rf · rates KR · crypto funding) |
| XVA | one CVA/DVA/FVA dashboard over the netted book |
| Blotter | one virtualized, column-configurable grid; class is a column/filter |

## 3. The Axe + execution model (out-SpectrAxe/OptAxe/Synoption)
The differentiator, made concrete:
- **Cross-asset Axe Board.** Contributors post `{instrument, side, size, skew, firmness}` honoring the
  ICMA firm-vs-indicative distinction. **Inventory-aware skew is computed and rendered visually**
  (favoured side prices inside mid; strength as bar length, direction as color) — pattern-recognition
  first, not table cells. One board spans FX, crypto, metals, equity, rates.
- **Three interchangeable protocols on the SAME order**, chosen per size/sensitivity:
  1. **Firm CLOB · no last-look** (beats SpectrAxe on its own turf, but cross-asset + exotic),
  2. **Indicative axe** (OptAxe-style distribution; auto-RFQ the axed side first — measurably better
     price + hit-rate),
  3. **Analytical RFQ** (Synoption/Digital-Vega style, with full analytics attached).
- **Trust surface:** explainable pricing overlay (why the price skews — inventory/axe/funding);
  LP ranking by live + historic axe reliability + TCA; leakage controls (channel/audience per axe)
  and a credit/anonymity toggle are first-class.

## 4. Cross-asset OPPORTUNITY surfaces (only possible because of §2)
- One netted book → cross-asset portfolio **risk / scenario / XVA** in one grid.
- Cross-asset **axes/RFQ** → relative-value + switch trades expressed natively.
- One underlier→ticket→hedge→re-RFQ loop from any blotter row.
- Unified vol across FX/metal/crypto/equity (surfaces the priced-but-hidden crypto strike-axis).

## 5. The 12 IA/UX principles (from RESEARCH-sota-ia, adopted)
1. One asset-class-parametric IA, zero silos.
2. Entity-centric spine × task-centric verbs, joined by a context bus.
3. Command palette as primary navigation.
4. Composable dockable link-groupable panels + named saved perspectives.
5. One ticket, swappable representations (Table/Curve/Ladder), class-aware.
6. Registry-driven structure builder: leg-ladder + template gallery + net-greek strip + payoff.
7. Protocol-as-parameter RFQ with a ranked competitive panel (the 3-protocol model, §3).
8. Interactive risk-scenario surface driven by scenario slices (spot%/σ/vol-step/time).
9. XVA as a first-class dashboard with pre-deal incremental what-if.
10. Convention-native (ATM/RR/BF) coordinated vol surface that live re-prices.
11. One virtualized, column-configurable, attention-first blotter.
12. Density with discipline: dark-first; redundant color+glyph; restrained decaying flash;
    translucent chrome, **opaque data planes**; WCAG AA 4.5:1 (no color-only P&L, ≤3 flashes/s).

## 6. Anti-patterns to avoid (evidenced)
Per-silo component duplication (our 4 blotters/15 workspaces) · model-mirrored form-per-table UI ·
task-siloing without shared entity context · unbounded modules without link-group discipline ·
sustained blink / color-only gain-loss · translucency behind live numbers.

## 7. Mockup deliverable set (covers ALL flows)
`00-shell` (price + axe teaser, DONE/verified) · `01-structure` (multi-leg/exotic builder) ·
`02-axeboard-rfq` (full cross-asset Axe Board + 3 protocols) · `03-surface` (5-smile vol surface,
ATM/RR/BF, arb) · `04-risk-scenario` (cross-asset scenario grid + ladders) · `05-xva` (portfolio
CVA/DVA/FVA + pre-trade what-if) · `06-blotter` (one virtualized cross-asset book) · `07-stream`
(two-way streaming + click-to-trade to a counterparty) · `08-fixed-income` (curve + quoting + deal
booking, folded into the spine) · `09-admin` (users/roles/capabilities/FIX) · `10-command-palette`
(the navigation overlay). Excel parity is documented, not separately mocked (same contract).

## 8. Resolved open questions (from VISION §Open)
- **Axe-board interaction:** the 3-protocol model in §3 (firm CLOB / indicative axe / analytical RFQ
  on one order) — settled.
- **Best unified-IA precedent:** command-driven spine + context-linked dockable panels + saved
  perspectives (Bloomberg Launchpad/MARS + FDC3), NOT free-tiling — settled.
- **Cross-asset structures (multi-class legs):** **v1 mocks show single-class structures**; the Axe
  Board and blotter/risk/XVA are cross-asset from v1; multi-class *structures* (e.g. an FX-vs-crypto
  RV package) are flagged as a fast-follow evolution, not v1 — settled to keep v1 honest to the
  current `price_cross_asset` seam.

## 9. API evolution the IA requires (clean, unversioned)
XVA wire surface (proto msg + `handle_unary` arm; synergy w/ **D-xva** lane) · axe-distribution
messages (contributor axe + client board) · cross-asset exotic coverage past 3/24 where the carry
seam allows (else honest capability gates) · crypto strike-axis surface field · rates/cross-asset
streaming. Each lands as a lodestar `decision`/`spec:satisfies` claim (§ no-drift).
