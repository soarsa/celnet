# GUI Experience Redesign — Phase 1 Discovery (canonical artifact)

Lodestar project `github.com-soarsa-celnet` (24,573 nodes / 75,833 edges). Every claim cited to `file:line`/qualified symbol by the discovery agent (`a6ce784ac8160f1ae`). Destined for `docs/gui-redesign/DISCOVERY.md` once the lane worktree exists.

## A. Full capability surface (the ONE API)

**Asset classes / engines**: FX vanilla (`celnet-vanilla`, GK European) · FX exotics (`celnet-exotics`, 32 files: barriers single/double/window, digital, touch 1T/NT/DNT/DOT, Asian, cliquet, TARF+pivot, accumulator, lookback, American/Bermudan, quanto, var/vol swap, basket/best/worst, perpetual) · Heston+LSV (`celnet-heston`, ADI-PDE/MC) · FX linear (`celnet-linear`: forward/swap/NDF) · Equity vanilla (gen-BSM+div) · Commodity vanilla (Black-76) · Crypto vanilla (`celnet-crypto-vanilla`: linear-funding + inverse coin-margined 1/S_T) · Rates/FI (`celnet-rates`: OIS/SOFR bootstrap, FRA, IRS, STIR futures, bonds, key-rate DV01) · Perpetual/listed-future opts (arms 30/31, **LANDING**) · **XVA** (`celnet-xva`: CVA/DVA/FVA, built + parity-gated, **NO wire surface**).

Routing: `price_instrument` (`celnet-server/src/pricer.rs:495`) → non-FX `price_cross_asset` over generalized `CostOfCarry{b}`; FX/metal → `engines::dispatch_live` (+ plugin-host `ModelRegistry`, Vanilla arm only).

**Verbs**: Price · Greeks (14-field struct, class-appropriate two-rho pair via `RateSensitivities::{Fx,Carry}`) · Implied-vol/Surface (5 smile models VV/SABR/SVI/SSVI/eSSVI, `MarkSurface` calibrate+version) · Scenarios (`SurfaceService.Scenario` shock grid; `celnet-risk-cube` OLAP VaR/ES/FRTB-curvature; `-risk-normalize`, `-limits` RAG, `-entitlements` deny-wins) · XVA (`compute_xva`, no RPC) · RFQ multi-dealer (`MultiDealerEngine`, `celnet-rfq/panel.rs:225`, ranked panel + last-look) · Blotter/positions (`RiskService.ListPositions/ListRatesPositions`, `RfqDeskService.ListDeals`) · Streaming (`StreamService.StreamSession` bidi snapshot+delta, click-to-trade `TradableToken`, `celnet-fanout::BroadcastRing`).

**Proto**: `celnet.proto` (4485 lines, ~145 msgs), **8 services**: Pricing · Quote(RFQ) · Stream · Risk(7 RPC) · Surface(GetSmile/MarkSurface/Scenario) · FixAdmin · Auth(20 RPC) · RfqDesk(5 RPC) · Notification. `Instrument.product` oneof = **26 payoff arms**. **No XVA message.** `handle_unary` (`ws/mod.rs`) wires price/price_rates/request_quote/request_multi_dealer_quote/quote_accept/quote_reject + risk/surface/fix/auth/rfq-desk. **No `xva` arm.**

## B. Current GUI (`gui/`) — what exists

**IA**: single SPA, NO router/URL routes; nav = `WorkspaceId` enum switched by `app.setWorkspace` (URL only for saved-view recall). **3 domain tabs × disjoint rails = 15 workspaces**, ALL persistent-mounted (`Shell.tsx:63-79`, CSS/`inert` visibility, never unmount):
- **FX Options**: Ticket, Stream, Surface, Risk, Excel
- **Fixed Income**: Rates, Curve, RatesRisk, Quoting, Deals, RatesBook, Book
- **Administration**: Connections, Admin, Permissions

Nav capability-gated by HIDING (not disabling) per identity (`commands.ts:171-203`).

**Components** (155 nodes): `TicketWorkspace.tsx` (772 lines, cyclo 32, fan-out 34 — biggest; delegates to `PRODUCT_REGISTRY`+`StructureGallery`/`PayoffChart`/`NetStructureStrip` but still owns date-math/RFQ-mode/dealer-panel/preview) · `products/` registry (24 per-family spec files, `ProductSpec{toInstrument, applicableClasses}`) · `StructureGallery` (grouped/searchable/keyboard listbox) · `PayoffChart` (SVG terminal payoff; em-dash for path-dependent) · `NetStructureStrip` · `GreeksStrip` (**the ONE clean cross-asset component** — label-only rho fork by class, `:46-63`) · **Blotter ×4 silos**: `StreamWorkspace`/`BookWorkspace` (FX) + `DealsBlotterWorkspace`/`RatesBookWorkspace` (FI) · Surface viewer (`SurfaceWorkspace`+`viz/SurfaceMesh,SmileChart`+`CubeWorkspace`, FX/metal only) · RFQ `DealerPanel` (**LIVE**, wired `TicketWorkspace.tsx:38,848` — ranked LP rows, last-look, book-by-row) · FI `QuotingWorkspace` (separate single-dealer, NO shared component w/ DealerPanel) · `StreamWorkspace` (virtualized click-to-trade, FX only) · rates stack (`Rates/RatesRisk/Curve`, fully parallel, non-shared) · `RiskWorkspace` (spot×vol P&L grid) · Admin trio.

**State**: `AppContext.tsx` single flat 767-line global context. FX-first: `pairCtx`/`setPair` primary/always-live; non-FX is an **overlay** (`nonFxUnderlier`+`selectUnderlier`→`ActiveUnderlier`) CLEARED when an FX pair is re-picked (`:518`). Class-aware components read `underlier.assetClass`; most FI workspaces IGNORE `underlier` (gated at rail/domain level instead → two class mechanisms, no single truth).

## C. Design system (already mature — REUSE, don't rebuild)

`gui/design-tokens.json` (DTCG, 73 tokens) → `gui/src/design/tokens.css` (plain CSS custom props, no Tailwind/CSS-in-JS): 28 **OKLCH** colors (`--brand oklch(0.72 0.17 35)` = coral #ff7357; `--accent oklch(0.62 0.19 280)` = indigo #6b6bf5), spacing 1-8, density axis (row-height 38px), 7-step type scale 10-28px, radius, elevation (shadow-panel/-float/-hud, blur-float 20px/-hud 40px), motion (flash-decay 450ms, workspace-switch 200ms). Three orthogonal `data-*` axes on `<html>`: appearance dark/light, contrast normal/high, density comfortable/compact (localStorage). "Aurora wash" radial-gradient bg. **CSS Modules** per component (~40 pairs). Storybook for primitives.

**CelNet brand faithfully implemented**: coral/indigo/Anaheim (headings only, barred from numerics)/pinwheel (`CelnetMark.tsx:14`, `CelnetLockup`/`CelnetWordmark`). **"MacOS-inspired" = Apple-HIG materials/vibrancy/depth** (Liquid-Glass backdrop-filter), explicitly NOT literal traffic-light chrome (`docs/celnet-capabilities.html:1732`).

Primitives live flat in `gui/src/components/` mixed with big feature comps — no primitive/feature boundary (a redesign opportunity).

## D. Pain-point thesis (evidenced)

1. **Coverage gaps**: XVA absent end-to-end (no proto/arm/GUI) · cross-asset exotics = only 3/24 arms for equity/commodity/crypto (others dimmed, mirrors `price_cross_asset` carry-seam limit) · crypto strike-axis surface priced-but-unsurfaced · no rates streaming.
2. **Duplication**: blotter ×4 by asset silo · **two fully parallel FX vs FI stacks sharing ~0 components** (RatesRisk duplicates Risk shock-grid wholesale) · two client pricing engines (`data/pricing.ts` 2281L + `mockSource.ts` 2226L, offline demo, conformance-gated) · flat 40-file components dir, no primitive/feature split.
3. **Asset-class inconsistency**: FX-first state, non-FX is a secondary overlay; `GreeksStrip` is the clean exception, every other cross-asset surface is missing or a separate parallel impl; two class-gating mechanisms coexist.
4. **Verbosity / no cross-capability flow**: 15 persistent workspaces, no unified router/drill-down grammar; `TicketWorkspace` still 772L; `AppContext` 767L flat.

Stale docs flagged: `docs/GUI-EXPERIENCE-DESIGN.md` (GW0-7) is a prior FX-only 8-workspace plan, now stale vs 15-workspace/3-domain reality; `WORLD-CLASS-BACKLOG.md:329` strategy-legs claim contradicted by code. No decision-kind knowledge claims exist for GUI architecture yet (gap to close in Phase 5).

## E. Excel (`excel/`) — parity context
13 `CELNET.*` Office.js fns (INSTRUMENT/PRICE/GREEKS/RATES/RFQ[panel=TRUE ranked!]/SUBSCRIBE/SURFACE/MARKSURFACE/SERIES/MARK/RISK/POSITIONS/LIMITS/STATUS). Hand-rolled independent contract mirror (`excel/src/contract/*`), NOT shared code with GUI — duplication to collapse behind one transport seam.

## THE 11 WORKFLOWS the new GUI must cover end-to-end
1. Price a vanilla option — ALL classes (FX full today; others partial)
2. Structure a multi-leg strategy / exotic — FX/metal today (24 arms); target registry-driven, class-scoped
3. Multi-dealer RFQ — FX live (DealerPanel); rates separate single-dealer → UNIFY
4. Stream live two-way quotes to a counterparty — FX only → extend cross-asset
5. Build/calibrate vol surface (5 smiles) + cube — FX/metal; crypto strike-axis missing
6. Blotter/positions — all classes, 4 silos today → ONE class-parameterized blotter
7. Portfolio/firm risk + scenario P&L — FX Risk + rates RatesRisk (dup) → unify
8. Portfolio XVA (CVA/DVA/FVA) — ABSENT end-to-end (server built)
9. FI dealer quoting / deal booking / curve inspection — rates only, isolated
10. Administer users/roles/capabilities/FIX connectivity — cross-cutting
11. Excel-parity workflows — all classes via CELNET.*, hand-mirrored

## Redesign north-star (derived)
Replace 15 siloed persistent workspaces + FX-first overlay state with a **unified, asset-class-parametric information architecture**: one entry → universal instrument/underlier picker (peer asset classes) → price → structure → RFQ → risk/XVA → blotter, where each surface is ONE component parameterized by asset class (the `GreeksStrip` pattern generalized). Reuse the mature OKLCH/CelNet design system; add the primitive/feature boundary + a composable drill-down/router grammar. Beat Synoption via MacOS-HIG vibrancy + cross-capability flow. Surface the built-but-hidden capabilities (XVA needs the D-xva wire lane; crypto strike-axis surface).
